//! Contains types that abstract over the various archive formats.

use crate::TmcError;
use blake3::{Hash, Hasher};
use serde::Deserialize;
use std::{
    fmt::Display,
    io::{BufReader, Cursor, Read, Seek, Write},
    ops::ControlFlow::{self, Break},
    path::{Path, PathBuf},
    str::FromStr,
    usize,
};
use tar::Builder;
use tmc_langs_util::file_util;
use walkdir::WalkDir;
use zip::{DateTime, ZipWriter, write::SimpleFileOptions};

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
                Err(TmcError::TarRead(std::io::Error::other(format!(
                    "Could not find {path} in tar"
                ))))
            }
            Self(ArchiveInner::TarZstd(archive)) => {
                for entry in archive.entries().map_err(TmcError::TarRead)? {
                    let entry = entry.map_err(TmcError::TarRead)?;
                    if entry.path().map_err(TmcError::TarRead)? == Path::new(path) {
                        return Ok(Entry::TarZstd(entry));
                    }
                }
                Err(TmcError::TarRead(std::io::Error::other(format!(
                    "Could not find {path} in tar"
                ))))
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

impl<T: Read + Seek> ArchiveIterator<'_, T> {
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
    Zip(zip::read::ZipFile<'a, T>),
}

impl<T: Read> Entry<'_, T> {
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

impl<T: Read> Read for Entry<'_, T> {
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
    pub fn compress(
        self,
        path: &Path,
        hash: bool,
        size_limit_mb: u32,
    ) -> Result<(Vec<u8>, Option<Hash>), TmcError> {
        let mut builder =
            ArchiveBuilder::new(Cursor::new(Vec::new()), self, size_limit_mb, true, hash);
        walk_dir_for_compression(path, size_limit_mb, |entry, relative_path| {
            if entry.path().is_dir() {
                builder.add_directory(entry.path(), relative_path)?;
            } else if entry.path().is_file() {
                builder.add_file(entry.path(), relative_path)?;
            }
            Ok(())
        })?;
        let (cursor, hash) = builder.finish()?;
        if u32::try_from(cursor.get_ref().len()).unwrap_or(u32::MAX) > size_limit_mb {
            return Err(TmcError::ArchiveSizeLimitExceeded {
                limit: size_limit_mb,
            });
        }
        Ok((cursor.into_inner(), hash))
    }
}

fn walk_dir_for_compression(
    root: &Path,
    size_limit_mb: u32,
    mut f: impl FnMut(&walkdir::DirEntry, &str) -> Result<(), TmcError>,
) -> Result<(), TmcError> {
    let size_limit_b = u64::from(size_limit_mb).saturating_mul(1000 * 1000);
    let mut size_total_b = 0;

    let parent = root.parent().map(PathBuf::from).unwrap_or_default();
    for entry in WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        // filter windows lock files
        .filter_entry(|e| e.file_name() != file_util::LOCK_FILE_NAME)
    {
        let entry = entry?;
        let metadata = entry.metadata()?;
        size_total_b += metadata.len();
        if size_total_b > size_limit_b {
            return Err(TmcError::ArchiveSizeLimitExceeded {
                limit: size_limit_mb,
            });
        }
        let stripped = entry
            .path()
            .strip_prefix(&parent)
            .expect("entries are within parent");
        let path_str = stripped
            .to_str()
            .ok_or_else(|| TmcError::InvalidUtf8(stripped.to_path_buf()))?;
        f(&entry, path_str)?;
    }
    Ok(())
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

pub struct ArchiveBuilder<W: Write + Seek> {
    size_limit_b: usize,
    size_limit_mb: u32,
    size_total_b: usize,
    hasher: Option<Hasher>,
    kind: Kind<W>,
}

enum Kind<W: Write + Seek> {
    Tar {
        builder: Builder<W>,
    },
    TarZstd {
        writer: W,
        builder: Builder<Cursor<Vec<u8>>>,
    },
    Zip {
        builder: Box<ZipWriter<W>>,
        deterministic: bool,
    },
}

impl<W: Write + Seek> ArchiveBuilder<W> {
    pub fn new(
        writer: W,
        compression: Compression,
        size_limit_mb: u32,
        deterministic: bool,
        hash: bool,
    ) -> Self {
        let size_limit_b = usize::try_from(size_limit_mb)
            .unwrap_or(usize::MAX)
            .saturating_mul(1000 * 1000);
        let hasher = if hash { Some(Hasher::new()) } else { None };
        let kind = match compression {
            Compression::Tar => {
                let mut builder = Builder::new(writer);
                if deterministic {
                    builder.mode(tar::HeaderMode::Deterministic);
                }
                Kind::Tar { builder }
            }
            Compression::TarZstd => {
                let mut builder = Builder::new(Cursor::new(vec![]));
                if deterministic {
                    builder.mode(tar::HeaderMode::Deterministic);
                }
                Kind::TarZstd { writer, builder }
            }
            Compression::Zip => Kind::Zip {
                builder: Box::new(ZipWriter::new(writer)),
                deterministic,
            },
        };
        Self {
            size_limit_b,
            size_limit_mb,
            size_total_b: 0,
            hasher,
            kind,
        }
    }

    /// Does not include any files within the directory.
    pub fn add_directory(&mut self, source: &Path, path_in_archive: &str) -> Result<(), TmcError> {
        log::trace!("adding directory {path_in_archive}");
        self.hash(path_in_archive.as_bytes());
        match &mut self.kind {
            Kind::Tar { builder } => {
                builder
                    .append_dir(path_in_archive, source)
                    .map_err(TmcError::TarWrite)?;
            }
            Kind::TarZstd { builder, .. } => {
                builder
                    .append_dir(path_in_archive, source)
                    .map_err(TmcError::TarWrite)?;
            }
            Kind::Zip {
                builder,
                deterministic,
            } => builder.add_directory(path_in_archive, zip_file_options(*deterministic))?,
        }
        Ok(())
    }

    pub fn add_file(&mut self, source: &Path, path_in_archive: &str) -> Result<(), TmcError> {
        log::trace!("writing file {} as {}", source.display(), path_in_archive);
        self.hash(path_in_archive.as_bytes());
        let bytes = file_util::read_file(source)?;
        self.size_total_b += bytes.len();
        if self.size_total_b > self.size_limit_b {
            return Err(TmcError::ArchiveSizeLimitExceeded {
                limit: self.size_limit_mb,
            });
        }
        self.hash(&bytes);
        match &mut self.kind {
            Kind::Tar { builder } => builder
                .append_path_with_name(source, path_in_archive)
                .map_err(TmcError::TarWrite)?,
            Kind::TarZstd { builder, .. } => builder
                .append_path_with_name(source, path_in_archive)
                .map_err(TmcError::TarWrite)?,
            Kind::Zip {
                builder,
                deterministic,
            } => {
                builder.start_file(path_in_archive, zip_file_options(*deterministic))?;
                builder
                    .write_all(&bytes)
                    .map_err(|e| TmcError::ZipWrite(source.into(), e))?;
            }
        }
        Ok(())
    }

    pub fn finish(self) -> Result<(W, Option<Hash>), TmcError> {
        let res = match self.kind {
            Kind::Tar { builder } => builder.into_inner().map_err(TmcError::TarWrite)?,
            Kind::TarZstd {
                mut writer,
                builder,
            } => {
                let tar_data = builder.into_inner().map_err(TmcError::TarWrite)?;
                zstd::stream::copy_encode(tar_data.get_ref().as_slice(), &mut writer, 0)
                    .map_err(TmcError::ZstdWrite)?;
                writer
            }
            Kind::Zip { builder, .. } => builder.finish()?,
        };
        let hash = self.hasher.map(|h| h.finalize());
        Ok((res, hash))
    }

    fn hash(&mut self, input: &[u8]) {
        self.hasher.as_mut().map(|h| h.update(input));
    }
}

fn zip_file_options(deterministic: bool) -> SimpleFileOptions {
    let file_options = SimpleFileOptions::default().unix_permissions(0o755);
    if deterministic {
        file_options.last_modified_time(
            DateTime::from_date_and_time(2023, 1, 1, 0, 0, 0).expect("known to work"),
        )
    } else {
        file_options
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn exceeding_file_limit_causes_error() {
        let mut builder =
            ArchiveBuilder::new(Cursor::new(Vec::new()), Compression::Tar, 1, true, true);

        // write exactly 1MB, OK
        let mut temp = NamedTempFile::new().unwrap();
        temp.write_all("a".as_bytes().repeat(1000 * 1000).as_slice())
            .unwrap();
        builder
            .add_file(temp.path(), "file")
            .expect("should not be over size limit");

        // write one byte more, error
        let mut temp = NamedTempFile::new().unwrap();
        temp.write_all("a".as_bytes()).unwrap();
        assert!(
            builder.add_file(temp.path(), "file").is_err(),
            "should be over size limit"
        );
    }
}
