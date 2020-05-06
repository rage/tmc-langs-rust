use isolang::Language;
use lazy_static::lazy_static;
use regex::Regex;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use tmc_langs_abstraction::ValidationResult;
use tmc_langs_framework::{
    domain::{self, ExerciseDesc, ExercisePackagingConfiguration, RunResult},
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
    repo_path: PathBuf,
    dest_path: PathBuf,
) {
    todo!()
}

pub fn run_check_code_style(path: PathBuf, locale: Language) -> ValidationResult {
    todo!()
}

pub fn run_tests(path: PathBuf) -> RunResult {
    todo!()
}

pub fn scan_exercise(path: PathBuf, exercise_name: String) -> Option<ExerciseDesc> {
    todo!()
}

pub fn is_exercise_root_directory(path: PathBuf) -> bool {
    todo!()
}

pub fn extract_project(compressed_project: PathBuf, target_location: PathBuf) {
    todo!()
}

pub fn extract_project_overwrite(compressed_project: PathBuf, target_location: PathBuf) {
    todo!()
}

pub fn extract_and_rewrite_everything(compressed_project: PathBuf, target_location: PathBuf) {
    todo!()
}

pub fn compress_project(path: PathBuf) -> Vec<u8> {
    todo!()
}

pub fn get_exercise_packaging_configuration(path: PathBuf) -> ExercisePackagingConfiguration {
    todo!()
}

pub fn compress_tar_for_submitting(
    project_dir: PathBuf,
    tmc_langs: PathBuf,
    tmcrun: PathBuf,
    target_location: PathBuf,
) {
    todo!()
}

pub fn clean(path: PathBuf) {
    todo!()
}
