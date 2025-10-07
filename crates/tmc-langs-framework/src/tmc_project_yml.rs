//! Contains a struct that models the .tmcproject.yml file.

use crate::TmcError;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{Error, Visitor},
};
use std::{
    fmt::{self, Display},
    path::{Path, PathBuf},
};
use tmc_langs_util::{
    deserialize,
    file_util::{self, Lock, LockOptions},
};

const DEFAULT_SUBMISSION_SIZE_LIMIT_MB: u32 = 1;

/// Extra data from a `.tmcproject.yml` file.
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct TmcProjectYml {
    /// A list of files or directories that will always be considered student files.
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extra_student_files: Vec<PathBuf>,

    /// A list of files or directories that will always be considered exercise files.
    /// `extra_student_files` takes precedence if a file is both an extra student file and an extra exercise file.
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extra_exercise_files: Vec<PathBuf>,

    /// A list of files that should always be overwritten by updates even if they are student files.
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub force_update: Vec<PathBuf>,

    /// If set, tests are forcibly stopped after this duration.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tests_timeout_ms: Option<u32>,

    /// Marks the exercise as not containing any tests.
    #[serde(rename = "no-tests")]
    #[cfg_attr(feature = "ts-rs", ts(skip))]
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_tests: Option<NoTests>,

    /// If set, Valgrind errors will be considered test errors.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fail_on_valgrind_error: Option<bool>,

    /// If set, will cause an error telling the student to update their Python if their version is older than the minimum.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_python_version: Option<PythonVer>,

    /// Overrides the default sandbox image. e.g. `eu.gcr.io/moocfi-public/tmc-sandbox-python:latest`
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_image: Option<String>,

    /// Overrides the default archive size limit (500 Mb).
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submission_size_limit_mb: Option<u32>,
}

impl TmcProjectYml {
    /// Returns the path of the TmcProjectYml file in the given directory.
    fn path_in_dir(dir: &Path) -> PathBuf {
        dir.join(".tmcproject.yml")
    }

    /// Loads a TmcProjectYml either from the directory or the default if there is none.
    pub fn load_or_default(project_dir: &Path) -> Result<Self, TmcError> {
        if let Some(config) = Self::load(project_dir)? {
            Ok(config)
        } else {
            Ok(Self::default())
        }
    }

    /// Loads a TmcProjectYml from the given directory. Returns None if no such file exists.
    pub fn load(project_dir: &Path) -> Result<Option<Self>, TmcError> {
        let mut config_path = project_dir.to_owned();
        config_path.push(".tmcproject.yml");

        if !config_path.exists() {
            log::trace!("no config found at {}", config_path.display());
            return Ok(None);
        }
        log::debug!("reading .tmcproject.yml from {}", config_path.display());
        let file = file_util::open_file(&config_path)?;
        let config = deserialize::yaml_from_reader(file)
            .map_err(|e| TmcError::YamlDeserialize(config_path, e))?;
        log::trace!("read {config:#?}");
        Ok(Some(config))
    }

    /// Merges the contents of `with` with `self`.
    /// Empty or missing values in self are replaced with those from with. Other values are left unchanged.
    /// Notably it does not merge lists together.
    pub fn merge(&mut self, with: Self) {
        let old = std::mem::take(self);
        let new = Self {
            extra_student_files: if self.extra_student_files.is_empty() {
                with.extra_student_files
            } else {
                old.extra_student_files
            },
            extra_exercise_files: if self.extra_exercise_files.is_empty() {
                with.extra_exercise_files
            } else {
                old.extra_exercise_files
            },
            force_update: if self.force_update.is_empty() {
                with.force_update
            } else {
                old.force_update
            },
            tests_timeout_ms: old.tests_timeout_ms.or(with.tests_timeout_ms),
            fail_on_valgrind_error: old.fail_on_valgrind_error.or(with.fail_on_valgrind_error),
            minimum_python_version: old.minimum_python_version.or(with.minimum_python_version),
            sandbox_image: old.sandbox_image.or(with.sandbox_image),
            no_tests: old.no_tests.or(with.no_tests),
            submission_size_limit_mb: old
                .submission_size_limit_mb
                .or(with.submission_size_limit_mb),
        };
        *self = new;
    }

    /// Saves the TmcProjectYml to the given directory.
    pub fn save_to_dir(&self, dir: &Path) -> Result<(), TmcError> {
        let config_path = Self::path_in_dir(dir);
        let mut lock = Lock::file(&config_path, LockOptions::WriteCreate)?;
        let mut guard = lock.lock()?;
        serde_yaml::to_writer(guard.get_file_mut(), &self)?;
        Ok(())
    }

    pub fn get_submission_size_limit_mb(&self) -> u32 {
        self.submission_size_limit_mb
            .unwrap_or(DEFAULT_SUBMISSION_SIZE_LIMIT_MB)
    }
}

/// Python version from TmcProjectYml.
#[derive(Debug, Default, Clone, Copy, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct PythonVer {
    major: u32,
    minor: Option<u32>,
    patch: Option<u32>,
}

impl PythonVer {
    // Try to keep up to date with https://devguide.python.org/versions/#versions
    // As of writing, 3.7 is the oldest maintained release and its EOL 2023-06-27
    pub fn recommended() -> (u32, u32, u32) {
        (3, 7, 0)
    }

    /// Returns the Python version as a (major, minor, patch) tuple.
    /// Defaults None values to to 3.0.0.
    pub fn min(self) -> (u32, u32, u32) {
        let major = self.major;
        let minor = self.minor.unwrap_or(0);
        let patch = self.patch.unwrap_or(0);
        (major, minor, patch)
    }
}

impl Display for PythonVer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.major, self.minor, self.patch) {
            (major, Some(minor), Some(patch)) => write!(f, "{major}.{minor}.{patch}"),
            (major, Some(minor), None) => write!(f, "{major}.{minor}"),
            (major, None, _) => write!(f, "{major}"),
        }
    }
}

/// Deserializes a major.minor?.patch? version into a PythonVer.
impl<'de> Deserialize<'de> for PythonVer {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PythonVerVisitor;

        impl Visitor<'_> for PythonVerVisitor {
            type Value = PythonVer;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("A string in one of the following formats: {major_ver}, {major_ver}.{minor_ver}, or {major_ver}.{minor_ver}.{patch_ver} where each version is a non-negative integer")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                let mut parts = v.split('.');
                let major = parts
                    .next()
                    .expect("split always yields at least one value");
                let major: u32 = major.parse().map_err(Error::custom)?;
                let minor = if let Some(minor) = parts.next() {
                    let parsed: u32 = minor.parse().map_err(Error::custom)?;
                    Some(parsed)
                } else {
                    None
                };
                let patch = if let Some(patch) = parts.next() {
                    let parsed: u32 = patch.parse().map_err(Error::custom)?;
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

/// Contents of the no-tests field.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
// we never take these structs as inputs from TS so it's ok to ignore from
#[cfg_attr(feature = "ts-rs", ts(ignore_serde_attr = "from"))]
#[serde(from = "NoTestsWrapper")]
pub struct NoTests {
    pub flag: bool,
    pub points: Vec<String>,
}

/// Converts the wrapper type into the more convenient one.
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

/// no-tests can either be a bool or a list of points.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum NoTestsWrapper {
    Flag(bool),
    Points(NoTestsPoints),
}

/// The list of points can contain numbers or strings.
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new().with_level(LevelFilter::Debug).init();
    }

    #[test]
    fn deserialize_no_tests() {
        init();

        let no_tests_yml = r#"no-tests:
  points:
    - 1
    - notests
"#;

        let cfg: TmcProjectYml = deserialize::yaml_from_str(no_tests_yml).unwrap();
        let no_tests = cfg.no_tests.unwrap();
        assert!(no_tests.flag);
        assert_eq!(no_tests.points, &["1", "notests"]);
    }

    #[test]
    fn deserialize_python_ver() {
        init();

        let python_ver: PythonVer = deserialize::yaml_from_str("1.2.3").unwrap();
        assert_eq!(python_ver.major, 1);
        assert_eq!(python_ver.minor, Some(2));
        assert_eq!(python_ver.patch, Some(3));

        let python_ver: PythonVer = deserialize::yaml_from_str("1.2").unwrap();
        assert_eq!(python_ver.major, 1);
        assert_eq!(python_ver.minor, Some(2));
        assert_eq!(python_ver.patch, None);

        let python_ver: PythonVer = deserialize::yaml_from_str("1").unwrap();
        assert_eq!(python_ver.major, 1);
        assert_eq!(python_ver.minor, None);
        assert_eq!(python_ver.patch, None);

        assert!(deserialize::yaml_from_str::<PythonVer>("asd").is_err())
    }

    #[test]
    fn merges() {
        init();

        let tpy_root = TmcProjectYml {
            tests_timeout_ms: Some(123),
            fail_on_valgrind_error: Some(true),
            ..Default::default()
        };
        let mut tpy_exercise = TmcProjectYml {
            tests_timeout_ms: Some(234),
            ..Default::default()
        };
        tpy_exercise.merge(tpy_root);
        assert_eq!(tpy_exercise.tests_timeout_ms, Some(234));
        assert_eq!(tpy_exercise.fail_on_valgrind_error, Some(true));
    }

    #[test]
    fn saves_to_dir() {
        init();

        let temp = tempfile::tempdir().unwrap();
        let path = TmcProjectYml::path_in_dir(temp.path());

        assert!(!path.exists());

        let tpy = TmcProjectYml {
            tests_timeout_ms: Some(1234),
            ..Default::default()
        };
        tpy.save_to_dir(temp.path()).unwrap();

        assert!(path.exists());
        let tpy = TmcProjectYml::load(temp.path()).unwrap().unwrap();
        assert_eq!(tpy.tests_timeout_ms, Some(1234));
    }
}
