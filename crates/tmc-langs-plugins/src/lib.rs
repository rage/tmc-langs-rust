#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Abstracts over the various language plugins.

pub mod compression;
mod error;

use blake3::Hash;
pub use error::PluginError;
use std::{
    io::{Read, Seek},
    path::{Path, PathBuf},
};
pub use tmc_langs_csharp::CSharpPlugin;
use tmc_langs_framework::{Archive, LanguagePlugin, TmcError};
pub use tmc_langs_framework::{
    Compression, ExerciseDesc, ExercisePackagingConfiguration, Language,
    NothingIsStudentFilePolicy, RunResult, StudentFilePolicy, StyleValidationResult,
    StyleValidationStrategy,
};
use tmc_langs_util::file_util;
// the Java plugin is disabled on musl
#[cfg(not(target_env = "musl"))]
pub use tmc_langs_java::{AntPlugin, MavenPlugin};
pub use tmc_langs_make::MakePlugin;
pub use tmc_langs_notests::NoTestsPlugin;
pub use tmc_langs_python3::Python3Plugin;
pub use tmc_langs_r::RPlugin;

/// Extracts the compressed project to the target location with the language plugin matching the archive,
/// see [`PluginType::extract_project`]. If no language plugin matches, extracts the whole archive as is.
pub fn extract_project(
    compressed_project: impl std::io::Read + std::io::Seek,
    target_location: &Path,
    compression: Compression,
    clean: bool,
) -> Result<(), PluginError> {
    let mut archive = Archive::new(compressed_project, compression)?;
    if let Ok(plugin) = PluginType::from_archive(&mut archive) {
        plugin.extract_project(&mut archive, target_location, clean)?;
    } else {
        log::debug!("no matching language plugin found",);
        archive.extract(target_location)?;
    }
    Ok(())
}

/// Compresses the directory at the given path, only including student files unless `naive` is set to true.
pub fn compress_project(
    path: &Path,
    compression: Compression,
    deterministic: bool,
    naive: bool,
    hash: bool,
    size_limit_mb: u32,
) -> Result<(Vec<u8>, Option<Hash>), PluginError> {
    let (compressed, hash) = if naive {
        compression.compress(path, hash)?
    } else {
        let policy = get_student_file_policy(path)?;
        compression::compress_student_files(
            policy.as_ref(),
            path,
            compression,
            deterministic,
            hash,
            size_limit_mb,
        )?
    };
    Ok((compressed, hash))
}

/// Enum containing variants for each language plugin.
pub enum Plugin {
    CSharp(CSharpPlugin),
    Make(MakePlugin),
    // the Java plugin is disabled on musl
    #[cfg(not(target_env = "musl"))]
    Maven(MavenPlugin),
    NoTests(NoTestsPlugin),
    Python3(Python3Plugin),
    R(RPlugin),
    // the Java plugin is disabled on musl
    #[cfg(not(target_env = "musl"))]
    Ant(AntPlugin),
}

impl Plugin {
    // Get language plugin for the given path.
    pub fn from_exercise(path: &Path) -> Result<Self, PluginError> {
        let plugin = match PluginType::from_exercise(path)? {
            PluginType::NoTests => Plugin::NoTests(NoTestsPlugin::new()),
            PluginType::CSharp => Plugin::CSharp(CSharpPlugin::new()),
            PluginType::Make => Plugin::Make(MakePlugin::new()),
            PluginType::Python3 => Plugin::Python3(Python3Plugin::new()),
            PluginType::R => Plugin::R(RPlugin::new()),
            // the Java plugin is disabled on musl
            #[cfg(not(target_env = "musl"))]
            PluginType::Maven => Plugin::Maven(MavenPlugin::new()?),
            // the Java plugin is disabled on musl
            #[cfg(not(target_env = "musl"))]
            PluginType::Ant => Plugin::Ant(AntPlugin::new()?),
        };
        Ok(plugin)
    }

    pub fn clean(&self, path: &Path) -> Result<(), TmcError> {
        match self {
            Plugin::CSharp(plugin) => plugin.clean(path),
            Plugin::Make(plugin) => plugin.clean(path),
            // the Java plugin is disabled on musl
            #[cfg(not(target_env = "musl"))]
            Plugin::Maven(plugin) => plugin.clean(path),
            Plugin::NoTests(plugin) => plugin.clean(path),
            Plugin::Python3(plugin) => plugin.clean(path),
            Plugin::R(plugin) => plugin.clean(path),
            // the Java plugin is disabled on musl
            #[cfg(not(target_env = "musl"))]
            Plugin::Ant(plugin) => plugin.clean(path),
        }
    }

    pub fn scan_exercise(
        &self,
        path: &Path,
        exercise_name: String,
    ) -> Result<ExerciseDesc, TmcError> {
        match self {
            Plugin::CSharp(plugin) => plugin.scan_exercise(path, exercise_name),
            Plugin::Make(plugin) => plugin.scan_exercise(path, exercise_name),
            // the Java plugin is disabled on musl
            #[cfg(not(target_env = "musl"))]
            Plugin::Maven(plugin) => plugin.scan_exercise(path, exercise_name),
            Plugin::NoTests(plugin) => plugin.scan_exercise(path, exercise_name),
            Plugin::Python3(plugin) => plugin.scan_exercise(path, exercise_name),
            Plugin::R(plugin) => plugin.scan_exercise(path, exercise_name),
            // the Java plugin is disabled on musl
            #[cfg(not(target_env = "musl"))]
            Plugin::Ant(plugin) => plugin.scan_exercise(path, exercise_name),
        }
    }

    pub fn run_tests(&self, path: &Path) -> Result<RunResult, TmcError> {
        match self {
            Plugin::CSharp(plugin) => plugin.run_tests(path),
            Plugin::Make(plugin) => plugin.run_tests(path),
            // the Java plugin is disabled on musl
            #[cfg(not(target_env = "musl"))]
            Plugin::Maven(plugin) => plugin.run_tests(path),
            Plugin::NoTests(plugin) => plugin.run_tests(path),
            Plugin::Python3(plugin) => plugin.run_tests(path),
            Plugin::R(plugin) => plugin.run_tests(path),
            // the Java plugin is disabled on musl
            #[cfg(not(target_env = "musl"))]
            Plugin::Ant(plugin) => plugin.run_tests(path),
        }
    }

    pub fn check_code_style(
        &self,
        path: &Path,
        locale: Language,
    ) -> Result<Option<StyleValidationResult>, TmcError> {
        match self {
            Plugin::CSharp(plugin) => plugin.check_code_style(path, locale),
            Plugin::Make(plugin) => plugin.check_code_style(path, locale),
            // the Java plugin is disabled on musl
            #[cfg(not(target_env = "musl"))]
            Plugin::Maven(plugin) => plugin.check_code_style(path, locale),
            Plugin::NoTests(plugin) => plugin.check_code_style(path, locale),
            Plugin::Python3(plugin) => plugin.check_code_style(path, locale),
            Plugin::R(plugin) => plugin.check_code_style(path, locale),
            // the Java plugin is disabled on musl
            #[cfg(not(target_env = "musl"))]
            Plugin::Ant(plugin) => plugin.check_code_style(path, locale),
        }
    }
}

/// Allows calling LanguagePlugin functions without constructing the plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginType {
    CSharp,
    Make,
    // the Java plugin is disabled on musl
    #[cfg(not(target_env = "musl"))]
    Maven,
    NoTests,
    Python3,
    R,
    // the Java plugin is disabled on musl
    #[cfg(not(target_env = "musl"))]
    Ant,
}

macro_rules! delegate_plugin_type {
    ($self:ident, $($args:tt)*) => {
        match $self {
            Self::CSharp => CSharpPlugin::$($args)*,
            Self::Make => MakePlugin::$($args)*,
            // the Java plugin is disabled on musl
            #[cfg(not(target_env = "musl"))]
            Self::Maven => MavenPlugin::$($args)*,
            Self::NoTests => NoTestsPlugin::$($args)*,
            Self::Python3 => Python3Plugin::$($args)*,
            Self::R => RPlugin::$($args)*,
            // the Java plugin is disabled on musl
            #[cfg(not(target_env = "musl"))]
            Self::Ant => AntPlugin::$($args)*,
        }
    };
}

impl PluginType {
    pub fn from_exercise(path: &Path) -> Result<Self, PluginError> {
        let (plugin_name, plugin_type) = if NoTestsPlugin::is_exercise_type_correct(path) {
            (NoTestsPlugin::PLUGIN_NAME, PluginType::NoTests)
        } else if CSharpPlugin::is_exercise_type_correct(path) {
            (CSharpPlugin::PLUGIN_NAME, PluginType::CSharp)
        } else if MakePlugin::is_exercise_type_correct(path) {
            (MakePlugin::PLUGIN_NAME, PluginType::Make)
        } else if Python3Plugin::is_exercise_type_correct(path) {
            (Python3Plugin::PLUGIN_NAME, PluginType::Python3)
        } else if RPlugin::is_exercise_type_correct(path) {
            (RPlugin::PLUGIN_NAME, PluginType::R)
        } else {
            // the Java plugin is disabled on musl
            #[cfg(not(target_env = "musl"))]
            if MavenPlugin::is_exercise_type_correct(path) {
                (MavenPlugin::PLUGIN_NAME, PluginType::Maven)
            } else if AntPlugin::is_exercise_type_correct(path) {
                // TODO: currently, ant needs to be last because any project with src and test are recognized as ant
                (AntPlugin::PLUGIN_NAME, PluginType::Ant)
            } else {
                return Err(PluginError::PluginNotFound(path.to_path_buf()));
            }
            #[cfg(target_env = "musl")]
            return Err(PluginError::PluginNotFound(path.to_path_buf()));
        };
        log::info!("Detected project at {} as {}", path.display(), plugin_name);
        Ok(plugin_type)
    }

    pub fn from_archive<R: Read + Seek>(archive: &mut Archive<R>) -> Result<Self, PluginError> {
        let (plugin_name, plugin_type) = if NoTestsPlugin::is_archive_type_correct(archive) {
            (NoTestsPlugin::PLUGIN_NAME, PluginType::NoTests)
        } else if CSharpPlugin::is_archive_type_correct(archive) {
            (CSharpPlugin::PLUGIN_NAME, PluginType::CSharp)
        } else if MakePlugin::is_archive_type_correct(archive) {
            (MakePlugin::PLUGIN_NAME, PluginType::Make)
        } else if Python3Plugin::is_archive_type_correct(archive) {
            (Python3Plugin::PLUGIN_NAME, PluginType::Python3)
        } else if RPlugin::is_archive_type_correct(archive) {
            (RPlugin::PLUGIN_NAME, PluginType::R)
        } else {
            // the Java plugin is disabled on musl
            #[cfg(not(target_env = "musl"))]
            if MavenPlugin::is_archive_type_correct(archive) {
                (MavenPlugin::PLUGIN_NAME, PluginType::Maven)
            } else if AntPlugin::is_archive_type_correct(archive) {
                // TODO: currently, ant needs to be last because any project with src and test are recognized as ant
                (AntPlugin::PLUGIN_NAME, PluginType::Ant)
            } else {
                return Err(PluginError::PluginNotFoundInArchive);
            }
            #[cfg(target_env = "musl")]
            return Err(PluginError::PluginNotFoundInArchive);
        };
        log::info!("Detected project in archive as {plugin_name}");
        Ok(plugin_type)
    }

    pub fn get_exercise_packaging_configuration(
        self,
        exercise_path: &Path,
    ) -> Result<ExercisePackagingConfiguration, TmcError> {
        delegate_plugin_type!(self, get_exercise_packaging_configuration(exercise_path))
    }

    /// Extracts `archive`, an exercise of this plugin's type, into `target_location`.
    ///
    /// If `target_location` holds an exercise of another type, the extract is always clean and ignores
    /// the old `.tmcproject.yml`, so student files are judged by this plugin's policy alone; the old
    /// exercise's files would otherwise win plugin detection over the new exercise.
    pub fn extract_project<R: Read + Seek>(
        self,
        archive: &mut Archive<R>,
        target_location: &Path,
        clean: bool,
    ) -> Result<(), TmcError> {
        let mut clean = clean;
        if let Ok(local_plugin) = Self::from_exercise(target_location)
            && local_plugin != self
        {
            log::info!(
                "exercise at {} changed from {local_plugin:?} to {self:?}, replacing it",
                target_location.display()
            );
            let tmc_project_yml = target_location.join(".tmcproject.yml");
            if tmc_project_yml.is_file() {
                file_util::remove_file(&tmc_project_yml)?;
            }
            clean = true;
        }
        delegate_plugin_type!(self, extract_project(archive, target_location, clean))
    }

    pub fn extract_student_files(
        self,
        compressed_project: impl std::io::Read + std::io::Seek,
        compression: Compression,
        target_location: &Path,
    ) -> Result<(), TmcError> {
        delegate_plugin_type!(
            self,
            extract_student_files(compressed_project, compression, target_location)
        )
    }

    pub fn find_project_dir_in_archive<R: Read + Seek>(
        self,
        archive: &mut Archive<R>,
    ) -> Result<PathBuf, TmcError> {
        delegate_plugin_type!(self, find_project_dir_in_archive(archive))
    }

    pub fn safe_find_project_dir_in_archive<R: Read + Seek>(
        self,
        archive: &mut Archive<R>,
    ) -> Result<PathBuf, TmcError> {
        Ok(delegate_plugin_type!(
            self,
            safe_find_project_dir_in_archive(archive)
        ))
    }

    pub fn get_available_points(self, exercise_path: &Path) -> Result<Vec<String>, TmcError> {
        delegate_plugin_type!(self, get_available_points(exercise_path))
    }
}

pub fn get_student_file_policy(path: &Path) -> Result<Box<dyn StudentFilePolicy>, PluginError> {
    let policy: Box<dyn StudentFilePolicy> = match PluginType::from_exercise(path)? {
        PluginType::NoTests => Box::new(<NoTestsPlugin as LanguagePlugin>::StudentFilePolicy::new(
            path,
        )?),
        PluginType::CSharp => Box::new(<CSharpPlugin as LanguagePlugin>::StudentFilePolicy::new(
            path,
        )?),
        PluginType::Make => Box::new(<MakePlugin as LanguagePlugin>::StudentFilePolicy::new(
            path,
        )?),
        PluginType::Python3 => Box::new(<Python3Plugin as LanguagePlugin>::StudentFilePolicy::new(
            path,
        )?),
        PluginType::R => Box::new(<RPlugin as LanguagePlugin>::StudentFilePolicy::new(path)?),
        // the Java plugin is disabled on musl
        #[cfg(not(target_env = "musl"))]
        PluginType::Maven => Box::new(<MavenPlugin as LanguagePlugin>::StudentFilePolicy::new(
            path,
        )?),
        // the Java plugin is disabled on musl
        #[cfg(not(target_env = "musl"))]
        PluginType::Ant => Box::new(<AntPlugin as LanguagePlugin>::StudentFilePolicy::new(path)?),
    };
    Ok(policy)
}

#[cfg(test)]
mod test {
    use super::*;
    use std::io::Write;

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new().with_level(LevelFilter::Debug).init();
    }

    fn file_to(target_dir: &Path, relative: &str, contents: &str) {
        let target = target_dir.join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, contents).unwrap();
    }

    fn zip(files: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
        for (path, contents) in files {
            writer
                .start_file(*path, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(contents.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn ant_exercise(target_dir: &Path) {
        file_to(target_dir, "build.xml", "<project/>");
        file_to(target_dir, "nbproject/project.properties", "");
        file_to(target_dir, "src/Main.java", "class Main {}");
        file_to(target_dir, "test/MainTest.java", "class MainTest {}");
        file_to(target_dir, "student-notes.txt", "notes");
        file_to(target_dir, ".tmcproject.yml", "extra_student_files: [src]");
    }

    // the Java plugin is disabled on musl
    #[cfg(not(target_env = "musl"))]
    #[test]
    fn extract_replaces_exercise_whose_language_changed() {
        init();
        let temp = tempfile::tempdir().unwrap();
        ant_exercise(temp.path());

        let archive = zip(&[
            ("ex/R/main.R", "f <- function() 1"),
            ("ex/tests/testthat/test-main.R", "test_that('f', {})"),
        ]);
        extract_project(
            std::io::Cursor::new(archive),
            temp.path(),
            Compression::Zip,
            false,
        )
        .unwrap();

        assert!(temp.path().join("R/main.R").is_file());
        assert!(temp.path().join("tests/testthat/test-main.R").is_file());
        for stale in [
            "build.xml",
            "nbproject",
            "src",
            "test",
            "student-notes.txt",
            ".tmcproject.yml",
        ] {
            assert!(!temp.path().join(stale).exists(), "{stale} was left behind");
        }
        assert_eq!(
            PluginType::from_exercise(temp.path()).unwrap(),
            PluginType::R
        );
    }

    // the Java plugin is disabled on musl
    #[cfg(not(target_env = "musl"))]
    #[test]
    fn extract_with_changed_language_keeps_files_the_new_policy_covers() {
        init();
        let temp = tempfile::tempdir().unwrap();
        ant_exercise(temp.path());

        let archive = zip(&[
            ("ex/R/main.R", "f <- function() 1"),
            ("ex/tests/testthat/test-main.R", "test_that('f', {})"),
            (
                "ex/.tmcproject.yml",
                "extra_student_files: [student-notes.txt]",
            ),
        ]);
        extract_project(
            std::io::Cursor::new(archive),
            temp.path(),
            Compression::Zip,
            false,
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(temp.path().join("student-notes.txt")).unwrap(),
            "notes"
        );
        assert!(!temp.path().join("src").exists());
    }

    // the Java plugin is disabled on musl
    #[cfg(not(target_env = "musl"))]
    #[test]
    fn extract_with_same_language_keeps_unlisted_files() {
        init();
        let temp = tempfile::tempdir().unwrap();
        ant_exercise(temp.path());

        let archive = zip(&[
            ("ex/build.xml", "<project name=\"new\"/>"),
            ("ex/src/Main.java", "class Main { int x; }"),
            ("ex/test/MainTest.java", "class MainTest {}"),
        ]);
        extract_project(
            std::io::Cursor::new(archive),
            temp.path(),
            Compression::Zip,
            false,
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(temp.path().join("build.xml")).unwrap(),
            "<project name=\"new\"/>"
        );
        assert_eq!(
            std::fs::read_to_string(temp.path().join("src/Main.java")).unwrap(),
            "class Main {}"
        );
        assert!(temp.path().join("student-notes.txt").is_file());
        assert!(temp.path().join("nbproject/project.properties").is_file());
    }
}
