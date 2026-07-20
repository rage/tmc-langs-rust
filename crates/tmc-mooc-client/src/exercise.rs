use chrono::{DateTime, Utc};
use mooc_langs_api as api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tmc_langs_util::{JsonError, deserialize};
#[cfg(feature = "ts-rs")]
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct TmcExerciseSlide {
    pub slide_id: Uuid,
    pub exercise_id: Uuid,
    pub exercise_name: String,
    pub exercise_order_number: i32,
    pub deadline: Option<DateTime<Utc>>,
    pub tasks: Vec<TmcExerciseTask>,
}

impl TryFrom<api::ExerciseSlide> for TmcExerciseSlide {
    type Error = JsonError;
    fn try_from(value: api::ExerciseSlide) -> Result<Self, Self::Error> {
        let slide = Self {
            slide_id: value.slide_id,
            exercise_id: value.exercise_id,
            exercise_name: value.exercise_name,
            exercise_order_number: value.exercise_order_number,
            deadline: value.deadline,
            tasks: value
                .tasks
                .into_iter()
                .map(TryFrom::try_from)
                .collect::<Result<_, _>>()?,
        };
        Ok(slide)
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct TmcExerciseTask {
    pub task_id: Uuid,
    pub order_number: i32,
    pub assignment: serde_json::Value,
    pub public_spec: Option<PublicSpec>,
    pub model_solution_spec: Option<ModelSolutionSpec>,
    pub checksum: Option<String>,
}

impl TryFrom<api::ExerciseTask> for TmcExerciseTask {
    type Error = JsonError;
    fn try_from(value: api::ExerciseTask) -> Result<Self, Self::Error> {
        let public_spec: Option<PublicSpec> = value
            .public_spec
            .map(deserialize::json_from_value)
            .transpose()?;
        let task = Self {
            task_id: value.task_id,
            order_number: value.order_number,
            assignment: value.assignment,
            checksum: public_spec.as_ref().map(|ps| ps.checksum.clone()),
            public_spec,
            model_solution_spec: value
                .model_solution_spec
                .map(deserialize::json_from_value)
                .transpose()?,
        };
        Ok(task)
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub enum ExerciseType {
    Browser,
    Editor,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct PublicSpec {
    #[serde(rename = "type")]
    exercise_type: ExerciseType,
    archive_name: String,
    stub_download_url: String,
    student_file_paths: Vec<String>,
    checksum: String,
    /// In-browser test config: script to run in the client and optional error
    /// if the build failed. Omitted for editor exercises or when no script was
    /// built. Serde treats the `Option` field as optional when absent.
    browser_test: Option<BrowserTestSpec>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub enum BrowserTestRuntime {
    Python,
}

/// In-browser test spec produced by the `tmc` exercise service: the script to
/// run in the client plus an optional error set when the build failed.
#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct BrowserTestSpec {
    runtime: BrowserTestRuntime,
    script: String,
    error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
#[allow(unused)]
pub enum UserAnswer {
    Browser { files: Vec<ExerciseFile> },
    Editor { archive_download_url: String },
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub enum ModelSolutionSpec {
    Browser { solution_files: Vec<ExerciseFile> },
    Editor { download_url: String },
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct ExerciseFile {
    filepath: String,
    contents: String,
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn deserializes_browser_public_spec() {
        let browser_task = r#"
{
    "type": "browser",
    "archive_name": "1",
    "stub_download_url": "2",
    "student_file_paths": ["3", "4"],
    "checksum": "abcd",
    "browser_test": {
        "runtime": "python",
        "script": "print('hello')"
    }
}
"#;
        let spec = serde_json::from_str::<PublicSpec>(browser_task).unwrap();
        assert_eq!(spec.exercise_type, ExerciseType::Browser);
        assert_eq!(spec.archive_name, "1");
        assert_eq!(spec.stub_download_url, "2");
        assert_eq!(spec.student_file_paths, vec!["3", "4"]);
        assert_eq!(spec.checksum, "abcd");
        let browser_test = spec.browser_test.expect("browser_test should be present");
        assert_eq!(browser_test.runtime, BrowserTestRuntime::Python);
        assert_eq!(browser_test.script, "print('hello')");
        assert_eq!(browser_test.error, None);
    }

    #[test]
    fn deserializes_browser_public_spec_with_browser_test_error() {
        let browser_task = r#"
{
    "type": "browser",
    "archive_name": "1",
    "stub_download_url": "2",
    "student_file_paths": ["3", "4"],
    "checksum": "abcd",
    "browser_test": {
        "runtime": "python",
        "script": "",
        "error": "template missing test/ or tmc/"
    }
}
"#;
        let spec = serde_json::from_str::<PublicSpec>(browser_task).unwrap();
        let browser_test = spec.browser_test.expect("browser_test should be present");
        assert_eq!(browser_test.runtime, BrowserTestRuntime::Python);
        assert_eq!(browser_test.script, "");
        assert_eq!(
            browser_test.error.as_deref(),
            Some("template missing test/ or tmc/")
        );
    }

    #[test]
    fn deserializes_editor_public_spec() {
        let editor_task = r#"
{
    "type": "editor",
    "archive_name": "1",
    "stub_download_url": "2",
    "student_file_paths": [],
    "checksum": "abcd"
}
"#;
        let spec = serde_json::from_str::<PublicSpec>(editor_task).unwrap();
        assert_eq!(spec.exercise_type, ExerciseType::Editor);
        assert_eq!(spec.archive_name, "1");
        assert_eq!(spec.stub_download_url, "2");
        assert!(spec.student_file_paths.is_empty());
        assert_eq!(spec.checksum, "abcd");
        // `browser_test` is absent for editor exercises and must deserialize to `None`.
        assert!(spec.browser_test.is_none());
    }
}
