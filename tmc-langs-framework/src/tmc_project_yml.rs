//! Contains a struct that models the .tmcproject.yml file.

use crate::TmcError;
use serde::{
    de::{Error, Visitor},
    Deserialize, Deserializer, Serialize,
};
use std::fmt;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use tmc_langs_util::{file_util, FileError};

/// Extra data from a `.tmcproject.yml` file.
// NOTE: when adding fields, remember to update the merge function as well
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct TmcProjectYml {
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extra_student_files: Vec<PathBuf>,

    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extra_exercise_files: Vec<PathBuf>,

    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub force_update: Vec<PathBuf>,

    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tests_timeout_ms: Option<u64>,

    #[serde(default)]
    #[serde(rename = "no-tests")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_tests: Option<NoTests>,

    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fail_on_valgrind_error: Option<bool>,

    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_python_version: Option<PythonVer>,

    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_image: Option<String>,
}

impl TmcProjectYml {
    fn path_in_dir(dir: &Path) -> PathBuf {
        dir.join(".tmcproject.yml")
    }

    /// Tries to load a
    pub fn load_or_default(project_dir: &Path) -> Result<Self, TmcError> {
        if let Some(config) = Self::load(project_dir)? {
            Ok(config)
        } else {
            Ok(Self::default())
        }
    }

    pub fn load(project_dir: &Path) -> Result<Option<Self>, TmcError> {
        let mut config_path = project_dir.to_owned();
        config_path.push(".tmcproject.yml");

        if !config_path.exists() {
            log::trace!("no config found at {}", config_path.display());
            return Ok(None);
        }
        log::debug!("reading .tmcproject.yml from {}", config_path.display());
        let file = file_util::open_file(&config_path)?;
        let config = serde_yaml::from_reader(file)?;
        log::trace!("read {:#?}", config);
        Ok(Some(config))
    }

    /// Merges the contents of `with` with `self`.
    /// Empty or missing values in self are replaced with those from with. Other values are left unchanged.
    pub fn merge(&mut self, with: Self) {
        if self.extra_student_files.is_empty() {
            self.extra_student_files = with.extra_student_files;
        }
        if self.extra_exercise_files.is_empty() {
            self.extra_exercise_files = with.extra_exercise_files;
        }
        if self.force_update.is_empty() {
            self.force_update = with.force_update;
        }
        if self.tests_timeout_ms.is_none() {
            self.tests_timeout_ms = with.tests_timeout_ms;
        }
        if self.no_tests.is_none() {
            self.no_tests = with.no_tests;
        }
        if self.fail_on_valgrind_error.is_none() {
            self.fail_on_valgrind_error = with.fail_on_valgrind_error;
        }
        if self.minimum_python_version.is_none() {
            self.minimum_python_version = with.minimum_python_version;
        }
    }

    pub fn save_to_dir(&self, dir: &Path) -> Result<(), TmcError> {
        let config_path = Self::path_in_dir(dir);
        let mut file = file_util::create_file_lock(&config_path)?;
        let guard = file
            .lock()
            .map_err(|e| FileError::FdLock(config_path.clone(), e))?;
        serde_yaml::to_writer(guard.deref(), &self)?;
        Ok(())
    }
}

/// Minimum Python version requirement.
/// TODO: if patch is Some minor is also guaranteed to be Some etc. encode this in the type system
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct PythonVer {
    pub major: Option<usize>,
    pub minor: Option<usize>,
    pub patch: Option<usize>,
}

/// Deserializes a major.minor?.patch? version into a PythonVer.
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

/// Contents of the no-tests field.
#[derive(Debug, Serialize, Deserialize, Clone)]
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

        let cfg: TmcProjectYml = serde_yaml::from_str(no_tests_yml).unwrap();
        let no_tests = cfg.no_tests.unwrap();
        assert!(no_tests.flag);
        assert_eq!(no_tests.points, &["1", "notests"]);
    }

    #[test]
    fn deserialize_python_ver() {
        init();

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
