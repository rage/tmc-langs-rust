//! Module for calling different tasks of TMC-langs language plug-ins.

use super::{
    tar, Error, ExerciseDesc, ExercisePackagingConfiguration, RunResult, ValidationResult,
};
use isolang::Language;
use log::info;
use std::path::{Path, PathBuf};
use tmc_langs_framework::{
    io::{submission_processing, zip},
    plugin::LanguagePlugin,
    policy::NothingIsStudentFilePolicy,
};
use tmc_langs_java::ant::AntPlugin;
use tmc_langs_java::maven::MavenPlugin;
use tmc_langs_make::plugin::MakePlugin;
use tmc_langs_python3::Python3Plugin;

/// See `domain::prepare_solutions`.
pub fn prepare_solutions<'a, I: IntoIterator<Item = &'a PathBuf>>(
    exercise_paths: I,
    dest_root: &Path,
) -> Result<(), Error> {
    submission_processing::prepare_solutions(exercise_paths, dest_root)?;
    Ok(())
}

/// See `LanguagePlugin::prepare_stubs`.
pub fn prepare_stubs<I: IntoIterator<Item = PathBuf>>(
    exercise_paths: I,
    repo_path: &Path,
    dest_path: &Path,
) -> Result<(), Error> {
    for exercise_path in exercise_paths {
        let plugin = get_language_plugin(&exercise_path)?;
        plugin.prepare_stub(&exercise_path, repo_path, dest_path)?;
    }
    Ok(())
}

/// See `LanguagePlugin::check_code_style`.
pub fn run_check_code_style(
    path: &Path,
    locale: Language,
) -> Result<Option<ValidationResult>, Error> {
    Ok(get_language_plugin(path)?.check_code_style(path, locale))
}

/// See `LanguagePlugin::run_tests`.
pub fn run_tests(path: &Path) -> Result<RunResult, Error> {
    get_language_plugin(path)?.run_tests(&path)
}

/// See `LanguagePlugin::scan_exercise`.
pub fn scan_exercise(path: &Path, exercise_name: String) -> Result<ExerciseDesc, Error> {
    Ok(get_language_plugin(path)?.scan_exercise(path, exercise_name)?)
}

/// Tries to find a language plugin for the path, returning `true` if one is found.
pub fn is_exercise_root_directory(path: &Path) -> bool {
    get_language_plugin(path).is_ok()
}

/// Finds the correct language plug-in for the given exercise path and calls `LanguagePlugin::extract_project`,
/// If no language plugin matches, see `extract_project_overwrite`.
pub fn extract_project(compressed_project: &Path, target_location: &Path) -> Result<(), Error> {
    if let Ok(plugin) = get_language_plugin(compressed_project) {
        plugin.extract_project(compressed_project, target_location)?;
    } else {
        extract_project_overwrite(compressed_project, target_location)?;
    }
    Ok(())
}

/// Extract a given archive file containing a compressed project to a target location.
/// This will overwrite any existing files.
// TODO: used?
pub fn extract_project_overwrite(
    compressed_project: &Path,
    target_location: &Path,
) -> Result<(), Error> {
    zip::unzip(
        Box::new(NothingIsStudentFilePolicy {}),
        compressed_project,
        target_location,
    )?;
    Ok(())
}

/// See `LanguagePlugin::compress_project`.
pub fn compress_project(path: &Path) -> Result<Vec<u8>, Error> {
    Ok(get_language_plugin(path)?.compress_project(path)?)
}

/// See `LanguagePlugin::get_exercise_packaging_configuration`.
pub fn get_exercise_packaging_configuration(
    path: &Path,
) -> Result<ExercisePackagingConfiguration, Error> {
    Ok(get_language_plugin(path)?.get_exercise_packaging_configuration(path)?)
}

/// Creates a tarball that can be submitted to TMC-sandbox.
// TODO: used?
pub fn compress_tar_for_submitting(
    project_dir: &Path,
    tmc_langs: &Path,
    tmcrun: &Path,
    target_location: &Path,
) -> Result<(), Error> {
    tar::create_tar_from_project(project_dir, tmc_langs, tmcrun, target_location)?;
    Ok(())
}

/// See `LanguagePlugin::clean`.
pub fn clean(path: &Path) -> Result<(), Error> {
    get_language_plugin(path)?.clean(path)?;
    Ok(())
}

// Get language plugin for the given path.
fn get_language_plugin(path: &Path) -> Result<Box<dyn LanguagePlugin>, Error> {
    let plugins: Vec<Box<dyn LanguagePlugin>> = vec![
        Box::new(Python3Plugin::new()),
        Box::new(MavenPlugin::new()?),
        Box::new(AntPlugin::new()?),
        Box::new(MakePlugin::new()),
    ];

    for plugin in plugins {
        if plugin.is_exercise_type_correct(path) {
            info!("Detected project as {}", plugin.get_plugin_name());
            return Ok(plugin);
        }
    }
    Err(Error::PluginNotFound(path.to_path_buf()))
}
