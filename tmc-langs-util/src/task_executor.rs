//! Module for calling different tasks of TMC-langs language plug-ins.

use super::{tar, PLUGINS};
use isolang::Language;
use lazy_static::lazy_static;
use log::info;
use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tmc_langs_abstraction::ValidationResult;
use tmc_langs_framework::{
    domain::{self, ExerciseDesc, ExercisePackagingConfiguration, RunResult},
    io::{zip, NothingIsStudentFilePolicy},
    Error, LanguagePlugin, Result,
};

lazy_static! {
    static ref FILES_TO_SKIP_ALWAYS: Regex =
        Regex::new("\\.tmcrc|metadata\\.yml|(.*)Hidden(.*)").unwrap();
    static ref NON_TEXT_TYPES: Regex =
        Regex::new("class|jar|exe|jpg|jpeg|gif|png|zip|tar|gz|db|bin|csv|tsv|^$").unwrap();
}

/// See `domain::prepare_solutions`.
pub fn prepare_solutions<'a, I: IntoIterator<Item = &'a PathBuf>>(
    exercise_paths: I,
    dest_root: &Path,
) -> Result<()> {
    Ok(domain::prepare_solutions(exercise_paths, dest_root)?)
}

/// See `domain::prepare_stubs`.
pub fn prepare_stubs(
    exercise_map: HashMap<PathBuf, Box<dyn LanguagePlugin>>,
    repo_path: &Path,
    dest_path: &Path,
) -> Result<()> {
    Ok(domain::prepare_stubs(exercise_map, repo_path, dest_path)?)
}

/// Finds the correct language plug-in for the given exercise path and calls `LanguagePlugin::check_code_style`.
pub fn run_check_code_style(path: &Path, locale: Language) -> Result<Option<ValidationResult>> {
    Ok(get_language_plugin(path)?.check_code_style(path, locale))
}

/// Finds the correct language plug-in for the given exercise path and calls `LanguagePlugin::run_tests`.
pub fn run_tests(path: &Path) -> Result<RunResult> {
    Ok(get_language_plugin(path)?.run_tests(&path))
}

/// Finds the correct language plug-in for the given exercise path and calls `LanguagePlugin::scan_exercise`.
pub fn scan_exercise(path: &Path, exercise_name: String) -> Result<ExerciseDesc> {
    Ok(get_language_plugin(path)?.scan_exercise(path, exercise_name)?)
}

/// Figures out if this path contains any exercise that TMC-langs can process.
pub fn is_exercise_root_directory(path: &Path) -> bool {
    get_language_plugin(path).is_ok()
}

/// Finds the correct language plug-in for the given exercise path and calls `LanguagePlugin::extract_project`.
pub fn extract_project(compressed_project: &Path, target_location: &Path) -> Result<()> {
    if let Ok(plugin) = get_language_plugin(compressed_project) {
        plugin.extract_project(compressed_project, target_location);
    } else {
        extract_project_overwrite(compressed_project, target_location);
    }
    Ok(())
}

/// Extract a given archive file containing a compressed project to a target location.
///
/// This will overwrite any existing files as long as they are not specified as student files
/// by the language dependent student file policy.
pub fn extract_project_overwrite(compressed_project: &Path, target_location: &Path) {
    zip::student_file_aware_unzip(
        Box::new(NothingIsStudentFilePolicy {}),
        compressed_project,
        target_location,
    );
}

/// Finds the correct language plug-in for the given exercise path and calls `LanguagePlugin::compress_project`.
pub fn compress_project(path: &Path) -> Result<Vec<u8>> {
    Ok(get_language_plugin(path)?.compress_project(path))
}

/// Finds the correct language plug-in for the given exercise path and calls `LanguagePlugin::get_exercise_packaging_configuration`.
pub fn get_exercise_packaging_configuration(path: &Path) -> Result<ExercisePackagingConfiguration> {
    Ok(get_language_plugin(path)?.get_exercise_packaging_configuration(path)?)
}

/// Creates a tarball that can be submitted to TMC-sandbox.
pub fn compress_tar_for_submitting(
    project_dir: &Path,
    tmc_langs: &Path,
    tmcrun: &Path,
    target_location: &Path,
) -> Result<()> {
    Ok(tar::create_tar_from_project(
        project_dir,
        tmc_langs,
        tmcrun,
        target_location,
    )?)
}

/// Finds the correct language plug-in for the given exercise path and calls `LanguagePlugin::clean`.
pub fn clean(path: &Path) -> Result<()> {
    get_language_plugin(path)?.clean(path);
    Ok(())
}

// Get language plugin for the given path.
fn get_language_plugin(path: &Path) -> Result<&dyn LanguagePlugin> {
    for plugin in PLUGINS.iter() {
        if plugin.is_exercise_type_correct(path) {
            info!("Detected project as {}", plugin.get_plugin_name());
            return Ok(*plugin);
        }
    }
    Err(Error::PluginNotFound)
}
