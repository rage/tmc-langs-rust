//! Contains types that abstract over the various archive formats.

use crate::TmcError;
use serde::Deserialize;
use std::{
    fmt::Display,
    io::{BufReader, Cursor, Read, Seek, Write},
    ops::ControlFlow::{self, Break},
    path::{Path, PathBuf},
    str::FromStr,
};
use tmc_langs_util::file_util;
use walkdir::WalkDir;

/// Wrapper unifying the API of all the different compression formats supported by langs.
/// Unfortunately the API is more complicated due to tar only supporting iterating through the files one by one,
/// while zip only supports accessing by index.
pub struct Archive<T: Read + Seek>(ArchiveInner<T>);

enum ArchiveInner<T: Read + Seek> {
    Tar(tar::Archive<T>),
    TarZstd(tar::Archive<zstd::Decoder<'static, BufReader<T>>>),
    Zip(zip::ZipArchive<T>),
    // This variant is only used for dummy values when swapping out the inner archive when we only have a &mut Archive
    Empty,
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
        Self(ArchiveInner::Tar(archive))
    }

    pub fn tar_zstd(archive: T) -> Result<Self, TmcError> {
        let archive = zstd::Decoder::new(archive).map_err(TmcError::ZstdRead)?;
        let archive = tar::Archive::new(archive);
        Ok(Self(ArchiveInner::TarZstd(archive)))
    }

    pub fn zip(archive: T) -> Result<Self, TmcError> {
        let archive = zip::ZipArchive::new(archive)?;
        Ok(Self(ArchiveInner::Zip(archive)))
    }

    pub fn extract(self, target_directory: &Path) -> Result<(), TmcError> {
        match self {
            Self(ArchiveInner::Tar(mut tar)) => {
                tar.unpack(target_directory).map_err(TmcError::TarRead)?
            }
            Self(ArchiveInner::TarZstd(mut zstd)) => {
                zstd.unpack(target_directory).map_err(TmcError::TarRead)?
            }
            Self(ArchiveInner::Zip(mut zip)) => zip.extract(target_directory)?,
            Self(ArchiveInner::Empty) => unreachable!("This is a bug."),
        }
        Ok(())
    }

    pub fn iter(&mut self) -> Result<ArchiveIterator<'_, T>, TmcError> {
        self.reset()?;
        match self {
            Self(ArchiveInner::Tar(archive)) => {
                let iter =
                    ArchiveIterator::Tar(archive.entries_with_seek().map_err(TmcError::TarRead)?);
                Ok(iter)
            }
            Self(ArchiveInner::TarZstd(archive)) => {
                let iter = ArchiveIterator::TarZstd(archive.entries().map_err(TmcError::TarRead)?);
                Ok(iter)
            }
            Self(ArchiveInner::Zip(archive)) => Ok(ArchiveIterator::Zip(0, archive)),
            Self(ArchiveInner::Empty) => unreachable!("This is a bug."),
        }
    }

    pub fn by_path(&mut self, path: &str) -> Result<Entry<'_, T>, TmcError> {
        self.reset()?;
        match self {
            Self(ArchiveInner::Tar(archive)) => {
                for entry in archive.entries_with_seek().map_err(TmcError::TarRead)? {
                    let entry = entry.map_err(TmcError::TarRead)?;
                    if entry.path().map_err(TmcError::TarRead)? == Path::new(path) {
                        return Ok(Entry::Tar(entry));
                    }
                }
                Err(TmcError::TarRead(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Could not find {path} in tar"),
                )))
            }
            Self(ArchiveInner::TarZstd(archive)) => {
                for entry in archive.entries().map_err(TmcError::TarRead)? {
                    let entry = entry.map_err(TmcError::TarRead)?;
                    if entry.path().map_err(TmcError::TarRead)? == Path::new(path) {
                        return Ok(Entry::TarZstd(entry));
                    }
                }
                Err(TmcError::TarRead(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Could not find {path} in tar"),
                )))
            }
            Self(ArchiveInner::Zip(archive)) => {
                archive.by_name(path).map(Entry::Zip).map_err(Into::into)
            }
            Self(ArchiveInner::Empty) => unreachable!("This is a bug."),
        }
    }

    pub fn compression(&self) -> Compression {
        match self {
            Self(ArchiveInner::Tar(_)) => Compression::Tar,
            Self(ArchiveInner::TarZstd(_)) => Compression::TarZstd,
            Self(ArchiveInner::Zip(_)) => Compression::Zip,
            Self(ArchiveInner::Empty) => unreachable!("This is a bug."),
        }
    }

    pub fn into_inner(self) -> T {
        match self {
            Self(ArchiveInner::Tar(archive)) => archive.into_inner(),
            Self(ArchiveInner::TarZstd(archive)) => archive.into_inner().finish().into_inner(),
            Self(ArchiveInner::Zip(archive)) => archive.into_inner(),
            Self(ArchiveInner::Empty) => unreachable!("This is a bug."),
        }
    }

    /// tar's entries functions require the archive's position to be at 0,
    /// but resetting the position is awkward, hence this helper function
    fn reset(&mut self) -> Result<(), TmcError> {
        let mut swap = ArchiveInner::Empty;
        std::mem::swap(&mut self.0, &mut swap);
        let mut swap = match swap {
            ArchiveInner::Tar(archive) => {
                let mut inner = archive.into_inner();
                inner
                    .seek(std::io::SeekFrom::Start(0))
                    .map_err(TmcError::Seek)?;
                ArchiveInner::Tar(tar::Archive::new(inner))
            }
            ArchiveInner::TarZstd(archive) => {
                let mut inner = archive.into_inner().finish().into_inner();
                inner
                    .seek(std::io::SeekFrom::Start(0))
                    .map_err(TmcError::Seek)?;
                let decoder = zstd::Decoder::new(inner).map_err(TmcError::ZstdRead)?;
                ArchiveInner::TarZstd(tar::Archive::new(decoder))
            }
            ArchiveInner::Zip(_) => {
                // no-op
                swap
            }
            ArchiveInner::Empty => unreachable!("This is a bug."),
        };
        // swap the value back in
        std::mem::swap(&mut self.0, &mut swap);
        Ok(())
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
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
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
        let buf = match self {
            Self::Tar => {
                let buf = Cursor::new(Vec::new());
                let mut builder = tar::Builder::new(buf);
                builder
                    .append_dir_all(".", path)
                    .map_err(TmcError::TarWrite)?;
                builder
                    .into_inner()
                    .map_err(TmcError::TarWrite)?
                    .into_inner()
            }
            Self::Zip => {
                let buf = Cursor::new(Vec::new());
                let mut writer = zip::ZipWriter::new(buf);
                let parent = path.parent().map(PathBuf::from).unwrap_or_default();
                for entry in WalkDir::new(path) {
                    let entry = entry?;
                    let stripped = entry
                        .path()
                        .strip_prefix(&parent)
                        .expect("entries are within parent");
                    let path_str = stripped
                        .to_str()
                        .ok_or_else(|| TmcError::InvalidUtf8(path.to_path_buf()))?;
                    if entry.path().is_dir() {
                        writer.add_directory(path_str, Default::default())?;
                    } else if entry.path().is_file() {
                        writer.start_file(path_str, Default::default())?;
                        let contents = file_util::read_file(entry.path())?;
                        writer
                            .write_all(&contents)
                            .map_err(|e| TmcError::ZipWrite(path.to_path_buf(), e))?;
                    }
                }
                writer.finish()?.into_inner()
            }
            Self::TarZstd => {
                let tar_buf = vec![];
                let mut builder = tar::Builder::new(tar_buf);
                builder
                    .append_dir_all(".", path)
                    .map_err(TmcError::TarWrite)?;
                let tar_buf = builder.into_inner().map_err(TmcError::TarWrite)?;
                zstd::stream::encode_all(tar_buf.as_slice(), 0).map_err(TmcError::ZstdWrite)?
            }
        };
        Ok(buf)
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
