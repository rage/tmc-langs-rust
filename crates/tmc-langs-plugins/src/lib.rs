#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Abstracts over the various language plugins.

pub mod archive;
pub mod compression;
mod error;

pub use error::PluginError;
use std::{
    io::{Read, Seek},
    path::{Path, PathBuf},
};
use tmc_langs_csharp::CSharpPlugin;
use tmc_langs_framework::{Archive, LanguagePlugin, TmcError};
pub use tmc_langs_framework::{
    Compression, ExerciseDesc, ExercisePackagingConfiguration, Language,
    NothingIsStudentFilePolicy, RunResult, StudentFilePolicy, StyleValidationResult,
    StyleValidationStrategy,
};
pub use tmc_langs_java::{AntPlugin, MavenPlugin};
pub use tmc_langs_make::MakePlugin;
pub use tmc_langs_notests::NoTestsPlugin;
pub use tmc_langs_python3::Python3Plugin;
pub use tmc_langs_r::RPlugin;

/// Finds the correct language plug-in for the given exercise path and calls `LanguagePlugin::extract_project`,
/// If no language plugin matches, see `extract_project_overwrite`.
pub fn extract_project(
    compressed_project: impl std::io::Read + std::io::Seek,
    target_location: &Path,
    compression: Compression,
    clean: bool,
) -> Result<(), PluginError> {
    if let Ok(plugin) = PluginType::from_exercise(target_location) {
        plugin.extract_project(compressed_project, target_location, compression, clean)?;
    } else {
        log::debug!(
            "no matching language plugin found for {}, overwriting",
            target_location.display()
        );
        let archive = Archive::new(compressed_project, compression)?;
        archive.extract(target_location)?;
    }
    Ok(())
}

/// Compresses the directory at the given path, only including student files unless `naive` is set to true.
pub fn compress_project(
    path: &Path,
    compression: Compression,
    naive: bool,
) -> Result<Vec<u8>, PluginError> {
    let compressed = if naive {
        compression.compress(path)?
    } else {
        let policy = get_student_file_policy(path)?;
        compression::compress_student_files(policy.as_ref(), path, compression)?
    };

    Ok(compressed)
}

/// Enum containing variants for each language plugin.
#[impl_enum::with_methods(
    pub fn clean(&self, path: &Path) -> Result<(), TmcError>
    pub fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc, TmcError>
    pub fn run_tests(&self, path: &Path) -> Result<RunResult, TmcError>
    pub fn check_code_style(&self, path: &Path, locale: Language) -> Result<Option<StyleValidationResult>, TmcError>
)]
pub enum Plugin {
    CSharp(CSharpPlugin),
    Make(MakePlugin),
    Maven(MavenPlugin),
    NoTests(NoTestsPlugin),
    Python3(Python3Plugin),
    R(RPlugin),
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
            PluginType::Maven => Plugin::Maven(MavenPlugin::new()?),
            PluginType::Ant => Plugin::Ant(AntPlugin::new()?),
        };
        Ok(plugin)
    }
}

/// Allows calling LanguagePlugin functions without constructing the plugin.
#[derive(Clone, Copy)]
pub enum PluginType {
    CSharp,
    Make,
    Maven,
    NoTests,
    Python3,
    R,
    Ant,
}

macro_rules! delegate_plugin_type {
    ($self:ident, $($args:tt)*) => {
        match $self {
            Self::CSharp => CSharpPlugin::$($args)*,
            Self::Make => MakePlugin::$($args)*,
            Self::Maven => MavenPlugin::$($args)*,
            Self::NoTests => NoTestsPlugin::$($args)*,
            Self::Python3 => Python3Plugin::$($args)*,
            Self::R => RPlugin::$($args)*,
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
        } else if MavenPlugin::is_exercise_type_correct(path) {
            (MavenPlugin::PLUGIN_NAME, PluginType::Maven)
        } else if AntPlugin::is_exercise_type_correct(path) {
            // TODO: currently, ant needs to be last because any project with src and test are recognized as ant
            (AntPlugin::PLUGIN_NAME, PluginType::Ant)
        } else {
            return Err(PluginError::PluginNotFound(path.to_path_buf()));
        };
        log::info!("Detected project at {} as {}", path.display(), plugin_name);
        Ok(plugin_type)
    }

    pub fn get_exercise_packaging_configuration(
        self,
        exercise_path: &Path,
    ) -> Result<ExercisePackagingConfiguration, TmcError> {
        delegate_plugin_type!(self, get_exercise_packaging_configuration(exercise_path))
    }

    pub fn extract_project(
        self,
        compressed_project: impl std::io::Read + std::io::Seek,
        target_location: &Path,
        compression: Compression,
        clean: bool,
    ) -> Result<(), TmcError> {
        delegate_plugin_type!(
            self,
            extract_project(compressed_project, target_location, compression, clean)
        )
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
        PluginType::Maven => Box::new(<MavenPlugin as LanguagePlugin>::StudentFilePolicy::new(
            path,
        )?),
        PluginType::Ant => Box::new(<AntPlugin as LanguagePlugin>::StudentFilePolicy::new(path)?),
    };
    Ok(policy)
}
