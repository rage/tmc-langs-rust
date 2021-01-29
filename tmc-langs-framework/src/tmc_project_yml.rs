//! Contains a struct that models the .tmcproject.yml file.

use crate::file_util;
use crate::TmcError;
use serde::{
    de::{Error, Visitor},
    Deserialize, Deserializer,
};
use std::fmt;
use std::path::{Path, PathBuf};

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
            log::trace!("no config found at {}", config_path.display());
            return Ok(Self::default());
        }
        log::debug!("reading .tmcproject.yml from {}", config_path.display());
        let file = file_util::open_file(&config_path)?;
        let config = serde_yaml::from_reader(file)?;
        log::trace!("read {:#?}", config);
        Ok(config)
    }
}

/// Minimum Python version requirement.
/// TODO: if patch is Some minor is also guaranteed to be Some etc. encode this in the type system
#[derive(Debug, Default)]
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
#[derive(Debug, Deserialize)]
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
        assert_eq!(no_tests.points, &["1", "notests"]);
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
