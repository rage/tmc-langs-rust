//! The language plugin for no-tests projects.

use crate::NoTestsStudentFilePolicy;
use std::collections::HashMap;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tmc_langs_framework::{
    anyhow,
    domain::{ExerciseDesc, RunResult, RunStatus, TestDesc, TestResult},
    nom::{self, error::VerboseError, IResult},
    zip::ZipArchive,
    LanguagePlugin, StudentFilePolicy, TmcError,
};

#[derive(Default)]
pub struct NoTestsPlugin {}

impl NoTestsPlugin {
    pub fn new() -> Self {
        Self {}
    }

    /// Convenience function to get a list of the points for the project at path.
    fn get_points(path: &Path) -> Vec<String> {
        <Self as LanguagePlugin>::StudentFilePolicy::new(path)
            .ok()
            .as_ref()
            .map(|p| p.get_project_config())
            .and_then(|c| c.no_tests.as_ref().map(|n| n.points.clone()))
            .unwrap_or_default()
    }
}

impl LanguagePlugin for NoTestsPlugin {
    const PLUGIN_NAME: &'static str = "No-Tests";
    const LINE_COMMENT: &'static str = "//";
    const BLOCK_COMMENT: Option<(&'static str, &'static str)> = None;
    type StudentFilePolicy = NoTestsStudentFilePolicy;

    fn scan_exercise(
        &self,
        path: &Path,
        exercise_name: String,
        _warnings: &mut Vec<anyhow::Error>,
    ) -> Result<ExerciseDesc, TmcError> {
        let test_name = format!("{}Test", exercise_name);
        Ok(ExerciseDesc {
            name: exercise_name,
            tests: vec![TestDesc {
                name: test_name,
                points: Self::get_points(path),
            }],
        })
    }

    fn run_tests_with_timeout(
        &self,
        path: &Path,
        _timeout: Option<Duration>,
        _warnings: &mut Vec<anyhow::Error>,
    ) -> Result<RunResult, TmcError> {
        Ok(RunResult {
            status: RunStatus::Passed,
            test_results: vec![TestResult {
                name: "Default test".to_string(),
                successful: true,
                points: Self::get_points(path),
                message: "".to_string(),
                exception: vec![],
            }],
            logs: HashMap::new(),
        })
    }

    /// Checks the no-tests field of in path/.tmcproject.yml, if any.
    fn is_exercise_type_correct(path: &Path) -> bool {
        Self::StudentFilePolicy::new(path)
            .ok()
            .as_ref()
            .map(|p| p.get_project_config())
            .and_then(|c| c.no_tests.as_ref())
            .map(|nt| nt.flag)
            .unwrap_or(false)
    }

    fn find_project_dir_in_zip<R: Read + Seek>(
        _zip_archive: &mut ZipArchive<R>,
    ) -> Result<PathBuf, TmcError> {
        Ok(PathBuf::from(""))
    }

    fn clean(&self, _path: &Path) -> Result<(), TmcError> {
        Ok(())
    }

    fn get_default_student_file_paths() -> Vec<PathBuf> {
        vec![PathBuf::from("src")]
    }

    fn get_default_exercise_file_paths() -> Vec<PathBuf> {
        vec![PathBuf::from("test")]
    }

    fn points_parser(i: &str) -> IResult<&str, &str, VerboseError<&str>> {
        // does not match any characters
        nom::combinator::value("", nom::character::complete::one_of(""))(i)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new().with_level(LevelFilter::Debug).init();
    }

    fn file_to(
        target_dir: impl AsRef<std::path::Path>,
        target_relative: impl AsRef<std::path::Path>,
        contents: impl AsRef<[u8]>,
    ) -> PathBuf {
        let target = target_dir.as_ref().join(target_relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&target, contents.as_ref()).unwrap();
        target
    }

    #[test]
    fn gets_points() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(
            &temp_dir,
            ".tmcproject.yml",
            r#"
no-tests: 
    points:
        - point1
        - point2
        - 3
        - 4
"#,
        );

        let points = NoTestsPlugin::get_points(temp_dir.path());
        assert_eq!(points.len(), 4)
    }

    #[test]
    fn scans_exercise() {
        init();

        let plugin = NoTestsPlugin::new();
        let _exercise_desc = plugin
            .scan_exercise(
                Path::new("/nonexistent path"),
                "ex".to_string(),
                &mut vec![],
            )
            .unwrap();
    }

    #[test]
    fn runs_tests_ignores_timeout() {
        init();

        let plugin = NoTestsPlugin::new();
        let run_result = plugin
            .run_tests_with_timeout(
                Path::new("/nonexistent"),
                Some(std::time::Duration::from_nanos(1)),
                &mut vec![],
            )
            .unwrap();
        assert_eq!(run_result.status, RunStatus::Passed);
    }

    #[test]
    fn exercise_type_is_correct() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(
            &temp_dir,
            ".tmcproject.yml",
            r#"
no-tests: 
    points: [point1]
"#,
        );
        assert!(NoTestsPlugin::is_exercise_type_correct(temp_dir.path()));

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(
            &temp_dir,
            ".tmcproject.yml",
            r#"
no-tests: true
"#,
        );
        assert!(NoTestsPlugin::is_exercise_type_correct(temp_dir.path()));
    }

    #[test]
    fn exercise_type_is_not_correct() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        assert!(!NoTestsPlugin::is_exercise_type_correct(temp_dir.path()));

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(&temp_dir, ".tmcproject.yml", r#""#);
        assert!(!NoTestsPlugin::is_exercise_type_correct(temp_dir.path()));

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(
            &temp_dir,
            ".tmcproject.yml",
            r#"
no-tests: false
"#,
        );
        assert!(!NoTestsPlugin::is_exercise_type_correct(temp_dir.path()));
    }

    #[test]
    fn parses_empty() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "test/.keep", r#""#);

        let points = NoTestsPlugin::get_available_points(temp.path()).unwrap();
        assert!(points.is_empty());

        let temp = tempfile::tempdir().unwrap();
        file_to(
            &temp,
            "test/.keep",
            r#"
"#,
        );

        let points = NoTestsPlugin::get_available_points(temp.path()).unwrap();
        assert!(points.is_empty());
    }
}
