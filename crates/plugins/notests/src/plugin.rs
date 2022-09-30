//! The language plugin for no-tests projects.

use crate::NoTestsStudentFilePolicy;
use std::{
    collections::HashMap,
    io::{Read, Seek},
    ops::ControlFlow::{Break, Continue},
    path::{Path, PathBuf},
    time::Duration,
};
use tmc_langs_framework::{
    nom::{self, error::VerboseError, IResult},
    Archive, ExerciseDesc, LanguagePlugin, RunResult, RunStatus, StudentFilePolicy, TestDesc,
    TestResult, TmcError, TmcProjectYml,
};
use tmc_langs_util::{deserialize, path_util};

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

/// Project directory:
/// Contains a .tmcproject.yml file that has `no-tests` set to `true`.
impl LanguagePlugin for NoTestsPlugin {
    const PLUGIN_NAME: &'static str = "No-Tests";
    const LINE_COMMENT: &'static str = "//";
    const BLOCK_COMMENT: Option<(&'static str, &'static str)> = None;
    type StudentFilePolicy = NoTestsStudentFilePolicy;

    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc, TmcError> {
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

    fn is_exercise_type_correct(path: &Path) -> bool {
        Self::StudentFilePolicy::new(path)
            .ok()
            .as_ref()
            .map(|p| p.get_project_config())
            .and_then(|c| c.no_tests.as_ref())
            .map(|nt| nt.flag)
            .unwrap_or(false)
    }

    fn find_project_dir_in_archive<R: Read + Seek>(
        archive: &mut Archive<R>,
    ) -> Result<PathBuf, TmcError> {
        let mut iter = archive.iter()?;

        let project_dir = loop {
            let next = iter.with_next(|file| {
                let file_path = file.path()?;

                if file.is_file() {
                    // check for .tmcproject.yml
                    if let Some(parent) =
                        path_util::get_parent_of_named(&file_path, ".tmcproject.yml")
                    {
                        let tmc_project_yml: TmcProjectYml = deserialize::yaml_from_reader(file)
                            .map_err(|e| TmcError::YamlDeserialize(file_path, e))?;
                        // check no-tests
                        if tmc_project_yml
                            .no_tests
                            .map(|nt| nt.flag)
                            .unwrap_or_default()
                        {
                            return Ok(Break(Some(parent)));
                        }
                    }
                }
                Ok(Continue(()))
            });
            match next? {
                Continue(_) => continue,
                Break(project_dir) => break project_dir,
            }
        };
        if let Some(project_dir) = project_dir {
            Ok(project_dir)
        } else {
            Err(TmcError::NoProjectDirInArchive)
        }
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

    fn points_parser(_: &str) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
        // never parses anything
        Err(nom::Err::Error(VerboseError { errors: vec![] }))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
            .scan_exercise(Path::new("/nonexistent path"), "ex".to_string())
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
