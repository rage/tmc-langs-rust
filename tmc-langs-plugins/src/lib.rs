#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

pub mod archive;
pub mod compression;
mod error;

pub use error::PluginError;
use std::{
    io::{Read, Seek},
    path::{Path, PathBuf},
};
use tmc_langs_csharp::CSharpPlugin;
use tmc_langs_framework::{Archive, LanguagePlugin, TmcError, TmcProjectYml};
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
    if let Ok(plugin) = get_language_plugin(target_location) {
        plugin.extract_project(compressed_project, target_location, compression, clean)?;
    } else {
        log::debug!(
            "no matching language plugin found for {}, overwriting",
            target_location.display()
        );
        extract_project_overwrite(compressed_project, target_location)?;
    }
    Ok(())
}

/// Extract a given archive file containing a compressed project to a target location.
/// This will overwrite any existing files.
pub fn extract_project_overwrite(
    compressed_project: impl std::io::Read + std::io::Seek,
    target_location: &Path,
) -> Result<(), PluginError> {
    compression::unzip(compressed_project, target_location)?;
    Ok(())
}

/// See `LanguagePlugin::compress_project`.
// TODO: clean up
pub fn compress_project(path: &Path, compression: Compression) -> Result<Vec<u8>, PluginError> {
    match get_language_plugin_type(path) {
        Some(PluginType::CSharp) => Ok(compression::compress_student_files(
            <CSharpPlugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
            compression,
        )?),
        Some(PluginType::Make) => Ok(compression::compress_student_files(
            <MakePlugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
            compression,
        )?),
        Some(PluginType::Maven) => Ok(compression::compress_student_files(
            <MavenPlugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
            compression,
        )?),
        Some(PluginType::NoTests) => Ok(compression::compress_student_files(
            <NoTestsPlugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
            compression,
        )?),
        Some(PluginType::Python3) => Ok(compression::compress_student_files(
            <Python3Plugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
            compression,
        )?),
        Some(PluginType::R) => Ok(compression::compress_student_files(
            <RPlugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
            compression,
        )?),
        Some(PluginType::Ant) => Ok(compression::compress_student_files(
            <AntPlugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
            compression,
        )?),
        None => Err(PluginError::PluginNotFound(path.to_path_buf())),
    }
}

// enum containing all the plugins
#[impl_enum::with_methods(
    pub fn clean(&self, path: &Path) -> Result<(), TmcError> {}
    pub fn get_exercise_packaging_configuration(config: TmcProjectYml) -> Result<ExercisePackagingConfiguration, TmcError> {}
    pub fn extract_project(compressed_project: impl std::io::Read + std::io::Seek, target_location: &Path, compression: Compression, clean: bool) -> Result<(), TmcError> {}
    pub fn extract_student_files(compressed_project: impl std::io::Read + std::io::Seek, target_location: &Path) -> Result<(), TmcError> {}
    pub fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc, TmcError> {}
    pub fn run_tests(&self, path: &Path) -> Result<RunResult, TmcError> {}
    pub fn check_code_style(&self, path: &Path, locale: Language) -> Result<Option<StyleValidationResult>, TmcError> {}
    pub fn get_available_points(exercise_path: &Path) -> Result<Vec<String>, TmcError> {}
    pub fn find_project_dir_in_archive<R: Read + Seek>(archive: &mut Archive<R>) -> Result<PathBuf, TmcError> {}
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

pub enum PluginType {
    CSharp,
    Make,
    Maven,
    NoTests,
    Python3,
    R,
    Ant,
}

pub fn get_language_plugin_type(path: &Path) -> Option<PluginType> {
    let plugin_type = if NoTestsPlugin::is_exercise_type_correct(path) {
        log::info!(
            "Detected project at {} as {}",
            path.display(),
            NoTestsPlugin::PLUGIN_NAME
        );
        PluginType::NoTests
    } else if CSharpPlugin::is_exercise_type_correct(path) {
        log::info!(
            "Detected project at {} as {}",
            path.display(),
            CSharpPlugin::PLUGIN_NAME
        );
        PluginType::CSharp
    } else if MakePlugin::is_exercise_type_correct(path) {
        log::info!(
            "Detected project at {} as {}",
            path.display(),
            MakePlugin::PLUGIN_NAME
        );
        PluginType::Make
    } else if Python3Plugin::is_exercise_type_correct(path) {
        log::info!(
            "Detected project at {} as {}",
            path.display(),
            Python3Plugin::PLUGIN_NAME
        );
        PluginType::Python3
    } else if RPlugin::is_exercise_type_correct(path) {
        log::info!(
            "Detected project at {} as {}",
            path.display(),
            RPlugin::PLUGIN_NAME
        );
        PluginType::R
    } else if MavenPlugin::is_exercise_type_correct(path) {
        log::info!(
            "Detected project at {} as {}",
            path.display(),
            MavenPlugin::PLUGIN_NAME
        );
        PluginType::Maven
    } else if AntPlugin::is_exercise_type_correct(path) {
        // TODO: currently, ant needs to be last because any project with src and test are recognized as ant
        log::info!(
            "Detected project at {} as {}",
            path.display(),
            AntPlugin::PLUGIN_NAME
        );
        PluginType::Ant
    } else {
        return None;
    };
    Some(plugin_type)
}

// Get language plugin for the given path.
pub fn get_language_plugin(path: &Path) -> Result<Plugin, PluginError> {
    let plugin = match get_language_plugin_type(path) {
        Some(PluginType::NoTests) => Plugin::NoTests(NoTestsPlugin::new()),
        Some(PluginType::CSharp) => Plugin::CSharp(CSharpPlugin::new()),
        Some(PluginType::Make) => Plugin::Make(MakePlugin::new()),
        Some(PluginType::Python3) => Plugin::Python3(Python3Plugin::new()),
        Some(PluginType::R) => Plugin::R(RPlugin::new()),
        Some(PluginType::Maven) => Plugin::Maven(MavenPlugin::new()?),
        Some(PluginType::Ant) => Plugin::Ant(AntPlugin::new()?),
        None => return Err(PluginError::PluginNotFound(path.to_path_buf())),
    };
    Ok(plugin)
}
