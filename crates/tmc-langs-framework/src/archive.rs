//! Contains types that abstract over the various archive formats.

use crate::{
    TmcError,
    archive_safety::{EntryNameStyle, ExtractionBudget, ExtractionLimits, validate_entry_name},
};
use blake3::{Hash, Hasher};
use serde::Deserialize;
use std::{
    fmt::Display,
    io::{BufReader, Cursor, Read, Seek, Write},
    ops::ControlFlow::{self, Break},
    path::{Component, Path, PathBuf},
    str::FromStr,
};
use tar::Builder;
use tmc_langs_util::{FileError, file_util};
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

    /// Extracts every file and directory under [`ExtractionLimits::default`], failing on the first
    /// unsafe entry name or exceeded limit. Links and other special entries are not extracted.
    pub fn extract(self, target_directory: &Path) -> Result<(), TmcError> {
        self.extract_with_limits(target_directory, ExtractionLimits::default())
    }

    pub fn extract_with_limits(
        mut self,
        target_directory: &Path,
        limits: ExtractionLimits,
    ) -> Result<(), TmcError> {
        let mut budget = ExtractionBudget::new(limits);
        let mut iter = self.iter()?;
        loop {
            let next = iter.with_next::<(), _>(|mut entry| {
                let relative = entry.path()?;
                budget.admit_entry(&entry.raw_name(), entry.size())?;
                let target = target_directory.join(relative);
                if entry.is_dir() {
                    file_util::create_dir_all(target)?;
                } else if entry.is_file() {
                    file_util::remove_all(&target)?;
                    entry.write_to_file(&target, &mut budget)?;
                    #[cfg(unix)]
                    if entry.is_executable() {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
                            .map_err(|e| FileError::FileWrite(target.clone(), e))?;
                    }
                } else {
                    log::warn!("not extracting special archive entry {}", entry.raw_name());
                }
                Ok(ControlFlow::Continue(()))
            })?;
            if let Break(_) = next {
                return Ok(());
            }
        }
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

    /// Finds the entry at `path` by comparing path components, so a zip entry matches whether its
    /// name uses `/` or `\` separators. Returns `None` if the archive has no such entry.
    pub fn by_path(&mut self, path: &Path) -> Result<Option<Entry<'_, T>>, TmcError> {
        self.reset()?;
        match self {
            Self(ArchiveInner::Tar(archive)) => {
                for entry in archive.entries_with_seek().map_err(TmcError::TarRead)? {
                    let entry = Entry::Tar(entry.map_err(TmcError::TarRead)?);
                    if entry.path()? == path {
                        return Ok(Some(entry));
                    }
                }
                Ok(None)
            }
            Self(ArchiveInner::TarZstd(archive)) => {
                for entry in archive.entries().map_err(TmcError::TarRead)? {
                    let entry = Entry::TarZstd(entry.map_err(TmcError::TarRead)?);
                    if entry.path()? == path {
                        return Ok(Some(entry));
                    }
                }
                Ok(None)
            }
            Self(ArchiveInner::Zip(archive)) => {
                // exact lookup first; the fallback matches names written with `\` separators
                let name = entry_name(path)?;
                let index = archive.index_for_name(&name).or_else(|| {
                    archive
                        .file_names()
                        .position(|n| n.replace('\\', "/") == name)
                });
                match index {
                    Some(i) => Ok(Some(Entry::Zip(archive.by_index(i)?))),
                    None => Ok(None),
                }
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
    /// The validated relative path of the entry, see [`validate_entry_name`]. Empty only for a
    /// directory entry naming the archive root.
    pub fn path(&self) -> Result<PathBuf, TmcError> {
        let raw_name = self.raw_name();
        let path = match self {
            Self::Tar(_) | Self::TarZstd(_) => validate_entry_name(&raw_name, EntryNameStyle::Tar),
            Self::Zip(_) => validate_entry_name(&raw_name, EntryNameStyle::Zip),
        }?;
        // Joined to the target, an empty path is the target itself, which extraction would delete.
        if path.as_os_str().is_empty() && !self.is_dir() {
            return Err(TmcError::UnsafeArchiveEntry {
                entry: raw_name,
                reason: "names the archive root but is not a directory",
            });
        }
        Ok(path)
    }

    /// The entry name as stored in the archive, for error messages.
    pub fn raw_name(&self) -> String {
        match self {
            Self::Tar(entry) => String::from_utf8_lossy(&entry.path_bytes()).into_owned(),
            Self::TarZstd(entry) => String::from_utf8_lossy(&entry.path_bytes()).into_owned(),
            Self::Zip(entry) => entry.name().to_string(),
        }
    }

    /// The uncompressed size the header declares; not trustworthy, see [`Self::write_to_file`].
    pub fn size(&self) -> u64 {
        match self {
            Self::Tar(entry) => entry.size(),
            Self::TarZstd(entry) => entry.size(),
            Self::Zip(entry) => entry.size(),
        }
    }

    /// Writes the entry's contents to `target`, counting the bytes actually read against `budget`
    /// and failing as soon as a limit is exceeded.
    pub fn write_to_file(
        &mut self,
        target: &Path,
        budget: &mut ExtractionBudget,
    ) -> Result<(), TmcError> {
        let name = self.raw_name();
        let mut file = file_util::create_file(target)?;
        let mut buffer = [0u8; 64 * 1024];
        let mut entry_bytes = 0u64;
        loop {
            let read = self
                .read(&mut buffer)
                .map_err(|e| TmcError::ZipReadFile(PathBuf::from(&name), e))?;
            if read == 0 {
                return Ok(());
            }
            entry_bytes += read as u64;
            budget.charge_written(&name, entry_bytes, read as u64)?;
            file.write_all(&buffer[..read])
                .map_err(|e| FileError::FileWrite(target.to_path_buf(), e))?;
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

    /// Whether the entry's Unix mode has an execute bit, e.g. a `gradlew` script.
    pub fn is_executable(&self) -> bool {
        let mode = match self {
            Self::Tar(entry) => entry.header().mode().ok(),
            Self::TarZstd(entry) => entry.header().mode().ok(),
            Self::Zip(entry) => entry.unix_mode(),
        };
        mode.is_some_and(|mode| mode & 0o111 != 0)
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
    pub fn compress(self, path: &Path, hash: bool) -> Result<(Vec<u8>, Option<Hash>), TmcError> {
        let mut builder = ArchiveBuilder::new(Cursor::new(Vec::new()), self, None, true, hash);
        walk_dir_for_compression(path, |entry, relative_path| {
            if entry.file_type().is_dir() {
                builder.add_directory(entry.path(), relative_path)?;
            } else if entry.file_type().is_file() {
                builder.add_file(entry.path(), relative_path)?;
            }
            Ok(())
        })?;
        let (cursor, hash) = builder.finish()?;
        Ok((cursor.into_inner(), hash))
    }
}

fn walk_dir_for_compression(
    root: &Path,
    mut f: impl FnMut(&walkdir::DirEntry, &Path) -> Result<(), TmcError>,
) -> Result<(), TmcError> {
    let parent = root.parent().map(PathBuf::from).unwrap_or_default();
    for entry in WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        // filter windows lock files
        .filter_entry(|e| e.file_name() != file_util::LOCK_FILE_NAME)
    {
        let entry = entry?;
        let stripped = entry
            .path()
            .strip_prefix(&parent)
            .expect("entries are within parent");
        f(&entry, stripped)?;
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
    size_limit_b: Option<usize>,
    size_limit_mb: Option<u32>,
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

// the one place allowed to write entries, as every name passes through `entry_name`
#[allow(clippy::disallowed_methods)]
impl<W: Write + Seek> ArchiveBuilder<W> {
    pub fn new(
        writer: W,
        compression: Compression,
        size_limit_mb: Option<u32>,
        deterministic: bool,
        hash: bool,
    ) -> Self {
        let size_limit_b = size_limit_mb.map(|slmb| {
            usize::try_from(slmb)
                .unwrap_or(usize::MAX)
                .saturating_mul(1000 * 1000)
        });
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

    /// Does not include any files within the directory. Skips the archive root itself.
    pub fn add_directory(&mut self, source: &Path, path_in_archive: &Path) -> Result<(), TmcError> {
        let path_in_archive = &entry_name(path_in_archive)?;
        if path_in_archive.is_empty() {
            return Ok(());
        }
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

    pub fn add_file(&mut self, source: &Path, path_in_archive: &Path) -> Result<(), TmcError> {
        let path_in_archive = &entry_name(path_in_archive)?;
        log::trace!("writing file {} as {}", source.display(), path_in_archive);
        self.hash(path_in_archive.as_bytes());
        let bytes = file_util::read_file(source)?;
        self.size_total_b += bytes.len();
        if let (Some(size_limit_b), Some(size_limit_mb)) = (self.size_limit_b, self.size_limit_mb) {
            if self.size_total_b > size_limit_b {
                return Err(TmcError::ArchiveSizeLimitExceeded {
                    limit: size_limit_mb,
                });
            }
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

/// Converts a relative path into an archive entry name, which uses `/` separators on every platform.
/// Non-UTF-8 components are converted lossily. Errors on paths that are absolute or contain `..`.
pub fn entry_name(path: &Path) -> Result<String, TmcError> {
    let mut name = String::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                if !name.is_empty() {
                    name.push('/');
                }
                name.push_str(&part.to_string_lossy());
            }
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => {
                return Err(TmcError::InvalidArchiveEntryPath(path.to_path_buf()));
            }
        }
    }
    Ok(name)
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
    fn entry_name_uses_forward_slashes() {
        let path = Path::new("course").join("exercise").join("file");
        assert_eq!(entry_name(&path).unwrap(), "course/exercise/file");
        assert_eq!(entry_name(Path::new("./file")).unwrap(), "file");
        assert!(entry_name(Path::new("../file")).is_err());
    }

    #[test]
    fn compressed_entry_names_use_forward_slashes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("exercise");
        file_util::write_to_file(b"", root.join("src").join("file")).unwrap();

        for compression in [Compression::Tar, Compression::TarZstd, Compression::Zip] {
            let (bytes, _) = compression.compress(&root, false).unwrap();
            let mut archive = Archive::new(Cursor::new(bytes), compression).unwrap();
            let mut names = vec![];
            let mut iter = archive.iter().unwrap();
            while let ControlFlow::Continue(()) = iter
                .with_next::<(), _>(|entry| {
                    names.push(entry_name_in_archive(&entry));
                    Ok(ControlFlow::Continue(()))
                })
                .unwrap()
            {}
            assert!(
                names.contains(&"exercise/src/file".to_string()),
                "{compression}: {names:?}"
            );
        }
    }

    /// The name as stored in the archive, without any separator conversion.
    fn entry_name_in_archive<T: Read>(entry: &Entry<'_, T>) -> String {
        match entry {
            Entry::Tar(entry) => String::from_utf8(entry.path_bytes().to_vec()).unwrap(),
            Entry::TarZstd(entry) => String::from_utf8(entry.path_bytes().to_vec()).unwrap(),
            Entry::Zip(entry) => entry.name().to_string(),
        }
    }

    #[test]
    fn by_path_ignores_separator_style() {
        for name in ["exercise/src/file", "exercise\\src\\file"] {
            let mut zw = ZipWriter::new(Cursor::new(vec![]));
            zw.start_file(name, SimpleFileOptions::default()).unwrap();
            let zip = zw.finish().unwrap().into_inner();

            let mut archive = Archive::zip(Cursor::new(zip)).unwrap();
            let path = Path::new("exercise").join("src").join("file");
            assert!(archive.by_path(&path).unwrap().is_some(), "{name}");
            assert!(archive.by_path(Path::new("missing")).unwrap().is_none());
        }
    }

    #[test]
    fn archive_root_gets_no_entry() {
        for compression in [Compression::Tar, Compression::TarZstd, Compression::Zip] {
            let temp = tempfile::tempdir().unwrap();
            let mut builder =
                ArchiveBuilder::new(Cursor::new(Vec::new()), compression, None, true, false);
            builder.add_directory(temp.path(), Path::new(".")).unwrap();
            let (bytes, _) = builder.finish().unwrap();
            let mut archive = Archive::new(Cursor::new(bytes.into_inner()), compression).unwrap();
            let mut iter = archive.iter().unwrap();
            assert!(matches!(
                iter.with_next::<(), _>(|_| Ok(ControlFlow::Continue(()))),
                Ok(ControlFlow::Break(None))
            ));
        }
    }

    #[test]
    fn exceeding_file_limit_causes_error() {
        let mut builder = ArchiveBuilder::new(
            Cursor::new(Vec::new()),
            Compression::Tar,
            Some(1),
            true,
            true,
        );

        // write exactly 1MB, OK
        let mut temp = NamedTempFile::new().unwrap();
        temp.write_all("a".as_bytes().repeat(1000 * 1000).as_slice())
            .unwrap();
        builder
            .add_file(temp.path(), Path::new("file"))
            .expect("should not be over size limit");

        // write one byte more, error
        let mut temp = NamedTempFile::new().unwrap();
        temp.write_all("a".as_bytes()).unwrap();
        assert!(
            builder.add_file(temp.path(), Path::new("file")).is_err(),
            "should be over size limit"
        );
    }

    fn tar_with_entry(name: &[u8], contents: &[u8]) -> Vec<u8> {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        // set_path rejects unsafe names, so write the name field directly
        header.as_old_mut().name[..name.len()].copy_from_slice(name);
        header.set_cksum();
        let mut builder = Builder::new(Vec::new());
        builder.append(&header, contents).unwrap();
        builder.into_inner().unwrap()
    }

    fn zip_with_entry(name: &str, contents: &[u8]) -> Vec<u8> {
        let mut zw = ZipWriter::new(Cursor::new(vec![]));
        zw.start_file(name, SimpleFileOptions::default()).unwrap();
        zw.write_all(contents).unwrap();
        zw.finish().unwrap().into_inner()
    }

    fn archives_with_entry(name: &str, contents: &[u8]) -> Vec<(Compression, Vec<u8>)> {
        let tar = tar_with_entry(name.as_bytes(), contents);
        let zstd = zstd::encode_all(Cursor::new(tar.clone()), 0).unwrap();
        vec![
            (Compression::Tar, tar),
            (Compression::TarZstd, zstd),
            (Compression::Zip, zip_with_entry(name, contents)),
        ]
    }

    #[test]
    fn extraction_rejects_unsafe_entries() {
        let windows_only = ["C:\\x", "CON", "aux.txt", "x:stream", "name."];
        let everywhere = ["../x", "/abs", "a\\..\\..\\x"];
        let names = everywhere
            .into_iter()
            .chain(windows_only.into_iter().filter(|_| cfg!(windows)));
        for name in names {
            for (compression, bytes) in archives_with_entry(name, b"data") {
                let temp = tempfile::tempdir().unwrap();
                let target = temp.path().join("target");
                let result = Archive::new(Cursor::new(bytes), compression)
                    .unwrap()
                    .extract(&target);
                let err = result.expect_err(&format!("{compression} {name}"));
                assert!(
                    matches!(err, TmcError::UnsafeArchiveEntry { .. }),
                    "{compression} {name}: {err}"
                );
                assert!(err.to_string().contains(name), "{err}");
                assert!(!temp.path().join("x").exists());
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn extraction_keeps_names_only_windows_rejects() {
        for name in ["notes:v2.txt", "Icon\r"] {
            for (compression, bytes) in archives_with_entry(name, b"data") {
                let temp = tempfile::tempdir().unwrap();
                Archive::new(Cursor::new(bytes), compression)
                    .unwrap()
                    .extract(temp.path())
                    .unwrap_or_else(|e| panic!("{compression} {name}: {e}"));
                assert!(temp.path().join(name).exists(), "{compression} {name}");
            }
        }
    }

    #[test]
    fn extraction_rejects_a_file_entry_naming_the_root() {
        for (compression, bytes) in archives_with_entry(".", b"data") {
            let temp = tempfile::tempdir().unwrap();
            file_util::write_to_file(b"kept", temp.path().join("existing")).unwrap();
            let result = Archive::new(Cursor::new(bytes), compression)
                .unwrap()
                .extract(temp.path());
            assert!(
                matches!(result, Err(TmcError::UnsafeArchiveEntry { .. })),
                "{compression}: {result:?}"
            );
            assert!(temp.path().join("existing").exists(), "{compression}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn extraction_keeps_execute_bits() {
        use std::os::unix::fs::PermissionsExt;
        let mut header = tar::Header::new_gnu();
        header.set_size(2);
        header.set_mode(0o755);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_path("gradlew").unwrap();
        header.set_cksum();
        let mut builder = Builder::new(Vec::new());
        builder.append(&header, &b"sh"[..]).unwrap();
        let tar = builder.into_inner().unwrap();

        let temp = tempfile::tempdir().unwrap();
        Archive::tar(Cursor::new(tar)).extract(temp.path()).unwrap();
        let mode = std::fs::metadata(temp.path().join("gradlew"))
            .unwrap()
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "{mode:o}");
    }

    #[test]
    fn extraction_writes_safe_entries() {
        for (compression, bytes) in archives_with_entry("dir/Makefile", b"all:") {
            let temp = tempfile::tempdir().unwrap();
            Archive::new(Cursor::new(bytes), compression)
                .unwrap()
                .extract(temp.path())
                .unwrap();
            assert_eq!(
                std::fs::read(temp.path().join("dir/Makefile")).unwrap(),
                b"all:"
            );
        }
    }

    #[test]
    fn extraction_enforces_limits() {
        let limits = ExtractionLimits {
            max_total_bytes: 100,
            max_entry_bytes: 50,
            max_entries: 3,
        };
        let cases = [(vec![0u8; 51], "bytes per entry"), (vec![0u8; 50], "ok")];
        for (contents, expected) in cases {
            for (compression, bytes) in archives_with_entry("f", &contents) {
                let temp = tempfile::tempdir().unwrap();
                let result = Archive::new(Cursor::new(bytes), compression)
                    .unwrap()
                    .extract_with_limits(temp.path(), limits);
                match (expected, result) {
                    ("ok", Ok(())) => {}
                    (
                        "bytes per entry",
                        Err(TmcError::ArchiveExtractionLimitExceeded { limit, .. }),
                    ) => assert_eq!(limit, "bytes per entry"),
                    (_, other) => panic!("{compression} {expected}: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn extraction_limits_total_bytes_and_entry_count() {
        let mut builder = Builder::new(Vec::new());
        for i in 0..4 {
            let mut header = tar::Header::new_gnu();
            header.set_size(40);
            header.set_mode(0o644);
            builder
                .append_data(&mut header, format!("f{i}"), &[1u8; 40][..])
                .unwrap();
        }
        let tar = builder.into_inner().unwrap();
        let limits = |max_total_bytes, max_entries| ExtractionLimits {
            max_total_bytes,
            max_entry_bytes: 40,
            max_entries,
        };
        let extract = |limits| {
            let temp = tempfile::tempdir().unwrap();
            Archive::tar(Cursor::new(tar.clone())).extract_with_limits(temp.path(), limits)
        };
        extract(limits(160, 4)).unwrap();
        assert!(matches!(
            extract(limits(159, 4)),
            Err(TmcError::ArchiveExtractionLimitExceeded {
                limit: "bytes in total",
                ..
            })
        ));
        assert!(matches!(
            extract(limits(160, 3)),
            Err(TmcError::ArchiveExtractionLimitExceeded {
                limit: "entries",
                ..
            })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn compress_does_not_follow_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside.txt");
        file_util::write_to_file(b"secret", &outside).unwrap();
        let root = temp.path().join("exercise");
        file_util::write_to_file(b"x", root.join("a.txt")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link.txt")).unwrap();
        for compression in [Compression::Tar, Compression::TarZstd, Compression::Zip] {
            let (bytes, _) = compression.compress(&root, false).unwrap();
            let unpacked = tempfile::tempdir().unwrap();
            Archive::new(Cursor::new(bytes), compression)
                .unwrap()
                .extract(unpacked.path())
                .unwrap();
            assert!(unpacked.path().join("exercise/a.txt").is_file());
            assert!(!unpacked.path().join("exercise/link.txt").exists());
        }
    }
}
