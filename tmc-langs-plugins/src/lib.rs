mod error;
pub mod tmc_zip;

pub use error::PluginError;
pub use tmc_langs_framework::domain::{
    ExerciseDesc, ExercisePackagingConfiguration, RunResult, StyleValidationResult,
    StyleValidationStrategy,
};
pub use tmc_langs_framework::plugin::Language;
pub use tmc_langs_framework::policy::{NothingIsStudentFilePolicy, StudentFilePolicy};

use std::path::Path;
use tmc_langs_csharp::CSharpPlugin;
use tmc_langs_framework::{LanguagePlugin, TmcError, TmcProjectYml};
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
    clean: bool,
) -> Result<(), PluginError> {
    if let Ok(plugin) = get_language_plugin(target_location) {
        plugin.extract_project(compressed_project, target_location, clean)?;
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
    tmc_zip::unzip(
        NothingIsStudentFilePolicy::new(target_location)?,
        compressed_project,
        target_location,
    )?;
    Ok(())
}

/// See `LanguagePlugin::compress_project`.
// TODO: clean up
pub fn compress_project(path: &Path) -> Result<Vec<u8>, PluginError> {
    let plugin = get_language_plugin(path)?;
    match plugin {
        Plugin::CSharp(_) => Ok(tmc_zip::zip(
            <CSharpPlugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
        )?),
        Plugin::Make(_) => Ok(tmc_zip::zip(
            <MakePlugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
        )?),
        Plugin::Maven(_) => Ok(tmc_zip::zip(
            <MavenPlugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
        )?),
        Plugin::NoTests(_) => Ok(tmc_zip::zip(
            <NoTestsPlugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
        )?),
        Plugin::Python3(_) => Ok(tmc_zip::zip(
            <Python3Plugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
        )?),
        Plugin::R(_) => Ok(tmc_zip::zip(
            <RPlugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
        )?),
        Plugin::Ant(_) => Ok(tmc_zip::zip(
            <AntPlugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
        )?),
    }
}

// enum containing all the plugins
#[impl_enum::with_methods(
    pub fn clean(&self, path: &Path) -> Result<(), TmcError> {}
    pub fn get_exercise_packaging_configuration(config: TmcProjectYml) -> Result<ExercisePackagingConfiguration, TmcError> {}
    pub fn extract_project(compressed_project: impl std::io::Read + std::io::Seek, target_location: &Path, clean: bool) -> Result<(), TmcError> {}
    pub fn extract_student_files(compressed_project: impl std::io::Read + std::io::Seek, target_location: &Path) -> Result<(), TmcError> {}
    pub fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc, TmcError> {}
    pub fn run_tests(&self, path: &Path) -> Result<RunResult, TmcError> {}
    pub fn check_code_style(&self, path: &Path, locale: Language) -> Result<Option<StyleValidationResult>, TmcError> {}
    pub fn get_available_points(exercise_path: &Path) -> Result<Vec<String>, TmcError> {}
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

// Get language plugin for the given path.
pub fn get_language_plugin(path: &Path) -> Result<Plugin, TmcError> {
    if NoTestsPlugin::is_exercise_type_correct(path) {
        log::info!(
            "Detected project at {} as {}",
            path.display(),
            NoTestsPlugin::PLUGIN_NAME
        );
        Ok(Plugin::NoTests(NoTestsPlugin::new()))
    } else if CSharpPlugin::is_exercise_type_correct(path) {
        let csharp = CSharpPlugin::new();
        log::info!(
            "Detected project at {} as {}",
            path.display(),
            CSharpPlugin::PLUGIN_NAME
        );
        Ok(Plugin::CSharp(csharp))
    } else if MakePlugin::is_exercise_type_correct(path) {
        let make = MakePlugin::new();
        log::info!(
            "Detected project at {} as {}",
            path.display(),
            MakePlugin::PLUGIN_NAME
        );
        Ok(Plugin::Make(make))
    } else if Python3Plugin::is_exercise_type_correct(path) {
        log::info!(
            "Detected project at {} as {}",
            path.display(),
            Python3Plugin::PLUGIN_NAME
        );
        Ok(Plugin::Python3(Python3Plugin::new()))
    } else if RPlugin::is_exercise_type_correct(path) {
        log::info!(
            "Detected project at {} as {}",
            path.display(),
            RPlugin::PLUGIN_NAME
        );
        Ok(Plugin::R(RPlugin::new()))
    } else if MavenPlugin::is_exercise_type_correct(path) {
        log::info!(
            "Detected project at {} as {}",
            path.display(),
            MavenPlugin::PLUGIN_NAME
        );
        Ok(Plugin::Maven(MavenPlugin::new()?))
    } else if AntPlugin::is_exercise_type_correct(path) {
        // TODO: currently, ant needs to be last because any project with src and test are recognized as ant
        log::info!(
            "Detected project at {} as {}",
            path.display(),
            AntPlugin::PLUGIN_NAME
        );
        Ok(Plugin::Ant(AntPlugin::new()?))
    } else {
        Err(TmcError::PluginNotFound(path.to_path_buf()))
    }
}
