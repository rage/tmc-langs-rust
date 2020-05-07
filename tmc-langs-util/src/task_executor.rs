use super::{
    tar, PLUGINS, {Error, Result},
};
use isolang::Language;
use lazy_static::lazy_static;
use log::info;
use regex::Regex;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use tmc_langs_abstraction::ValidationResult;
use tmc_langs_framework::{
    domain::{self, ExerciseDesc, ExercisePackagingConfiguration, RunResult},
    io::zip,
    LanguagePlugin,
};

lazy_static! {
    static ref FILES_TO_SKIP_ALWAYS: Regex =
        Regex::new("\\.tmcrc|metadata\\.yml|(.*)Hidden(.*)").unwrap();
    static ref NON_TEXT_TYPES: Regex =
        Regex::new("class|jar|exe|jpg|jpeg|gif|png|zip|tar|gz|db|bin|csv|tsv|^$").unwrap();
}

/// Walks through each path in ```exercise_map```, processing files and copying them into ```dest_path```.
/// Skips hidden directories, directories that contain a ```.tmcignore``` file in their root, as well as files matching patterns defined in ```FILES_TO_SKIP_ALWAYS``` and directories and files named ```private```.
/// Binary files are copied without extra processing, while text files have solution tags and stubs removed.
pub fn prepare_solutions<'a, I: IntoIterator<Item = &'a PathBuf>>(
    exercise_paths: I,
    dest_root: &Path,
) -> io::Result<()> {
    domain::prepare_solutions(exercise_paths, dest_root)
}

pub fn prepare_stubs(
    exercise_map: HashMap<PathBuf, Box<dyn LanguagePlugin>>,
    repo_path: &Path,
    dest_path: &Path,
) -> io::Result<()> {
    domain::prepare_stubs(exercise_map, repo_path, dest_path)
}

pub fn run_check_code_style(path: &Path, locale: Language) -> Result<ValidationResult> {
    Ok(get_language_plugin(path)?.check_code_style(path, locale))
}

pub fn run_tests(path: &Path) -> Result<RunResult> {
    Ok(get_language_plugin(path)?.run_tests(&path))
}

pub fn scan_exercise(path: &Path, exercise_name: String) -> Result<Option<ExerciseDesc>> {
    Ok(get_language_plugin(path)?.scan_exercise(path, exercise_name))
}

pub fn is_exercise_root_directory(path: &Path) -> bool {
    get_language_plugin(path).is_ok()
}

pub fn extract_project(compressed_project: &Path, target_location: &Path) -> Result<()> {
    get_language_plugin(compressed_project)?.extract_project(compressed_project, target_location);
    Ok(())
}

pub fn extract_project_overwrite(compressed_project: &Path, target_location: &Path) {
    zip::student_file_aware_unzip((), compressed_project, target_location);
}

pub fn compress_project(path: &Path) -> Result<Vec<u8>> {
    Ok(get_language_plugin(path)?.compress_project(path))
}

pub fn get_exercise_packaging_configuration(path: &Path) -> Result<ExercisePackagingConfiguration> {
    Ok(get_language_plugin(path)?.get_exercise_packaging_configuration(path))
}

pub fn compress_tar_for_submitting(
    project_dir: &Path,
    tmc_langs: &Path,
    tmcrun: &Path,
    target_location: &Path,
) {
    tar::create_tar_from_project(project_dir, tmc_langs, tmcrun, target_location)
}

pub fn clean(path: &Path) -> Result<()> {
    get_language_plugin(path)?.clean(path);
    Ok(())
}

/// Get language plugin for the given path.
fn get_language_plugin(path: &Path) -> Result<&'static Box<dyn LanguagePlugin + Sync>> {
    for plugin in PLUGINS.iter() {
        if plugin.is_exercise_type_correct(path) {
            info!("Detected project as {}", plugin.get_plugin_name());
            return Ok(plugin);
        }
    }
    Err(Error::PluginNotFound)
}
