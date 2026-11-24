use crate::MoocClientError;
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
    /// Lets a client locate the exercise's course without a separate lookup. The nil UUID for an
    /// exam exercise, which `exam_id` names instead.
    pub course_id: Uuid,
    /// `None` for a course exercise.
    #[serde(default)]
    pub exam_id: Option<Uuid>,
    pub exercise_name: String,
    pub exercise_order_number: i32,
    /// The effective deadline: the exercise's own, else its chapter's. Submissions are refused
    /// from this moment on.
    pub deadline: Option<DateTime<Utc>>,
    pub tasks: Vec<TmcExerciseTask>,
    /// The course material page the exercise is on, which also shows the student's latest
    /// submission. `None` from a host that does not report it.
    pub page_url: Option<String>,
    /// `None` for an exercise outside any chapter, or from a host that does not report it.
    pub chapter: Option<ExerciseChapter>,
    /// Why the exercise cannot be downloaded to work on in an editor: `browser-only` or
    /// `several-editor-tasks`, the failure `download-or-update-course-exercises` would report.
    /// Absent when it can be downloaded, and from CLIs older than this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts-rs", ts(optional))]
    pub unsupported: Option<MoocDownloadFailure>,
}

/// The chapter an exercise belongs to, for grouping a course's exercises the way its material
/// does.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct ExerciseChapter {
    pub id: Uuid,
    pub name: String,
    /// The chapter's position in the course, from 1.
    pub chapter_number: i32,
    /// The user's status in the course's chapter locking; submissions are refused unless it is
    /// `Unlocked`. `None` when the course does not lock chapters, or the host does not report it.
    #[serde(default)]
    pub locking_status: Option<ChapterLockingStatus>,
}

impl From<api::ExerciseChapter> for ExerciseChapter {
    fn from(value: api::ExerciseChapter) -> Self {
        Self {
            id: value.id,
            name: value.name,
            chapter_number: value.chapter_number,
            locking_status: value
                .locking_status
                .and_then(ChapterLockingStatus::from_api),
        }
    }
}

/// Where the user is in a course's chapter locking, which opens chapters one at a time as the
/// previous ones are completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub enum ChapterLockingStatus {
    /// The chapter's exercises can be submitted to.
    Unlocked,
    /// The user has completed the chapter and locked it; its exercises take no more submissions.
    CompletedAndLocked,
    /// The previous chapters must be completed first.
    NotUnlockedYet,
}

impl ChapterLockingStatus {
    /// `None` for a status newer than this client.
    fn from_api(value: api::ChapterLockingStatus) -> Option<Self> {
        match value {
            api::ChapterLockingStatus::Unlocked => Some(Self::Unlocked),
            api::ChapterLockingStatus::CompletedAndLocked => Some(Self::CompletedAndLocked),
            api::ChapterLockingStatus::NotUnlockedYet => Some(Self::NotUnlockedYet),
            api::ChapterLockingStatus::Unrecognized => None,
        }
    }
}

impl TmcExerciseSlide {
    /// The task id and stub archive URL an editor downloads the exercise with, or why it cannot
    /// (`SeveralEditorTasks` or `BrowserOnly`), which [`Self::unsupported`] reports.
    pub fn editor_download(&self) -> Result<(Uuid, &str), MoocDownloadFailure> {
        if self.has_several_editor_tasks() {
            return Err(MoocDownloadFailure::SeveralEditorTasks);
        }
        self.editor_task_id()
            .zip(self.editor_stub_download_url())
            .ok_or(MoocDownloadFailure::BrowserOnly)
    }

    /// [`Self::editor_download`], failing with the client error that names this exercise.
    pub fn require_editor_download(&self) -> Result<(Uuid, &str), Box<MoocClientError>> {
        let exercise_id = self.exercise_id;
        self.editor_download().map_err(|failure| {
            Box::new(match failure {
                MoocDownloadFailure::SeveralEditorTasks => {
                    MoocClientError::SeveralEditorTasks { exercise_id }
                }
                _ => MoocClientError::NoDownloadableExerciseTask { exercise_id },
            })
        })
    }

    /// Stub archive download URL for this slide's first editor task, if any.
    /// `None` for browser exercises, which expose no downloadable archive.
    pub fn editor_stub_download_url(&self) -> Option<&str> {
        self.editor_task()
            .and_then(|task| task.public_spec.as_ref())
            .and_then(PublicSpec::editor_stub_download_url)
    }

    /// Task id of this slide's first editor task (the submit target), if any.
    pub fn editor_task_id(&self) -> Option<Uuid> {
        self.editor_task().map(|task| task.task_id)
    }

    /// Whether the slide has more than one editor task. A submission answers one task, so such
    /// a slide can only be answered in the course material.
    pub fn has_several_editor_tasks(&self) -> bool {
        self.editor_tasks().nth(1).is_some()
    }

    /// Checksum of this slide's first editor task, if any; compared with the
    /// stored one to detect an out-of-date local exercise.
    pub fn editor_checksum(&self) -> Option<&str> {
        self.editor_task().and_then(|task| task.checksum.as_deref())
    }

    /// This slide's first editor task, the one a native client works on. Browser tasks do not count.
    fn editor_task(&self) -> Option<&TmcExerciseTask> {
        self.editor_tasks().next()
    }

    fn editor_tasks(&self) -> impl Iterator<Item = &TmcExerciseTask> {
        self.tasks
            .iter()
            .filter(|task| task.public_spec.as_ref().is_some_and(PublicSpec::is_editor))
    }
}

impl TryFrom<api::ExerciseSlide> for TmcExerciseSlide {
    type Error = JsonError;
    fn try_from(value: api::ExerciseSlide) -> Result<Self, Self::Error> {
        let mut slide = Self {
            slide_id: value.slide_id,
            exercise_id: value.exercise_id,
            course_id: value.course_id,
            exam_id: value.exam_id,
            exercise_name: value.exercise_name,
            exercise_order_number: value.exercise_order_number,
            deadline: value.deadline,
            tasks: value
                .tasks
                .into_iter()
                .map(TryFrom::try_from)
                .collect::<Result<_, _>>()?,
            page_url: value.page_url,
            chapter: value.chapter.map(Into::into),
            unsupported: None,
        };
        slide.unsupported = slide.editor_download().err();
        Ok(slide)
    }
}

/// Why a mooc exercise could not be downloaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub enum MoocDownloadFailure {
    /// Not among the exercises of the course, the exam or any enrolled course.
    NotFound,
    /// Its chapter has not opened yet.
    ClosedChapter,
    /// It is answered in the browser and has no project to download.
    BrowserOnly,
    /// Its slide has more than one editor task; it is answered in the course material.
    SeveralEditorTasks,
    /// courses.mooc.fi's file store has no files for it.
    FileMissing,
    ServerError,
    ConnectionError,
    /// The download could not be unpacked or written into the exercise folder.
    ExtractFailed,
    Other,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct TmcExerciseTask {
    pub task_id: Uuid,
    pub order_number: i32,
    pub assignment: serde_json::Value,
    /// The plain text of `assignment`, one entry per block that has text, nested blocks after
    /// their parent. Absent when it has none, and from CLIs older than this field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts-rs", ts(as = "Option<Vec<String>>", optional))]
    pub instructions_text: Vec<String>,
    pub public_spec: Option<PublicSpec>,
    pub model_solution_spec: Option<ModelSolutionSpec>,
    pub checksum: Option<String>,
    /// The exercise service that owns the task. Specs are read only for [`TMC_SERVICE_SLUG`];
    /// any other service's task has neither spec.
    pub exercise_service_slug: String,
}

/// The exercise service whose tasks this client can work on.
pub const TMC_SERVICE_SLUG: &str = "tmc";

impl TryFrom<api::ExerciseTask> for TmcExerciseTask {
    type Error = JsonError;
    fn try_from(value: api::ExerciseTask) -> Result<Self, Self::Error> {
        // Another service's specs have another shape; failing on them would fail the whole course.
        let is_tmc_task = value.exercise_service_slug == TMC_SERVICE_SLUG;
        let public_spec: Option<PublicSpec> = value
            .public_spec
            .filter(|_| is_tmc_task)
            .map(deserialize::json_from_value)
            .transpose()?;
        let task = Self {
            task_id: value.task_id,
            order_number: value.order_number,
            instructions_text: block_texts(&value.assignment),
            assignment: value.assignment,
            checksum: public_spec.as_ref().map(|ps| ps.checksum.clone()),
            public_spec,
            model_solution_spec: value
                .model_solution_spec
                .filter(|_| is_tmc_task)
                .map(deserialize::json_from_value)
                .transpose()?,
            exercise_service_slug: value.exercise_service_slug,
        };
        Ok(task)
    }
}

/// The text of Gutenberg blocks, as courses.mooc.fi stores an assignment: each block's
/// `attributes.content` HTML as plain text, then its `innerBlocks`'.
fn block_texts(blocks: &serde_json::Value) -> Vec<String> {
    let Some(blocks) = blocks.as_array() else {
        return vec![];
    };
    let mut texts = vec![];
    for block in blocks {
        let text = block
            .pointer("/attributes/content")
            .and_then(serde_json::Value::as_str)
            .map(html_to_text)
            .unwrap_or_default();
        if !text.is_empty() {
            texts.push(text);
        }
        if let Some(inner_blocks) = block.get("innerBlocks") {
            texts.extend(block_texts(inner_blocks));
        }
    }
    texts
}

/// Inline HTML as plain text: `<br>` becomes a line break, other tags are dropped and character
/// references decoded.
fn html_to_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find(['<', '&']) {
        text.push_str(&rest[..start]);
        rest = &rest[start..];
        if rest.starts_with('<') {
            let Some(end) = rest.find('>') else {
                text.push_str(rest);
                rest = "";
                break;
            };
            let tag = rest[1..end].trim().to_ascii_lowercase();
            if tag == "br" || tag.starts_with("br ") || tag.starts_with("br/") {
                text.push('\n');
            }
            rest = &rest[end + 1..];
        } else {
            let decoded = rest
                .find(';')
                .filter(|&end| end <= 10)
                .and_then(|end| Some((decode_character_reference(&rest[1..end])?, end)));
            match decoded {
                Some((c, end)) => {
                    text.push(c);
                    rest = &rest[end + 1..];
                }
                None => {
                    text.push('&');
                    rest = &rest[1..];
                }
            }
        }
    }
    text.push_str(rest);
    text.trim().to_string()
}

/// The character `&name;` stands for, given `name`.
fn decode_character_reference(name: &str) -> Option<char> {
    if let Some(number) = name.strip_prefix('#') {
        let code = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse().ok()?,
        };
        return char::from_u32(code);
    }
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => None,
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

    pub fn is_editor(&self) -> bool {
        self.exercise_type == ExerciseType::Editor
    }

    /// Stub archive download URL; `None` for browser exercises, which have no project archive.
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

/// In-browser test spec from the `tmc` exercise service: the script to
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

    /// The exact bytes `services/tmc`'s model-solution endpoint emits (pinned there
    /// by `modelSolution.test.ts`). A mismatch breaks `mooc exercise`,
    /// `download-exercise` and `submit` for any exercise the student has solved.
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

    /// Guards the deserialize side, which the serialize-only bindings drift gate
    /// can't catch: parses a mixed editor+browser slide, then round-trips it.
    ///
    /// The conversion has a hand-written mirror in
    /// `tmc-vscode/src/test/moocMockReconciliation.test.ts` (`toCliStdoutSlide`),
    /// which is what reconciles the backend's wire shape with the CLI's stdout
    /// schema. Neither repo's CI can see the other, so a change to the `TryFrom`
    /// impls above has to land there in the same change, or that file goes on
    /// reconciling a conversion that no longer exists.
    #[test]
    fn another_services_task_does_not_fail_the_slide() {
        let slide: api::ExerciseSlide = serde_json::from_value(serde_json::json!({
            "slide_id": Uuid::new_v4(),
            "exercise_id": Uuid::new_v4(),
            "course_id": Uuid::nil(),
            "exam_id": Uuid::new_v4(),
            "exercise_name": "other",
            "exercise_order_number": 0,
            "deadline": null,
            "tasks": [{
                "task_id": Uuid::new_v4(),
                "order_number": 0,
                "assignment": [],
                "public_spec": { "some": "other shape" },
                "model_solution_spec": [1, 2],
                "exercise_service_slug": "quizzes"
            }]
        }))
        .unwrap();
        let slide: TmcExerciseSlide = slide.try_into().unwrap();
        assert!(slide.exam_id.is_some());
        assert_eq!(slide.tasks[0].exercise_service_slug, "quizzes");
        assert!(slide.tasks[0].public_spec.is_none());
        assert!(slide.tasks[0].model_solution_spec.is_none());
        assert_eq!(slide.editor_task_id(), None);
        assert_eq!(slide.unsupported, Some(MoocDownloadFailure::BrowserOnly));
    }

    #[test]
    fn instructions_text_is_the_plain_text_of_the_assignment_blocks() {
        let assignment = serde_json::json!([
            {
                "name": "core/paragraph",
                "attributes": { "content": "Write a <b>loop</b> &amp; print &lt;3 &#39;x&#x27;." },
                "innerBlocks": []
            },
            {
                "name": "core/list",
                "attributes": {},
                "innerBlocks": [
                    { "name": "core/list-item", "attributes": { "content": "First<br>Second" } },
                    { "name": "core/list-item", "attributes": { "content": "  " } }
                ]
            },
            { "name": "core/paragraph", "attributes": { "content": "a & b <unclosed" } }
        ]);
        assert_eq!(
            block_texts(&assignment),
            [
                "Write a loop & print <3 'x'.",
                "First\nSecond",
                "a & b <unclosed"
            ]
        );
        assert!(block_texts(&serde_json::Value::Null).is_empty());
    }

    #[test]
    fn a_slide_with_two_editor_tasks_is_flagged() {
        let editor_task = || {
            serde_json::json!({
                "task_id": Uuid::new_v4(),
                "order_number": 0,
                "assignment": [],
                "public_spec": {
                    "type": "editor",
                    "archive_name": "stub.tar.zst",
                    "stub_download_url": "http://example.com/e.tar.zst",
                    "student_file_paths": [],
                    "checksum": "sum"
                },
                "model_solution_spec": null,
                "exercise_service_slug": "tmc"
            })
        };
        let slide: api::ExerciseSlide = serde_json::from_value(serde_json::json!({
            "slide_id": Uuid::new_v4(),
            "exercise_id": Uuid::new_v4(),
            "course_id": Uuid::new_v4(),
            "exercise_name": "two",
            "exercise_order_number": 0,
            "deadline": null,
            "tasks": [editor_task(), editor_task()]
        }))
        .unwrap();
        let slide: TmcExerciseSlide = slide.try_into().unwrap();
        assert!(slide.has_several_editor_tasks());
        assert_eq!(
            slide.unsupported,
            Some(MoocDownloadFailure::SeveralEditorTasks)
        );
    }

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
        assert_eq!(slide.unsupported, None);
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
