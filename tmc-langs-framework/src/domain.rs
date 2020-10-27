//! Contains structs that model data related to exercises.

pub mod meta_syntax;

use crate::io::file_util;
use crate::TmcError;
use log::debug;
use schemars::JsonSchema;
use serde::{
    de::{Error, Visitor},
    Deserialize, Deserializer, Serialize,
};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

/// A description of an exercise's test case.
#[derive(Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Hash)]
pub struct TestDesc {
    /// The full name of the test.
    ///
    /// If the language organises tests into suites or classes, it is customary
    /// to name the test as "class_name.method_name".
    pub name: String,
    /// The list of point names that passing this test may give.
    ///
    /// To obtain a point X, the user must pass all exercises that require point X.
    pub points: Vec<String>,
}

impl TestDesc {
    pub fn new(name: String, points: Vec<String>) -> Self {
        Self { name, points }
    }
}

/// The result of a single test case.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct TestResult {
    pub name: String,
    pub successful: bool,
    pub points: Vec<String>,
    pub message: String,
    #[serde(default)]
    pub exception: Vec<String>,
}

/// A description of an exercise.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ExerciseDesc {
    /// The name of the exercise to be shown to the user.
    /// Does not necessarily match or even contain the directory name.
    pub name: String,
    /// Descriptions of the tests that will be run for this exercise.
    pub tests: Vec<TestDesc>,
}

impl ExerciseDesc {
    pub fn new(name: String, tests: Vec<TestDesc>) -> Self {
        Self { name, tests }
    }
}

/// The result of running an exercise's test suite against a submission.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunResult {
    /// The overall status of a test run.
    pub status: RunStatus,
    /// Whether each test passed and which points were awarded.
    pub test_results: Vec<TestResult>,
    /// Logs from the test run.
    /// The key may be an arbitrary string identifying the type of log.
    pub logs: HashMap<String, String>,
}

impl RunResult {
    pub fn new(
        status: RunStatus,
        test_results: Vec<TestResult>,
        logs: HashMap<String, String>,
    ) -> Self {
        Self {
            status,
            test_results,
            logs,
        }
    }
}

/// The overall status of a test run.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RunStatus {
    /// The submission and tests compiled and all tests passed.
    Passed,
    /// The submission and tests compiled but some tests failed.
    TestsFailed,
    /// The submission or tests did not compile.
    // TODO: "The compiler error should be given in {@code logs[SpecialLogs.COMPILER_OUTPUT]}."
    CompileFailed,
    /// The submission compiled but testrun was interrupted.
    TestrunInterrupted,
    /// For when no other status seems suitable, or the language plugin has
    /// suffered an internal error.
    // TODO: "Details should be given in {@code logs[SpecialLogs.GENERIC_ERROR_MESSAGE]}.""
    GenericError,
}

/// Represents configuration based on which submission may be packaged.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ExercisePackagingConfiguration {
    /// Student folders or files which are copied from submission.
    pub student_file_paths: HashSet<PathBuf>,
    /// Exercise folders or files which are copied from exercise template or clone.
    pub exercise_file_paths: HashSet<PathBuf>,
}

impl ExercisePackagingConfiguration {
    pub fn new(
        student_file_paths: HashSet<PathBuf>,
        exercise_file_paths: HashSet<PathBuf>,
    ) -> Self {
        Self {
            student_file_paths,
            exercise_file_paths,
        }
    }
}

/// Extra data from a `.tmcproject.yml` file.
#[derive(Debug, Deserialize, Default)]
pub struct TmcProjectYml {
    #[serde(default)]
    pub extra_student_files: Vec<PathBuf>,

    #[serde(default)]
    pub extra_exercise_files: Vec<PathBuf>,

    #[serde(default)]
    pub force_update: Vec<PathBuf>,

    #[serde(default)]
    pub tests_timeout_ms: Option<u64>,

    #[serde(default)]
    #[serde(rename = "no-tests")]
    pub no_tests: Option<NoTests>,

    #[serde(default)]
    pub fail_on_valgrind_error: Option<bool>,

    #[serde(default)]
    pub minimum_python_version: Option<PythonVer>,
}

impl TmcProjectYml {
    pub fn from(project_dir: &Path) -> Result<Self, TmcError> {
        let mut config_path = project_dir.to_owned();
        config_path.push(".tmcproject.yml");

        if !config_path.exists() {
            debug!("no config found at {}", config_path.display());
            return Ok(Self::default());
        }
        debug!("reading .tmcproject.yml from {}", config_path.display());
        let file = file_util::open_file(&config_path)?;
        Ok(serde_yaml::from_reader(file)?)
    }
}

#[derive(Debug, Default)]
pub struct PythonVer {
    pub major: Option<usize>,
    pub minor: Option<usize>,
    pub patch: Option<usize>,
}

impl<'de> Deserialize<'de> for PythonVer {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PythonVerVisitor;

        impl<'de> Visitor<'de> for PythonVerVisitor {
            type Value = PythonVer;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("A string in one of the following formats: {major_ver}, {major_ver}.{minor_ver}, or {major_ver}.{minor_ver}.{patch_ver} where each version is a non-negative integer")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                let mut parts = v.split('.');
                let major = if let Some(major) = parts.next() {
                    let parsed: usize = major.parse().map_err(Error::custom)?;
                    Some(parsed)
                } else {
                    None
                };
                let minor = if let Some(minor) = parts.next() {
                    let parsed: usize = minor.parse().map_err(Error::custom)?;
                    Some(parsed)
                } else {
                    None
                };
                let patch = if let Some(patch) = parts.next() {
                    let parsed: usize = patch.parse().map_err(Error::custom)?;
                    Some(parsed)
                } else {
                    None
                };
                Ok(PythonVer {
                    major,
                    minor,
                    patch,
                })
            }
        }

        deserializer.deserialize_str(PythonVerVisitor)
    }
}

#[derive(Debug, Deserialize)]
#[serde(from = "NoTestsWrapper")]
pub struct NoTests {
    pub flag: bool,
    pub points: Vec<String>,
}

impl From<NoTestsWrapper> for NoTests {
    fn from(wrapper: NoTestsWrapper) -> Self {
        match wrapper {
            NoTestsWrapper::Flag(flag) => Self {
                flag,
                points: vec![],
            },
            NoTestsWrapper::Points(no_tests_points) => Self {
                flag: true,
                points: no_tests_points
                    .points
                    .into_iter()
                    .map(|v| match v {
                        IntOrString::Int(i) => i.to_string(),
                        IntOrString::String(s) => s,
                    })
                    .collect(),
            },
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum NoTestsWrapper {
    Flag(bool),
    Points(NoTestsPoints),
}

#[derive(Debug, Deserialize)]
pub struct NoTestsPoints {
    pub points: Vec<IntOrString>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum IntOrString {
    Int(isize),
    String(String),
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum Strategy {
    Fail,
    Warn,
    Disabled,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidationError {
    pub column: usize,
    pub line: usize,
    pub message: String,
    pub source_name: String,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidationResult {
    pub strategy: Strategy,
    pub validation_errors: Option<HashMap<PathBuf, Vec<ValidationError>>>,
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn deserialize_no_tests() {
        let no_tests_yml = r#"no-tests:
  points:
    - 1
    - notests
"#;

        let cfg: TmcProjectYml = serde_yaml::from_str(no_tests_yml).unwrap();
        let no_tests = cfg.no_tests.unwrap();
        assert!(no_tests.flag);
        assert!(!no_tests.points.is_empty());
    }

    #[test]
    fn deserialize_python_ver() {
        let python_ver: PythonVer = serde_yaml::from_str("1.2.3").unwrap();
        assert_eq!(python_ver.major, Some(1));
        assert_eq!(python_ver.minor, Some(2));
        assert_eq!(python_ver.patch, Some(3));

        let python_ver: PythonVer = serde_yaml::from_str("1.2").unwrap();
        assert_eq!(python_ver.major, Some(1));
        assert_eq!(python_ver.minor, Some(2));
        assert_eq!(python_ver.patch, None);

        let python_ver: PythonVer = serde_yaml::from_str("1").unwrap();
        assert_eq!(python_ver.major, Some(1));
        assert_eq!(python_ver.minor, None);
        assert_eq!(python_ver.patch, None);

        assert!(serde_yaml::from_str::<PythonVer>("asd").is_err())
    }
}
