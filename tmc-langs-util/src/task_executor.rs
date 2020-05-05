use isolang::Language;
use std::collections::HashMap;
use std::path::PathBuf;
use tmc_langs_abstraction::ValidationResult;
use tmc_langs_framework::{
    domain::{ExerciseDesc, ExercisePackagingConfiguration, RunResult},
    LanguagePlugin,
};

pub fn prepare_solutions(
    exercise_map: HashMap<PathBuf, Box<dyn LanguagePlugin>>,
    repo_path: PathBuf,
    dest_path: PathBuf,
) {
    todo!()
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
