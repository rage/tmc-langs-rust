//! Contains types that abstract over the various archive formats.

use crate::TmcError;
use serde::Deserialize;
use std::{
    fmt::Display,
    io::{BufReader, Cursor, Read, Seek},
    ops::ControlFlow::{self, Break},
    path::{Path, PathBuf},
    str::FromStr,
};
#[cfg(feature = "ts")]
use ts_rs::TS;

/// Wrapper unifying the API of all the different compression formats supported by langs.
/// Unfortunately the API is more complicated due to tar only supporting iterating through the files one by one,
/// while zip only supports accessing by index.
pub enum Archive<T: Read + Seek> {
    Tar(tar::Archive<T>),
    TarZstd(tar::Archive<zstd::Decoder<'static, BufReader<T>>>),
    Zip(zip::ZipArchive<T>),
}

impl<T: Read + Seek> Archive<T> {
    pub fn new(archive: T, compression: Compression) -> Result<Self, TmcError> {
        match compression {
            Compression::Tar => Ok(Self::tar(archive)),
            Compression::TarZstd => Self::tar_zstd(archive),
            Compression::Zip => Self::zip(archive),
        }
    }

    pub fn tar(archive: T) -> Self {
        let archive = tar::Archive::new(archive);
        Self::Tar(archive)
    }

    pub fn tar_zstd(archive: T) -> Result<Self, TmcError> {
        let archive = zstd::Decoder::new(archive).map_err(TmcError::ZstdRead)?;
        let archive = tar::Archive::new(archive);
        Ok(Self::TarZstd(archive))
    }

    pub fn zip(archive: T) -> Result<Self, TmcError> {
        let archive = zip::ZipArchive::new(archive)?;
        Ok(Self::Zip(archive))
    }

    /// a
    pub fn iter(&mut self) -> Result<ArchiveIterator<'_, T>, TmcError> {
        match self {
            Self::Tar(archive) => {
                let iter = ArchiveIterator::Tar(archive.entries().map_err(TmcError::TarRead)?);
                Ok(iter)
            }
            Self::TarZstd(archive) => {
                let iter = ArchiveIterator::TarZstd(archive.entries().map_err(TmcError::TarRead)?);
                Ok(iter)
            }
            Self::Zip(archive) => Ok(ArchiveIterator::Zip(0, archive)),
        }
    }

    pub fn by_path(&mut self, path: &str) -> Result<Entry<'_, T>, TmcError> {
        match self {
            Self::Tar(archive) => {
                for entry in archive.entries().map_err(TmcError::TarRead)? {
                    let mut entry = entry.map_err(TmcError::TarRead)?;
                    if entry.path().map_err(TmcError::TarRead)? == Path::new(path) {
                        return Ok(Entry::Tar(entry));
                    }
                    // "process" file
                    let mut buf = Vec::new();
                    entry.read_to_end(&mut buf).map_err(TmcError::TarRead)?;
                }
                Err(TmcError::TarRead(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Could not find {path} in tar"),
                )))
            }
            Self::TarZstd(archive) => {
                for entry in archive.entries().map_err(TmcError::TarRead)? {
                    let mut entry = entry.map_err(TmcError::TarRead)?;
                    if entry.path().map_err(TmcError::TarRead)? == Path::new(path) {
                        return Ok(Entry::TarZstd(entry));
                    }
                    // "process" file
                    let mut buf = Vec::new();
                    entry.read_to_end(&mut buf).map_err(TmcError::TarRead)?;
                }
                Err(TmcError::TarRead(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Could not find {path} in tar"),
                )))
            }
            Self::Zip(archive) => archive.by_name(path).map(Entry::Zip).map_err(Into::into),
        }
    }
}

pub enum ArchiveIterator<'a, T: Read + Seek> {
    Tar(tar::Entries<'a, T>),
    TarZstd(tar::Entries<'a, zstd::Decoder<'static, BufReader<T>>>),
    Zip(usize, &'a mut zip::ZipArchive<T>),
}

impl<'a, T: Read + Seek> ArchiveIterator<'a, T> {
    /// Returns Break(None) when there's nothing left to iterate.
    pub fn with_next<U, F: FnMut(Entry<'_, T>) -> Result<ControlFlow<Option<U>>, TmcError>>(
        &mut self,
        mut f: F,
    ) -> Result<ControlFlow<Option<U>>, TmcError> {
        match self {
            Self::Tar(iter) => {
                let next = iter
                    .next()
                    .map(|e| e.map(Entry::Tar))
                    .transpose()
                    .map_err(TmcError::TarRead)?;
                if let Some(next) = next {
                    let res = f(next)?;
                    Ok(res)
                } else {
                    Ok(Break(None))
                }
            }
            Self::TarZstd(iter) => {
                let next = iter
                    .next()
                    .map(|e| e.map(Entry::TarZstd))
                    .transpose()
                    .map_err(TmcError::TarRead)?;
                if let Some(next) = next {
                    let res = f(next)?;
                    Ok(res)
                } else {
                    Ok(Break(None))
                }
            }
            Self::Zip(i, archive) => {
                if *i < archive.len() {
                    let next = archive.by_index(*i)?;
                    *i += 1;
                    let res = f(Entry::Zip(next))?;
                    Ok(res)
                } else {
                    Ok(Break(None))
                }
            }
        }
    }
}

pub enum Entry<'a, T: Read> {
    Tar(tar::Entry<'a, T>),
    TarZstd(tar::Entry<'a, zstd::Decoder<'static, BufReader<T>>>),
    Zip(zip::read::ZipFile<'a>),
}

impl<'a, T: Read> Entry<'a, T> {
    pub fn path(&self) -> Result<PathBuf, TmcError> {
        match self {
            Self::Tar(entry) => {
                let name = entry.path().map_err(TmcError::TarRead)?.into_owned();
                Ok(name)
            }
            Self::TarZstd(entry) => {
                let name = entry.path().map_err(TmcError::TarRead)?.into_owned();
                Ok(name)
            }
            Self::Zip(entry) => {
                let name = entry
                    .enclosed_name()
                    .ok_or_else(|| TmcError::ZipName(entry.name().to_string()))?
                    .to_path_buf();
                Ok(name)
            }
        }
    }

    pub fn is_dir(&self) -> bool {
        match self {
            Self::Tar(entry) => matches!(entry.header().entry_type(), tar::EntryType::Directory),
            Self::TarZstd(entry) => {
                matches!(entry.header().entry_type(), tar::EntryType::Directory)
            }
            Self::Zip(entry) => entry.is_dir(),
        }
    }

    pub fn is_file(&self) -> bool {
        match self {
            Self::Tar(entry) => matches!(entry.header().entry_type(), tar::EntryType::Regular),
            Self::TarZstd(entry) => {
                matches!(entry.header().entry_type(), tar::EntryType::Regular)
            }
            Self::Zip(entry) => entry.is_file(),
        }
    }
}

impl<'a, T: Read> Read for Entry<'a, T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Tar(archive) => archive.read(buf),
            Self::TarZstd(archive) => archive.read(buf),
            Self::Zip(archive) => archive.read(buf),
        }
    }
}

/// Supported compression methods.
#[derive(Debug, Clone, Copy, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
pub enum Compression {
    /// .tar
    #[serde(rename = "tar")]
    Tar,
    /// .zip
    #[serde(rename = "zip")]
    Zip,
    /// .tar.ztd
    #[serde(rename = "zstd")]
    TarZstd,
}

impl Compression {
    pub fn compress(self, path: &Path) -> Result<Vec<u8>, TmcError> {
        let buf = Cursor::new(Vec::new());
        let buf = match self {
            Self::Tar => {
                let mut builder = tar::Builder::new(buf);
                builder
                    .append_dir_all(".", path)
                    .map_err(TmcError::TarWrite)?;
                builder.into_inner().map_err(TmcError::TarWrite)?
            }
            Self::Zip => {
                let mut writer = zip::ZipWriter::new(buf);
                let path_str = path
                    .to_str()
                    .ok_or_else(|| TmcError::InvalidUtf8(path.to_path_buf()))?;
                writer.add_directory(path_str, Default::default())?;
                writer.finish()?
            }
            Self::TarZstd => {
                let mut builder = tar::Builder::new(buf);
                builder
                    .append_dir_all(".", path)
                    .map_err(TmcError::TarWrite)?;
                let buf = builder.into_inner().map_err(TmcError::TarWrite)?;
                let encoder = zstd::Encoder::new(buf, 0).map_err(TmcError::ZstdWrite)?;
                encoder.finish().map_err(TmcError::ZstdWrite)?
            }
        };
        Ok(buf.into_inner())
    }
}

impl Display for Compression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tar => write!(f, "tar"),
            Self::Zip => write!(f, "zip"),
            Self::TarZstd => write!(f, "zstd"),
        }
    }
}

impl FromStr for Compression {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let format = match s {
            "tar" => Compression::Tar,
            "zip" => Compression::Zip,
            "zstd" => Compression::TarZstd,
            _ => return Err("invalid format"),
        };
        Ok(format)
    }
}
