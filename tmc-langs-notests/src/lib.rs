//! Language plugin for no_tests exercises

use tmc_langs_framework::{
    domain::{ExerciseDesc, RunResult, RunStatus, TestDesc, TestResult},
    policy::EverythingIsStudentFilePolicy,
    LanguagePlugin, StudentFilePolicy, TmcError,
};

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

#[derive(Default)]
pub struct NoTestsPlugin {}

impl NoTestsPlugin {
    pub fn new() -> Self {
        Self {}
    }

    fn get_points(&self, path: &Path) -> Vec<String> {
        Self::get_student_file_policy(path)
            .get_tmc_project_yml()
            .ok()
            .and_then(|c| c.no_tests.map(|n| n.points))
            .unwrap_or(vec![])
    }
}

impl LanguagePlugin for NoTestsPlugin {
    const PLUGIN_NAME: &'static str = "No-Tests";
    type StudentFilePolicy = EverythingIsStudentFilePolicy;

    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc, TmcError> {
        let test_name = format!("{}Test", exercise_name);
        Ok(ExerciseDesc {
            name: exercise_name,
            tests: vec![TestDesc {
                name: test_name,
                points: self.get_points(path),
            }],
        })
    }

    fn run_tests_with_timeout(
        &self,
        path: &Path,
        _timeout: Option<Duration>,
    ) -> Result<RunResult, TmcError> {
        Ok(RunResult {
            status: RunStatus::Passed,
            test_results: vec![TestResult {
                name: "Default test".to_string(),
                successful: true,
                points: self.get_points(path),
                message: "".to_string(),
                exception: vec![],
            }],
            logs: HashMap::new(),
        })
    }

    fn get_student_file_policy(project_path: &Path) -> Self::StudentFilePolicy {
        EverythingIsStudentFilePolicy::new(project_path.to_path_buf())
    }

    fn is_exercise_type_correct(path: &Path) -> bool {
        Self::get_student_file_policy(path)
            .get_tmc_project_yml()
            .map(|c| c.no_tests.is_some())
            .unwrap_or(false)
    }

    fn clean(&self, _path: &Path) -> Result<(), TmcError> {
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn no_points() {
        init();

        let plugin = NoTestsPlugin {};
        let path = Path::new("tests/data/notests");
        assert!(NoTestsPlugin::is_exercise_type_correct(path));
        let desc = plugin
            .scan_exercise(path, "No Tests Exercise".to_string())
            .unwrap();
        assert_eq!(desc.tests.len(), 1);
        assert_eq!(desc.tests[0].points.len(), 0);
        let runres = plugin.run_tests(path).unwrap();
        assert_eq!(runres.status, RunStatus::Passed);
    }

    #[test]
    fn points() {
        init();

        let plugin = NoTestsPlugin {};
        let path = Path::new("tests/data/notests-points");
        assert!(NoTestsPlugin::is_exercise_type_correct(path));
        let desc = plugin
            .scan_exercise(path, "No Tests Exercise".to_string())
            .unwrap();
        assert_eq!(desc.tests.len(), 1);
        assert_eq!(desc.tests[0].points.len(), 2);
        assert_eq!(desc.tests[0].points[0], "1");
        assert_eq!(desc.tests[0].points[1], "notests");
        let runres = plugin.run_tests(path).unwrap();
        assert_eq!(runres.status, RunStatus::Passed);
    }
}
