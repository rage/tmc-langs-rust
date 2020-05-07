use isolang::Language;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tmc_langs_abstraction::ValidationResult;
use tmc_langs_framework::{
    domain::{ExerciseDesc, ExercisePackagingConfiguration, RunResult},
    LanguagePlugin,
};

pub struct Python3Plugin {}

impl Python3Plugin {
    pub fn new() -> Self {
        todo!()
    }
}

impl LanguagePlugin for Python3Plugin {
    fn get_plugin_name(&self) -> String {
        todo!()
    }

    fn find_exercises(&self, base_path: &Path) -> Vec<PathBuf> {
        todo!()
    }

    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Option<ExerciseDesc> {
        todo!()
    }

    fn run_tests(&self, path: &Path) -> RunResult {
        todo!()
    }

    fn prepare_submission(&self, submission_path: &Path, dest_path: &Path) {
        todo!()
    }

    fn prepare_stubs(
        &self,
        exercise_map: HashMap<PathBuf, Box<dyn LanguagePlugin>>,
        repo_path: &Path,
        dest_path: &Path,
    ) {
        todo!()
    }

    fn prepare_solutions(
        &self,
        exercise_map: HashMap<PathBuf, Box<dyn LanguagePlugin>>,
        repo_path: &Path,
        dest_path: &Path,
    ) {
        todo!()
    }

    fn check_code_style(&self, path: &Path, locale: Language) -> ValidationResult {
        todo!()
    }

    fn compress_project(&self, path: &Path) -> Vec<u8> {
        todo!()
    }

    fn extract_project(&self, compressed_project: &Path, target_location: &Path) {
        todo!()
    }

    fn is_exercise_type_correct(&self, path: &Path) -> bool {
        todo!()
    }

    fn maybe_copy_shared_stuff(&self, dest_path: &Path) {
        todo!()
    }

    fn get_exercise_packaging_configuration(&self, path: &Path) -> ExercisePackagingConfiguration {
        todo!()
    }

    fn clean(&self, path: &Path) {
        todo!()
    }
}
