pub mod domain;

use domain::{ExerciseDesc, ExercisePackagingConfiguration, RunResult};
use isolang::Language;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tmc_langs_abstraction::ValidationResult;

#[cfg_attr(test, mockall::automock)]
pub trait LanguagePlugin {
    fn get_plugin_name(&self) -> String;

    fn find_exercises(&self, base_path: &Path) -> Vec<PathBuf>;

    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Option<ExerciseDesc>;

    fn run_tests(&self, path: &Path) -> RunResult;

    fn prepare_submission(&self, submission_path: &Path, dest_path: &Path);

    fn prepare_stubs(
        &self,
        exercise_map: HashMap<PathBuf, Box<dyn LanguagePlugin>>,
        repo_path: &Path,
        dest_path: &Path,
    );

    fn prepare_solutions(
        &self,
        exercise_map: HashMap<PathBuf, Box<dyn LanguagePlugin>>,
        repo_path: &Path,
        dest_path: &Path,
    );

    fn check_code_style(&self, path: &Path, locale: Language) -> ValidationResult;

    fn compress_project(&self, path: &Path) -> Vec<u8>;

    fn extract_project(&self, compressed_project: &Path, target_location: &Path);

    fn is_exercise_type_correct(&self, path: &Path) -> bool;

    fn maybe_copy_shared_stuff(&self, dest_path: &Path);

    fn get_exercise_packaging_configuration(&self, path: &Path) -> ExercisePackagingConfiguration;

    fn clean(&self, path: &Path);
}
