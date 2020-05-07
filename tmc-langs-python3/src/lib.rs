use isolang::Language;
use std::path::Path;
use tmc_langs_abstraction::ValidationResult;
use tmc_langs_framework::{
    domain::{ExerciseDesc, RunResult},
    LanguagePlugin,
};

pub struct Python3Plugin {}

impl Python3Plugin {
    pub fn new() -> Self {
        todo!()
    }
}

impl LanguagePlugin for Python3Plugin {
    fn get_plugin_name(&self) -> &'static str {
        "python3"
    }

    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Option<ExerciseDesc> {
        todo!()
    }

    fn run_tests(&self, path: &Path) -> RunResult {
        todo!()
    }

    fn check_code_style(&self, path: &Path, locale: Language) -> ValidationResult {
        todo!()
    }

    fn is_exercise_type_correct(&self, path: &Path) -> bool {
        todo!()
    }

    fn clean(&self, path: &Path) {
        // no op
    }
}
