use chrono::{DateTime, Utc};
use exercise_services_api as api;
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
    /// The course the exercise belongs to, so a client can locate it without a
    /// separate lookup or an enrolled-course scan.
    pub course_id: Uuid,
    pub exercise_name: String,
    pub exercise_order_number: i32,
    pub deadline: Option<DateTime<Utc>>,
    pub tasks: Vec<TmcExerciseTask>,
}

impl TmcExerciseSlide {
    /// Stub archive download URL for this slide's first editor task, if any.
    /// `None` for browser exercises, which expose no downloadable archive.
    pub fn editor_stub_download_url(&self) -> Option<&str> {
        self.tasks
            .iter()
            .filter_map(|task| task.public_spec.as_ref())
            .find_map(|spec| spec.editor_stub_download_url())
    }

    /// Task id of this slide's first editor task (the one a native client submits
    /// to), if any. `None` for browser exercises, which have no editor task.
    pub fn editor_task_id(&self) -> Option<Uuid> {
        self.editor_task().map(|task| task.task_id)
    }

    /// Checksum of this slide's first editor task, if any. Compared against the
    /// stored one to detect whether the local exercise is out of date.
    pub fn editor_checksum(&self) -> Option<&str> {
        self.editor_task().and_then(|task| task.checksum.as_deref())
    }

    /// This slide's first editor task: the one a native client downloads, works
    /// on, and submits. Browser tasks do not count.
    fn editor_task(&self) -> Option<&TmcExerciseTask> {
        self.tasks.iter().find(|task| {
            task.public_spec
                .as_ref()
                .and_then(|spec| spec.editor_stub_download_url())
                .is_some()
        })
    }
}

impl TryFrom<api::ExerciseSlide> for TmcExerciseSlide {
    type Error = JsonError;
    fn try_from(value: api::ExerciseSlide) -> Result<Self, Self::Error> {
        let slide = Self {
            slide_id: value.slide_id,
            exercise_id: value.exercise_id,
            course_id: value.course_id,
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
    /// In-browser test config; omitted for editor exercises or when no script
    /// was built.
    browser_test: Option<BrowserTestSpec>,
}

impl PublicSpec {
    pub fn exercise_type(&self) -> &ExerciseType {
        &self.exercise_type
    }

    pub fn stub_download_url(&self) -> &str {
        &self.stub_download_url
    }

    /// Returns the stub archive download URL for editor exercises. Returns `None`
    /// for browser exercises, which have no downloadable project archive.
    pub fn editor_stub_download_url(&self) -> Option<&str> {
        match self.exercise_type {
            ExerciseType::Editor => Some(&self.stub_download_url),
            ExerciseType::Browser => None,
        }
    }
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

/// Mirrors the `tmc` exercise service's `ModelSolutionSpec`
/// (`services/tmc/src/util/stateInterfaces.ts`). The backend forwards it once
/// the model solution may be revealed; the solution is an uploaded project
/// archive for both exercise types.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct ModelSolutionSpec {
    #[serde(rename = "type")]
    exercise_type: ExerciseType,
    solution_download_url: String,
}

impl ModelSolutionSpec {
    pub fn exercise_type(&self) -> &ExerciseType {
        &self.exercise_type
    }

    pub fn solution_download_url(&self) -> &str {
        &self.solution_download_url
    }
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

    /// The exact bytes `services/tmc`'s model-solution endpoint emits (pinned on
    /// that side by `modelSolution.test.ts`). A task the student has solved
    /// carries this, so a mismatch breaks every later `mooc exercise`,
    /// `download-exercise` and `submit` for that exercise.
    #[test]
    fn deserializes_the_emitted_model_solution_spec() {
        let editor =
            r#"{ "type": "editor", "solution_download_url": "http://example.com/sol.tar.zst" }"#;
        let spec = serde_json::from_str::<ModelSolutionSpec>(editor).unwrap();
        assert_eq!(spec.exercise_type(), &ExerciseType::Editor);
        assert_eq!(
            spec.solution_download_url(),
            "http://example.com/sol.tar.zst"
        );

        let browser =
            r#"{ "type": "browser", "solution_download_url": "http://example.com/sol.tar.zst" }"#;
        let spec = serde_json::from_str::<ModelSolutionSpec>(browser).unwrap();
        assert_eq!(spec.exercise_type(), &ExerciseType::Browser);
    }

    /// Guards the deserialize (input) side that the serialize-only bindings drift
    /// gate can't catch: parses a mixed editor+browser slide, then re-serializes
    /// and re-parses it.
    #[test]
    fn slide_with_mixed_tasks_round_trips() {
        let editor_task_id = Uuid::new_v4();
        let browser_task_id = Uuid::new_v4();
        let slide_json = serde_json::json!({
            "slide_id": Uuid::new_v4(),
            "exercise_id": Uuid::new_v4(),
            "course_id": Uuid::new_v4(),
            "exercise_name": "mixed",
            "exercise_order_number": 3,
            "deadline": null,
            "tasks": [
                {
                    "task_id": editor_task_id,
                    "order_number": 0,
                    "assignment": [{"type": "paragraph"}],
                    "public_spec": {
                        "type": "editor",
                        "archive_name": "stub.tar.zst",
                        "stub_download_url": "http://example.com/e.tar.zst",
                        "student_file_paths": ["src/main.py"],
                        "checksum": "editorsum"
                    },
                    "model_solution_spec": { "type": "editor", "solution_download_url": "http://example.com/sol" },
                    "exercise_service_slug": "tmc"
                },
                {
                    "task_id": browser_task_id,
                    "order_number": 1,
                    "assignment": [],
                    "public_spec": {
                        "type": "browser",
                        "archive_name": "stub.tar.zst",
                        "stub_download_url": "http://example.com/b.tar.zst",
                        "student_file_paths": [],
                        "checksum": "browsersum",
                        "browser_test": { "runtime": "python", "script": "print(1)" }
                    },
                    "model_solution_spec": null,
                    "exercise_service_slug": "tmc"
                }
            ]
        });

        let api_slide: api::ExerciseSlide = serde_json::from_value(slide_json).unwrap();
        let slide: TmcExerciseSlide = api_slide.try_into().unwrap();
        assert_eq!(slide.tasks.len(), 2);
        assert_eq!(slide.tasks[0].task_id, editor_task_id);
        // checksum is derived from the public spec
        assert_eq!(slide.tasks[0].checksum.as_deref(), Some("editorsum"));
        assert_eq!(
            slide.tasks[0].public_spec.as_ref().unwrap().exercise_type(),
            &ExerciseType::Editor
        );
        assert_eq!(
            slide.editor_stub_download_url(),
            Some("http://example.com/e.tar.zst")
        );
        assert_eq!(
            slide.tasks[0]
                .model_solution_spec
                .as_ref()
                .map(|spec| spec.solution_download_url()),
            Some("http://example.com/sol")
        );

        let serialized = serde_json::to_string(&slide).unwrap();
        let reparsed: TmcExerciseSlide = serde_json::from_str(&serialized).unwrap();
        assert_eq!(reparsed.tasks.len(), 2);
        assert_eq!(reparsed.tasks[1].task_id, browser_task_id);
        assert_eq!(
            reparsed.tasks[1]
                .public_spec
                .as_ref()
                .unwrap()
                .exercise_type(),
            &ExerciseType::Browser
        );
    }
}
