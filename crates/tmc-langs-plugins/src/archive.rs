//! Contains types that abstract over the various archive formats.

use std::{
    io::{Cursor, Seek, Write},
    path::Path,
};
use tar::Builder;
use tmc_langs_framework::{Compression, TmcError};
use tmc_langs_util::file_util;
use zip::{write::SimpleFileOptions, DateTime, ZipWriter};

pub enum ArchiveBuilder<W: Write + Seek> {
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
    pub fn new(writer: W, compression: Compression, deterministic: bool) -> Self {
        match compression {
            Compression::Tar => {
                let mut builder = Builder::new(writer);
                if deterministic {
                    builder.mode(tar::HeaderMode::Deterministic);
                }
                Self::Tar { builder }
            }
            Compression::TarZstd => {
                let mut builder = Builder::new(Cursor::new(vec![]));
                if deterministic {
                    builder.mode(tar::HeaderMode::Deterministic);
                }
                Self::TarZstd { writer, builder }
            }
            Compression::Zip => Self::Zip {
                builder: Box::new(ZipWriter::new(writer)),
                deterministic,
            },
        }
    }

    /// Does not include any files within the directory.
    pub fn add_directory(&mut self, source: &Path, path_in_archive: &str) -> Result<(), TmcError> {
        log::trace!("adding directory {}", path_in_archive);
        match self {
            Self::Tar { builder } => {
                builder
                    .append_dir(path_in_archive, source)
                    .map_err(TmcError::TarWrite)?;
            }
            Self::TarZstd { builder, .. } => {
                builder
                    .append_dir(path_in_archive, source)
                    .map_err(TmcError::TarWrite)?;
            }
            Self::Zip {
                builder,
                deterministic,
            } => builder.add_directory(path_in_archive, zip_file_options(*deterministic))?,
        }
        Ok(())
    }

    pub fn add_file(&mut self, source: &Path, path_in_archive: &str) -> Result<(), TmcError> {
        log::trace!("writing file {} as {}", source.display(), path_in_archive);
        match self {
            Self::Tar { builder } => builder
                .append_path_with_name(source, path_in_archive)
                .map_err(TmcError::TarWrite)?,
            Self::TarZstd { builder, .. } => builder
                .append_path_with_name(source, path_in_archive)
                .map_err(TmcError::TarWrite)?,
            Self::Zip {
                builder,
                deterministic,
            } => {
                let bytes = file_util::read_file(source)?;
                builder.start_file(path_in_archive, zip_file_options(*deterministic))?;
                builder
                    .write_all(&bytes)
                    .map_err(|e| TmcError::ZipWrite(source.into(), e))?;
            }
        }
        Ok(())
    }

    pub fn finish(self) -> Result<W, TmcError> {
        let res = match self {
            Self::Tar { builder } => builder.into_inner().map_err(TmcError::TarWrite)?,
            Self::TarZstd {
                mut writer,
                builder,
            } => {
                let tar_data = builder.into_inner().map_err(TmcError::TarWrite)?;
                zstd::stream::copy_encode(tar_data.get_ref().as_slice(), &mut writer, 0)
                    .map_err(TmcError::ZstdWrite)?;
                writer
            }
            Self::Zip { builder, .. } => builder.finish()?,
        };
        Ok(res)
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
