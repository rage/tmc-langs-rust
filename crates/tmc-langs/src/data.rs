//! Various data types.

use crate::error::{LangsError, ParamError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fmt::{Display, Formatter, Result as FmtResult},
    path::PathBuf,
};
use tmc_testmycode_client::response::{CourseData, CourseDetails, CourseExercise};

/// Exercise inside the projects directory.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct LocalExercise {
    pub exercise_slug: String,
    pub exercise_path: PathBuf,
}

/// TmcParams is used to safely construct data for a .tmcparams file, which contains lines in the form of
/// export A=B
/// export C=(D, E, F)
/// the keys and values of the inner hashmap are validated to make sure they are valid as bash variables
#[derive(Debug, Default)]
pub struct TmcParams(pub HashMap<ShellString, TmcParam>);

impl TmcParams {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn insert_string<S: AsRef<str>, T: AsRef<str>>(
        &mut self,
        key: S,
        value: T,
    ) -> Result<(), LangsError> {
        // validate key
        let key = {
            let key = key.as_ref();
            match Self::is_valid_key(key) {
                Ok(()) => ShellString(key.to_string()),
                Err(e) => return Err(LangsError::InvalidParam(key.to_string(), e)),
            }
        };

        // validate value
        let value = {
            let value = value.as_ref();
            match Self::is_valid_value(value) {
                Ok(()) => ShellString(value.to_string()),
                Err(e) => return Err(LangsError::InvalidParam(value.to_string(), e)),
            }
        };

        self.0.insert(key, TmcParam::String(value));
        Ok(())
    }

    pub fn insert_array<S: AsRef<str>, T: AsRef<str>>(
        &mut self,
        key: S,
        values: Vec<T>,
    ) -> Result<(), LangsError> {
        let key = {
            let key = key.as_ref();
            match Self::is_valid_key(key) {
                Ok(()) => ShellString(key.to_string()),
                Err(e) => return Err(LangsError::InvalidParam(key.to_string(), e)),
            }
        };

        let values = values
            .into_iter()
            .map(|s| {
                let s = s.as_ref();
                match Self::is_valid_value(s) {
                    Ok(()) => Ok(ShellString(s.to_string())),
                    Err(e) => Err(LangsError::InvalidParam(s.to_string(), e)),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        self.0.insert(key, TmcParam::Array(values));
        Ok(())
    }

    fn is_valid_key<S: AsRef<str>>(string: S) -> Result<(), ParamError> {
        if string.as_ref().is_empty() {
            return Err(ParamError::Empty);
        }

        for c in string.as_ref().chars() {
            if !c.is_ascii_alphabetic() && c != '_' {
                return Err(ParamError::InvalidChar(c));
            }
        }
        Ok(())
    }

    fn is_valid_value<S: AsRef<str>>(string: S) -> Result<(), ParamError> {
        if string.as_ref().is_empty() {
            return Err(ParamError::Empty);
        }

        for c in string.as_ref().chars() {
            if !c.is_ascii_alphabetic() && c != '_' && c != '-' {
                return Err(ParamError::InvalidChar(c));
            }
        }
        Ok(())
    }
}

// string checked to be a valid shell string
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct ShellString(String);

/// .tmcparams variables can be strings or arrays
#[derive(Debug)]
pub enum TmcParam {
    String(ShellString),
    Array(Vec<ShellString>),
}

// the Display impl escapes the inner strings with shellwords
impl Display for TmcParam {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::String(s) => s.fmt(f),
            Self::Array(v) => write!(
                f,
                "( {} )",
                v.iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        }
    }
}

// the Display impl escapes the inner string with shellwords
impl Display for ShellString {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", shellwords::escape(&self.0))
    }
}

#[derive(Debug)]
pub enum DownloadResult {
    Success {
        downloaded: Vec<ExerciseDownload>,
        skipped: Vec<ExerciseDownload>,
    },
    Failure {
        downloaded: Vec<ExerciseDownload>,
        skipped: Vec<ExerciseDownload>,
        failed: Vec<(ExerciseDownload, Vec<String>)>,
    },
}

pub struct DownloadTarget {
    pub target: ExerciseDownload,
    pub checksum: String,
    pub kind: DownloadTargetKind,
}

pub enum DownloadTargetKind {
    Template,
    Submission { submission_id: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct ExerciseDownload {
    pub id: u32,
    pub course_slug: String,
    pub exercise_slug: String,
    pub path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct CombinedCourseData {
    pub details: CourseDetails,
    pub exercises: Vec<CourseExercise>,
    pub settings: CourseData,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct DownloadOrUpdateCourseExercisesResult {
    pub downloaded: Vec<ExerciseDownload>,
    pub skipped: Vec<ExerciseDownload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<Vec<(ExerciseDownload, Vec<String>)>>,
}
