#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Used to communicate with the Courses MOOC server. See the `MoocClient` struct for more details.

mod auth;
mod error;
mod exercise;

pub use self::{
    auth::{
        DEFAULT_CLIENT_ID, DEFAULT_POLL_INTERVAL_SECS, DEVICE_GRANT_TYPE, DEVICE_SCOPE,
        DeviceAuthorizationResponse, DeviceTokenPoll, device_authorization, poll_device_token,
        refresh_token,
    },
    error::{MoocClientError, MoocClientResult},
    exercise::{
        ExerciseFile, ExerciseType, ModelSolutionSpec, PublicSpec, TmcExerciseSlide,
        TmcExerciseTask,
    },
};
use bytes::Bytes;
use chrono::{DateTime, Utc};
pub use exercise_services_api as api;
use oauth2::TokenResponse;
use reqwest::{
    Method, StatusCode,
    blocking::{
        Client, RequestBuilder, Response,
        multipart::{Form, Part},
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{path::Path, sync::Arc};
use tmc_langs_util::{JsonError, serialize};
#[cfg(feature = "ts-rs")]
use ts_rs::TS;
use url::Url;
use uuid::Uuid;

/// Header advertising this client's version to the backend, which may reject
/// obsolete clients with `426 Upgrade Required`.
const CLIENT_VERSION_HEADER: &str = "X-Client-Version";

/// Env knob (`=1`) that also trusts `localhost`/`127.0.0.1` as bearer-token
/// destinations. Off by default so the token is never silently sent to a local
/// host in production; tests and local development set it to exercise the
/// authenticated request path against a mock or a locally-served backend. Gating
/// it on an explicit opt-in keeps production behavior byte-identical.
const TRUST_LOCALHOST_VAR: &str = "TMC_LANGS_MOOC_TRUST_LOCALHOST";

fn trust_localhost() -> bool {
    std::env::var(TRUST_LOCALHOST_VAR).as_deref() == Ok("1")
}

/// Client for accessing the Courses MOOC API.
/// Uses an `Arc` internally so it is cheap to clone.
#[derive(Clone)]
pub struct MoocClient(Arc<MoocClientInner>);

struct MoocClientInner {
    client: Client,
    root_url: Url,
    token: Option<api::Token>,
}

/// Non-API methods.
impl MoocClient {
    /// Creates a new client.
    pub fn new(root_url: Url) -> Self {
        // guarantee a trailing slash, otherwise join will drop the last component
        let root_url = if root_url.as_str().ends_with('/') {
            root_url
        } else {
            format!("{root_url}/").parse().expect("invalid root url")
        };

        Self(Arc::new(MoocClientInner {
            client: Client::new(),
            root_url,
            token: None,
        }))
    }

    fn request(&self, method: Method, url: Url) -> MoocRequest {
        log::debug!("building a request to {url}");

        // The bearer token is only attached to hosts we trust, so it is never
        // leaked to an arbitrary host a (possibly attacker-controlled) URL points
        // at. `host_str` rather than `domain` so IP literals (e.g. `127.0.0.1`)
        // are considered too; for real domains the two agree, so production
        // behavior is unchanged.
        let trusted_domains = &["courses.mooc.fi", "project-331.local"];
        let is_trusted_domain = url
            .host_str()
            .map(|h| {
                trusted_domains.contains(&h)
                    || (trust_localhost() && (h == "localhost" || h == "127.0.0.1"))
            })
            .unwrap_or_default();
        let mut builder = self
            .0
            .client
            .request(method.clone(), url.clone())
            // The crate version is the langs version this client ships as,
            // mirroring the TMC client's `client_version`.
            .header(CLIENT_VERSION_HEADER, env!("CARGO_PKG_VERSION"));
        if let Some(token) = self.0.token.as_ref() {
            if is_trusted_domain {
                log::debug!("setting bearer token");
                builder = builder.bearer_auth(token.access_token().secret());
            } else {
                log::debug!("leaving out bearer token due to untrusted domain");
            }
        } else {
            log::debug!("no bearer token");
        }
        MoocRequest {
            url,
            method,
            builder,
        }
    }

    pub fn set_token(&mut self, token: api::Token) {
        Arc::get_mut(&mut self.0)
            .expect("called when multiple clones exist")
            .token = Some(token);
    }
}

/// API methods.
impl MoocClient {
    pub fn course(&self, course_id: Uuid) -> MoocClientResult<Course> {
        let url = make_client_api_url(self, &format!("courses/{course_id}"))?;
        let res = self
            .request(Method::GET, url)
            .send_expect_json::<api::Course>()?;
        Ok(res.into())
    }

    pub fn courses(&self) -> MoocClientResult<Vec<Course>> {
        let url = make_client_api_url(self, "courses")?;
        let res = self
            .request(Method::GET, url)
            .send_expect_json::<Vec<api::Course>>()?;
        Ok(res.into_iter().map(Into::into).collect())
    }

    pub fn course_exercises(&self, course: Uuid) -> MoocClientResult<Vec<TmcExerciseSlide>> {
        let url = make_client_api_url(self, format!("courses/{course}/exercises"))?;
        let res = self
            .request(Method::GET, url.clone())
            .send_expect_json::<Vec<api::ExerciseSlide>>()?
            .into_iter()
            .map(TryFrom::try_from)
            .collect::<Result<_, JsonError>>()
            .map_err(|err| MoocClientError::DeserializingResponse {
                url,
                error: err.into(),
            })?;
        Ok(res)
    }

    pub fn exercise(&self, exercise: Uuid) -> MoocClientResult<TmcExerciseSlide> {
        let url = make_client_api_url(self, format!("exercises/{exercise}"))?;
        let res = self
            .request(Method::GET, url.clone())
            .send_expect_json::<api::ExerciseSlide>()?
            .try_into()
            .map_err(|err: JsonError| MoocClientError::DeserializingResponse {
                url,
                error: err.into(),
            })?;
        Ok(res)
    }

    pub fn download(&self, url: Url) -> MoocClientResult<Bytes> {
        let res = self.request(Method::GET, url).send_expect_bytes()?;
        Ok(res)
    }

    /// Downloads the stub archive for an exercise.
    ///
    /// There is no `exercises/{id}/download` route; the archive lives at the
    /// editor task public spec's `stub_download_url`. Returns
    /// [`MoocClientError::NoDownloadableExerciseTask`] for a browser-only exercise
    /// with no editor task.
    pub fn download_exercise(&self, exercise: Uuid) -> MoocClientResult<Bytes> {
        let slide = self.exercise(exercise)?;
        let download_url = slide.editor_stub_download_url().ok_or_else(|| {
            Box::new(MoocClientError::NoDownloadableExerciseTask {
                exercise_id: exercise,
            })
        })?;
        let url = Url::parse(download_url)
            .map_err(|err| MoocClientError::UrlParse(download_url.to_string(), err))
            .map_err(Box::new)?;
        self.download(url)
    }

    /// Submits an archive for an exercise.
    ///
    /// A native client stores only the exercise id, so this fetches the slide to
    /// resolve its editor task and submits to it. Returns
    /// [`MoocClientError::NoSubmittableExerciseTask`] for a browser-only exercise.
    pub fn submit_exercise(
        &self,
        exercise_id: Uuid,
        archive: &Path,
    ) -> MoocClientResult<ExerciseTaskSubmissionResult> {
        let slide = self.exercise(exercise_id)?;
        let task_id = slide
            .editor_task_id()
            .ok_or_else(|| Box::new(MoocClientError::NoSubmittableExerciseTask { exercise_id }))?;
        self.submit(exercise_id, slide.slide_id, task_id, archive)
    }

    pub fn submit(
        &self,
        exercise_id: Uuid,
        slide_id: Uuid,
        task_id: Uuid,
        archive: &Path,
    ) -> MoocClientResult<ExerciseTaskSubmissionResult> {
        // The `submission` part is just the two ids now; the backend derives the
        // task's `data_json` (the tmc editor answer) itself from the uploaded file.
        let exercise_slide_submission = api::ExerciseSlideSubmission {
            exercise_slide_id: slide_id,
            exercise_task_id: task_id,
        };
        let exercise_slide_submission = serialize::to_json_vec(&exercise_slide_submission)
            .map_err(Into::into)
            .map_err(Box::new)?;
        let submission = Form::new()
            .part(
                // Backend `SubmissionForm` names this JSON part `submission`
                // (headless-lms exercise_services/client.rs).
                "submission",
                Part::bytes(exercise_slide_submission)
                    .mime_str("application/json")
                    .expect("known to work"),
            )
            .file("file", archive)
            .map_err(|err| MoocClientError::AttachFileToForm { error: err.into() })?;

        let url = make_client_api_url(self, format!("exercises/{exercise_id}/submit"))?;
        let res = self
            .request(Method::POST, url)
            .multipart(submission)
            .send_expect_json::<api::ExerciseTaskSubmissionResult>()?;
        Ok(res.into())
    }

    pub fn get_submission_grading(
        &self,
        submission_id: Uuid,
    ) -> MoocClientResult<ExerciseTaskSubmissionStatus> {
        let url = make_client_api_url(self, format!("submissions/{submission_id}/grading"))?;
        let res = self
            .request(Method::GET, url)
            .send_expect_json::<api::ExerciseTaskSubmissionStatus>()?;
        Ok(res.into())
    }

    /// Fetches each exercise by its exercise id, one slide per id. Each slide
    /// carries `exercise_id` so callers can correlate results back to requests.
    pub fn get_exercises(&self, exercise_ids: &[Uuid]) -> MoocClientResult<Vec<TmcExerciseSlide>> {
        // todo: implement in a single request...
        let mut slides = Vec::new();
        for exercise_id in exercise_ids {
            slides.push(self.exercise(*exercise_id)?);
        }
        Ok(slides)
    }

    /// Returns the current user's past submissions to an exercise, newest first.
    /// Each item's `id` is an exercise-slide-submission id, the value passed to
    /// [`MoocClient::download_submission_archive_url`] and
    /// [`MoocClient::share_submission`].
    pub fn get_exercise_submissions(
        &self,
        exercise_id: Uuid,
    ) -> MoocClientResult<Vec<ExerciseSlideSubmissionListItem>> {
        let url = make_client_api_url(self, format!("exercises/{exercise_id}/submissions"))?;
        let res = self
            .request(Method::GET, url)
            .send_expect_json::<Vec<api::ExerciseSlideSubmissionListItem>>()?;
        Ok(res.into_iter().map(Into::into).collect())
    }

    /// Resolves an exercise-slide-submission id (from
    /// [`MoocClient::get_exercise_submissions`]) to the file-store URL of the
    /// archive that was submitted, so an old submission can be re-downloaded.
    pub fn download_submission_archive_url(&self, submission_id: Uuid) -> MoocClientResult<String> {
        let url = make_client_api_url(self, format!("submissions/{submission_id}/download"))?;
        let res = self
            .request(Method::GET, url)
            .send_expect_json::<api::SubmissionArchiveDownloadUrl>()?;
        Ok(res.archive_download_url)
    }

    /// Mints a shareable link to an existing submission of the current user and
    /// returns the paste URL.
    pub fn share_submission(&self, submission_id: Uuid) -> MoocClientResult<PasteResult> {
        let url = make_client_api_url(self, format!("submissions/{submission_id}/share"))?;
        let res = self
            .request(Method::POST, url)
            .send_expect_json::<api::PasteResult>()?;
        Ok(res.into())
    }
}

/// Helper for creating and sending requests.
struct MoocRequest {
    url: Url,
    method: Method,
    builder: RequestBuilder,
}

impl MoocRequest {
    fn json<T: Serialize>(mut self, json: &T) -> Self {
        self.builder = self.builder.json(json);
        self
    }

    fn multipart(mut self, form: Form) -> Self {
        self.builder = self.builder.multipart(form);
        self
    }

    fn send(self) -> MoocClientResult<Response> {
        match self.builder.send() {
            Ok(res) => {
                let status = res.status();
                match status {
                    _success if status.is_success() => Ok(res),
                    StatusCode::UNAUTHORIZED => Err(Box::new(MoocClientError::NotAuthenticated)),
                    _other => {
                        let status = res.status();
                        // 426 Upgrade Required signals the client is too old; the
                        // backend enforces a minimum `X-Client-Version`.
                        let obsolete_client = status == StatusCode::UPGRADE_REQUIRED;
                        let body =
                            res.text()
                                .map_err(|err| MoocClientError::ReadingResponseBody {
                                    method: self.method,
                                    url: self.url.clone(),
                                    error: Box::new(err),
                                })?;
                        // The backend returns controlled errors as an
                        // `ApiErrorResponse` carrying a `message_key` (e.g.
                        // `not_enrolled`). Try to lift that key so the CLI can map
                        // it to a typed error kind; keep the raw body for messages.
                        let message_key = serde_json::from_str::<ApiErrorBody>(&body)
                            .ok()
                            .and_then(|parsed| parsed.message_key);
                        Err(Box::new(MoocClientError::HttpError {
                            url: self.url,
                            status,
                            error: body,
                            obsolete_client,
                            message_key,
                        }))
                    }
                }
            }
            Err(error) => Err(Box::new(MoocClientError::ConnectionError(
                self.method,
                self.url,
                error,
            ))),
        }
    }

    fn send_expect_bytes(self) -> MoocClientResult<Bytes> {
        let method = self.method.clone();
        let url = self.url.clone();
        let res = self.send()?;
        let body = res
            .bytes()
            .map_err(|err| MoocClientError::ReadingResponseBody {
                method,
                url,
                error: Box::new(err),
            })?;
        Ok(body)
    }

    fn send_expect_json<T>(self) -> MoocClientResult<T>
    where
        T: DeserializeOwned,
    {
        let url = self.url.clone();
        let bytes = self.send_expect_bytes()?;
        let json = serde_json::from_slice(&bytes).map_err(|err| {
            MoocClientError::DeserializingResponse {
                url,
                error: Box::new(err),
            }
        })?;
        Ok(json)
    }
}

/// The single field of the backend's `ApiErrorResponse` the client needs: the
/// stable `message_key` identifying a controlled error. Deserialized leniently
/// (missing/null key = `None`) so any non-conforming error body simply yields no
/// key rather than failing.
#[derive(Deserialize)]
struct ApiErrorBody {
    #[serde(default)]
    message_key: Option<String>,
}

// joins the URL "tail" with the API url root from the client
fn make_client_api_url(client: &MoocClient, tail: impl AsRef<str>) -> MoocClientResult<Url> {
    client
        .0
        .root_url
        .join("/api/v0/exercise-services/client/")
        .and_then(|u| u.join(tail.as_ref()))
        .map_err(|e| MoocClientError::UrlParse(tail.as_ref().to_string(), e))
        .map_err(Box::new)
}

#[derive(Debug, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
// Renamed to avoid colliding with the TMC `Course` type: a duplicate
// `export type Course` would make `bindings.d.ts` uncompilable, and schemars
// would auto-disambiguate the `$defs` key to `Course2`.
#[cfg_attr(feature = "ts-rs", ts(rename = "MoocCourse"))]
#[schemars(rename = "MoocCourse")]
pub struct Course {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub organization_name: String,
}

impl From<api::Course> for Course {
    fn from(value: api::Course) -> Self {
        Self {
            id: value.id,
            slug: value.slug,
            name: value.name,
            description: value.description,
            organization_name: value.organization_name,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct CourseInfo {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct ExerciseTaskSubmissionResult {
    pub submission_id: Uuid,
}

impl From<api::ExerciseTaskSubmissionResult> for ExerciseTaskSubmissionResult {
    fn from(value: api::ExerciseTaskSubmissionResult) -> Self {
        Self {
            submission_id: value.submission_id,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub enum ExerciseTaskSubmissionStatus {
    NoGradingYet,
    Grading {
        grading_progress: GradingProgress,
        score_given: Option<f32>,
        grading_started_at: Option<DateTime<Utc>>,
        grading_completed_at: Option<DateTime<Utc>>,
        feedback_json: Option<serde_json::Value>,
        feedback_text: Option<String>,
    },
}

impl From<api::ExerciseTaskSubmissionStatus> for ExerciseTaskSubmissionStatus {
    fn from(value: api::ExerciseTaskSubmissionStatus) -> Self {
        match value {
            api::ExerciseTaskSubmissionStatus::NoGradingYet => Self::NoGradingYet,
            api::ExerciseTaskSubmissionStatus::Grading {
                grading_progress,
                score_given,
                grading_started_at,
                grading_completed_at,
                feedback_json,
                feedback_text,
            } => Self::Grading {
                grading_progress: grading_progress.into(),
                score_given,
                grading_started_at,
                grading_completed_at,
                feedback_json,
                feedback_text,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub enum GradingProgress {
    /// The grading could not complete.
    Failed,
    /// There is no grading process occurring; for example, the student has not yet made any submission.
    NotReady,
    /// Final Grade is pending, and it does require human intervention; if a Score value is present, it indicates the current value is partial and may be updated during the manual grading.
    PendingManual,
    /// Final Grade is pending, but does not require manual intervention; if a Score value is present, it indicates the current value is partial and may be updated.
    Pending,
    /// The grading process is completed; the score value, if any, represents the current Final Grade;
    FullyGraded,
}

impl From<api::GradingProgress> for GradingProgress {
    fn from(value: api::GradingProgress) -> Self {
        match value {
            api::GradingProgress::Failed => Self::Failed,
            api::GradingProgress::NotReady => Self::NotReady,
            api::GradingProgress::PendingManual => Self::PendingManual,
            api::GradingProgress::Pending => Self::Pending,
            api::GradingProgress::FullyGraded => Self::FullyGraded,
        }
    }
}

/// A single past submission of the current user to an exercise. `id` is the
/// exercise-slide-submission id used to download or share the submission.
#[derive(Debug, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct ExerciseSlideSubmissionListItem {
    pub id: Uuid,
    pub exercise_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub score_given: Option<f32>,
    pub grading_progress: Option<GradingProgress>,
}

impl From<api::ExerciseSlideSubmissionListItem> for ExerciseSlideSubmissionListItem {
    fn from(value: api::ExerciseSlideSubmissionListItem) -> Self {
        Self {
            id: value.id,
            exercise_id: value.exercise_id,
            created_at: value.created_at,
            score_given: value.score_given,
            grading_progress: value.grading_progress.map(Into::into),
        }
    }
}

/// A shareable URL for a submission.
#[derive(Debug, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct PasteResult {
    pub paste_url: String,
}

impl From<api::PasteResult> for PasteResult {
    fn from(value: api::PasteResult) -> Self {
        Self {
            paste_url: value.paste_url,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use exercise_services_api::Token;
    use mockito::{Matcher, Server};
    use oauth2::{AccessToken, EmptyExtraTokenFields, basic::BasicTokenType};
    use std::sync::{Mutex, MutexGuard};

    // `TMC_LANGS_MOOC_TRUST_LOCALHOST` is process-wide, so the test that toggles
    // it holds this lock to keep concurrent tests from reading it mid-flight.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_lock() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn init() {
        use log::*;
        use simple_logger::*;

        let _ = SimpleLogger::new()
            .with_level(LevelFilter::Debug)
            // mockito does some logging
            .with_module_level("mockito", LevelFilter::Warn)
            // reqwest does a lot of logging
            .with_module_level("reqwest", LevelFilter::Warn)
            // hyper does a lot of logging
            .with_module_level("hyper", LevelFilter::Warn)
            .init();
    }

    fn make_client(server: &Server) -> MoocClient {
        let mut client = MoocClient::new(server.url().parse().unwrap());
        let token = Token::new(
            AccessToken::new("".to_string()),
            BasicTokenType::Bearer,
            EmptyExtraTokenFields {},
        );
        client.set_token(token);
        client
    }

    fn make_client_with_token(server: &Server, access_token: &str) -> MoocClient {
        let mut client = MoocClient::new(server.url().parse().unwrap());
        let token = Token::new(
            AccessToken::new(access_token.to_string()),
            BasicTokenType::Bearer,
            EmptyExtraTokenFields {},
        );
        client.set_token(token);
        client
    }

    #[test]
    fn bearer_attached_to_localhost_only_with_trust_knob() {
        // mockito serves on 127.0.0.1, an untrusted host, so by default the bearer
        // token is NOT sent. `TMC_LANGS_MOOC_TRUST_LOCALHOST=1` opts localhost in,
        // which is what lets tests (and the cross-repo auth coverage) exercise the
        // authenticated request path against a local mock.
        init();
        let _env = env_lock();
        // SAFETY: all reads/writes of this var in tests are serialized by ENV_LOCK.
        unsafe { std::env::remove_var(TRUST_LOCALHOST_VAR) };

        let mut server = Server::new();
        let client = make_client_with_token(&server, "test-token");

        // Without the knob: no Authorization header reaches the mock.
        let no_auth = server
            .mock("GET", "/api/v0/exercise-services/client/courses")
            .match_header("authorization", Matcher::Missing)
            .with_body("[]")
            .expect(1)
            .create();
        client.courses().unwrap();
        no_auth.assert();

        // With the knob: localhost is trusted, so the bearer is attached.
        unsafe { std::env::set_var(TRUST_LOCALHOST_VAR, "1") };
        let with_auth = server
            .mock("GET", "/api/v0/exercise-services/client/courses")
            .match_header("authorization", "Bearer test-token")
            .with_body("[]")
            .expect(1)
            .create();
        let res = client.courses();
        // Clear the var before any assertion that could panic and leak it.
        unsafe { std::env::remove_var(TRUST_LOCALHOST_VAR) };
        res.unwrap();
        with_auth.assert();
    }

    #[test]
    fn gets_courses() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        server
            .mock("GET", "/api/v0/exercise-services/client/courses")
            .with_body(
                serde_json::json!([{
                    "id": Uuid::new_v4(),
                    "slug": "mockslug",
                    "name": "mockname",
                    "description": "mockdesc",
                    "organization_name": "mockorg",
                }])
                .to_string(),
            )
            .create();
        let courses = client.courses().unwrap();
        assert_eq!(courses[0].name, "mockname");
    }

    #[test]
    fn gets_course_exercise_slides() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        server
            .mock(
                "GET",
                "/api/v0/exercise-services/client/courses/df5ee6c1-57d1-43b6-b39e-5d72119edb5f/exercises",
            )
            .with_body(
                serde_json::json!([{
                    "slide_id": Uuid::new_v4(),
                    "exercise_id": Uuid::new_v4(),
                    "course_id": Uuid::new_v4(),
                    "exercise_name": "mockname",
                    "exercise_order_number": 0,
                    "tasks": [],
                }])
                .to_string(),
            )
            .create();
        let exercise_slides = client
            .course_exercises(Uuid::parse_str("df5ee6c1-57d1-43b6-b39e-5d72119edb5f").unwrap())
            .unwrap();
        assert_eq!(exercise_slides[0].exercise_name, "mockname");
    }

    #[test]
    fn gets_course_exercise_slides_with_browser_task() {
        // Exercises the typed PublicSpec path that empty-task fixtures never hit.
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let course_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/courses/{course_id}/exercises").as_str(),
            )
            .with_body(
                serde_json::json!([{
                    "slide_id": Uuid::new_v4(),
                    "exercise_id": Uuid::new_v4(),
                    "course_id": Uuid::new_v4(),
                    "exercise_name": "browser exercise",
                    "exercise_order_number": 0,
                    "tasks": [{
                        "task_id": Uuid::new_v4(),
                        "order_number": 0,
                        "assignment": [],
                        "public_spec": {
                            "type": "browser",
                            "archive_name": "stub.tar.zst",
                            "stub_download_url": "http://example.com/stub.tar.zst",
                            "student_file_paths": [],
                            "checksum": "abcd1234",
                            "browser_test": {
                                "runtime": "python",
                                "script": "print('hi')"
                            }
                        },
                        "model_solution_spec": null,
                        "exercise_service_slug": "tmc"
                    }],
                }])
                .to_string(),
            )
            .create();
        let slides = client
            .course_exercises(Uuid::parse_str(course_id).unwrap())
            .unwrap();
        assert_eq!(slides.len(), 1);
        let spec = slides[0].tasks[0].public_spec.as_ref().unwrap();
        assert_eq!(spec.exercise_type(), &ExerciseType::Browser);
        // browser exercises are not downloadable as a local project archive
        assert!(spec.editor_stub_download_url().is_none());
    }

    #[test]
    fn gets_exercise() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        server
            .mock(
                "GET",
                "/api/v0/exercise-services/client/exercises/df5ee6c1-57d1-43b6-b39e-5d72119edb5f",
            )
            .with_body(
                serde_json::json!({
                    "slide_id": Uuid::new_v4(),
                    "exercise_id": Uuid::new_v4(),
                    "course_id": Uuid::new_v4(),
                    "exercise_name": "mockname",
                    "exercise_order_number": 0,
                    "tasks": [],
                })
                .to_string(),
            )
            .create();
        let exercise = client
            .exercise(Uuid::parse_str("df5ee6c1-57d1-43b6-b39e-5d72119edb5f").unwrap())
            .unwrap();
        assert_eq!(exercise.exercise_name, "mockname");
    }

    #[test]
    fn downloads_exercise() {
        // No download route: the archive comes from the editor task's stub_download_url.
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let exercise_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
        let stub_download_url = format!("{}/files/stub.tar.zst", server.url());
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/exercises/{exercise_id}").as_str(),
            )
            .with_body(
                serde_json::json!({
                    "slide_id": Uuid::new_v4(),
                    "exercise_id": Uuid::new_v4(),
                    "course_id": Uuid::new_v4(),
                    "exercise_name": "mockname",
                    "exercise_order_number": 0,
                    "tasks": [{
                        "task_id": Uuid::new_v4(),
                        "order_number": 0,
                        "assignment": [],
                        "public_spec": {
                            "type": "editor",
                            "archive_name": "stub.tar.zst",
                            "stub_download_url": stub_download_url,
                            "student_file_paths": ["src/main.py"],
                            "checksum": "abcd1234"
                        },
                        "model_solution_spec": null,
                        "exercise_service_slug": "tmc"
                    }],
                })
                .to_string(),
            )
            .create();
        server
            .mock("GET", "/files/stub.tar.zst")
            .with_body_from_file("./tests/data/file")
            .create();
        let exercise = client
            .download_exercise(Uuid::parse_str(exercise_id).unwrap())
            .unwrap();
        assert_eq!(String::from_utf8(exercise.into()).unwrap(), "hello!");
    }

    #[test]
    fn download_exercise_errors_on_browser_only_exercise() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let exercise_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/exercises/{exercise_id}").as_str(),
            )
            .with_body(
                serde_json::json!({
                    "slide_id": Uuid::new_v4(),
                    "exercise_id": Uuid::new_v4(),
                    "course_id": Uuid::new_v4(),
                    "exercise_name": "mockname",
                    "exercise_order_number": 0,
                    "tasks": [{
                        "task_id": Uuid::new_v4(),
                        "order_number": 0,
                        "assignment": [],
                        "public_spec": {
                            "type": "browser",
                            "archive_name": "stub.tar.zst",
                            "stub_download_url": "http://example.com/stub.tar.zst",
                            "student_file_paths": [],
                            "checksum": "abcd1234",
                            "browser_test": { "runtime": "python", "script": "" }
                        },
                        "model_solution_spec": null,
                        "exercise_service_slug": "tmc"
                    }],
                })
                .to_string(),
            )
            .create();
        let err = client
            .download_exercise(Uuid::parse_str(exercise_id).unwrap())
            .unwrap_err();
        assert!(matches!(
            *err,
            MoocClientError::NoDownloadableExerciseTask { .. }
        ));
    }

    #[test]
    fn downloads_from_raw_url() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        server
            .mock("GET", "/files/archive.tar.zst")
            .with_body_from_file("./tests/data/file")
            .create();
        let url = format!("{}/files/archive.tar.zst", server.url())
            .parse()
            .unwrap();
        let bytes = client.download(url).unwrap();
        assert_eq!(String::from_utf8(bytes.into()).unwrap(), "hello!");
    }

    #[test]
    fn gets_single_course() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let course_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/courses/{course_id}").as_str(),
            )
            .with_body(
                serde_json::json!({
                    "id": course_id,
                    "slug": "mockslug",
                    "name": "mockname",
                    "description": "mockdesc",
                    "organization_name": "mockorg",
                })
                .to_string(),
            )
            .create();
        let course = client.course(Uuid::parse_str(course_id).unwrap()).unwrap();
        assert_eq!(course.name, "mockname");
        assert_eq!(course.organization_name, "mockorg");
    }

    #[test]
    fn gets_exercises_batched() {
        // Exercises the typed editor PublicSpec path and checksum derivation.
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let exercise_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
        let task_id = "816ac03a-a713-4804-9ea6-3eb5e278ec2b";
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/exercises/{exercise_id}").as_str(),
            )
            .with_body(
                serde_json::json!({
                    "slide_id": Uuid::new_v4(),
                    "exercise_id": exercise_id,
                    "course_id": Uuid::new_v4(),
                    "exercise_name": "mockname",
                    "exercise_order_number": 0,
                    "tasks": [{
                        "task_id": task_id,
                        "order_number": 0,
                        "assignment": [],
                        "public_spec": {
                            "type": "editor",
                            "archive_name": "stub.tar.zst",
                            "stub_download_url": "http://example.com/stub.tar.zst",
                            "student_file_paths": ["src/main.py"],
                            "checksum": "abcd1234"
                        },
                        "model_solution_spec": null,
                        "exercise_service_slug": "tmc"
                    }],
                })
                .to_string(),
            )
            .create();
        let slides = client
            .get_exercises(&[Uuid::parse_str(exercise_id).unwrap()])
            .unwrap();
        assert_eq!(slides.len(), 1);
        assert_eq!(slides[0].exercise_id, Uuid::parse_str(exercise_id).unwrap());
        assert_eq!(slides[0].tasks.len(), 1);
        assert_eq!(
            slides[0].tasks[0].task_id,
            Uuid::parse_str(task_id).unwrap()
        );
        assert_eq!(slides[0].editor_checksum(), Some("abcd1234"));
        let public_spec = slides[0].tasks[0].public_spec.as_ref().unwrap();
        assert_eq!(public_spec.exercise_type(), &ExerciseType::Editor);
        assert_eq!(
            public_spec.stub_download_url(),
            "http://example.com/stub.tar.zst"
        );
    }

    #[test]
    fn submits() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        server
            .mock(
                "POST",
                "/api/v0/exercise-services/client/exercises/df5ee6c1-57d1-43b6-b39e-5d72119edb5f/submit",
            )
            // The JSON part must be named `submission` (backend `SubmissionForm`), not `metadata`.
            .match_body(Matcher::AllOf(vec![
                Matcher::Regex(r#"name="submission""#.to_string()),
                Matcher::Regex(r#"name="file""#.to_string()),
            ]))
            .with_body(
                serde_json::json!({
                    "submission_id": Uuid::new_v4(),
                })
                .to_string(),
            )
            .create();
        let _submission_result = client
            .submit(
                Uuid::parse_str("df5ee6c1-57d1-43b6-b39e-5d72119edb5f").unwrap(),
                Uuid::parse_str("e7bd5a07-1b83-4c97-91f2-e48cccf66b2a").unwrap(),
                Uuid::parse_str("816ac03a-a713-4804-9ea6-3eb5e278ec2b").unwrap(),
                Path::new("./tests/data/file"),
            )
            .unwrap();
    }

    #[test]
    fn submits_by_exercise_id_resolving_slide_and_task() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let exercise_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
        let slide_id = "e7bd5a07-1b83-4c97-91f2-e48cccf66b2a";
        let task_id = "816ac03a-a713-4804-9ea6-3eb5e278ec2b";
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/exercises/{exercise_id}").as_str(),
            )
            .with_body(
                serde_json::json!({
                    "slide_id": slide_id,
                    "exercise_id": exercise_id,
                    "course_id": Uuid::new_v4(),
                    "exercise_name": "mockname",
                    "exercise_order_number": 0,
                    "tasks": [{
                        "task_id": task_id,
                        "order_number": 0,
                        "assignment": [],
                        "public_spec": {
                            "type": "editor",
                            "archive_name": "stub.tar.zst",
                            "stub_download_url": "http://example.com/stub.tar.zst",
                            "student_file_paths": ["src/main.py"],
                            "checksum": "abcd1234"
                        },
                        "model_solution_spec": null,
                        "exercise_service_slug": "tmc"
                    }],
                })
                .to_string(),
            )
            .create();
        server
            .mock(
                "POST",
                format!("/api/v0/exercise-services/client/exercises/{exercise_id}/submit").as_str(),
            )
            .match_body(Matcher::AllOf(vec![
                Matcher::Regex(format!(r#""exercise_slide_id":"{slide_id}""#)),
                Matcher::Regex(format!(r#""exercise_task_id":"{task_id}""#)),
            ]))
            .with_body(serde_json::json!({ "submission_id": Uuid::new_v4() }).to_string())
            .create();
        client
            .submit_exercise(
                Uuid::parse_str(exercise_id).unwrap(),
                Path::new("./tests/data/file"),
            )
            .unwrap();
    }

    #[test]
    fn submit_exercise_errors_on_browser_only_exercise() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let exercise_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/exercises/{exercise_id}").as_str(),
            )
            .with_body(
                serde_json::json!({
                    "slide_id": Uuid::new_v4(),
                    "exercise_id": exercise_id,
                    "course_id": Uuid::new_v4(),
                    "exercise_name": "browser",
                    "exercise_order_number": 0,
                    "tasks": [{
                        "task_id": Uuid::new_v4(),
                        "order_number": 0,
                        "assignment": [],
                        "public_spec": {
                            "type": "browser",
                            "archive_name": "stub.tar.zst",
                            "stub_download_url": "http://example.com/stub.tar.zst",
                            "student_file_paths": [],
                            "checksum": "abcd1234",
                            "browser_test": { "runtime": "python", "script": "" }
                        },
                        "model_solution_spec": null,
                        "exercise_service_slug": "tmc"
                    }],
                })
                .to_string(),
            )
            .create();
        let err = client
            .submit_exercise(
                Uuid::parse_str(exercise_id).unwrap(),
                Path::new("./tests/data/file"),
            )
            .unwrap_err();
        assert!(matches!(
            *err,
            MoocClientError::NoSubmittableExerciseTask { .. }
        ));
    }

    #[test]
    fn gets_submission_grading() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        server
            .mock(
                "GET",
                "/api/v0/exercise-services/client/submissions/df5ee6c1-57d1-43b6-b39e-5d72119edb5f/grading",
            )
            .with_body(serde_json::json!("NoGradingYet").to_string())
            .create();
        server
            .mock(
                "GET",
                "/api/v0/exercise-services/client/submissions/e7bd5a07-1b83-4c97-91f2-e48cccf66b2a/grading",
            )
            .with_body(
                serde_json::json!({
                    "Grading": {
                        "grading_progress": "Failed",
                    }
                })
                .to_string(),
            )
            .create();
        let submission_result = client
            .get_submission_grading(
                Uuid::parse_str("df5ee6c1-57d1-43b6-b39e-5d72119edb5f").unwrap(),
            )
            .unwrap();
        assert!(matches!(
            submission_result,
            ExerciseTaskSubmissionStatus::NoGradingYet
        ));
        let submission_result = client
            .get_submission_grading(
                Uuid::parse_str("e7bd5a07-1b83-4c97-91f2-e48cccf66b2a").unwrap(),
            )
            .unwrap();
        assert!(matches!(
            submission_result,
            ExerciseTaskSubmissionStatus::Grading { .. }
        ));
    }

    #[test]
    fn gets_exercise_submissions() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let exercise_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
        let submission_id = Uuid::new_v4();
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/exercises/{exercise_id}/submissions")
                    .as_str(),
            )
            .with_body(
                serde_json::json!([{
                    "id": submission_id,
                    "exercise_id": exercise_id,
                    "created_at": "2026-07-21T00:00:00Z",
                    "score_given": 1.0,
                    "grading_progress": "FullyGraded"
                }])
                .to_string(),
            )
            .create();
        let submissions = client
            .get_exercise_submissions(Uuid::parse_str(exercise_id).unwrap())
            .unwrap();
        assert_eq!(submissions.len(), 1);
        assert_eq!(submissions[0].id, submission_id);
        assert_eq!(submissions[0].score_given, Some(1.0));
        assert!(matches!(
            submissions[0].grading_progress,
            Some(GradingProgress::FullyGraded)
        ));
    }

    #[test]
    fn downloads_submission_archive_url() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let submission_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/submissions/{submission_id}/download")
                    .as_str(),
            )
            .with_body(
                serde_json::json!({
                    "archive_download_url": "http://example.com/archive.tar.zst"
                })
                .to_string(),
            )
            .create();
        let url = client
            .download_submission_archive_url(Uuid::parse_str(submission_id).unwrap())
            .unwrap();
        assert_eq!(url, "http://example.com/archive.tar.zst");
    }

    #[test]
    fn shares_submission() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let submission_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
        server
            .mock(
                "POST",
                format!("/api/v0/exercise-services/client/submissions/{submission_id}/share")
                    .as_str(),
            )
            .with_body(
                serde_json::json!({
                    "paste_url": "http://example.com/shared-submissions/abc"
                })
                .to_string(),
            )
            .create();
        let result = client
            .share_submission(Uuid::parse_str(submission_id).unwrap())
            .unwrap();
        assert_eq!(
            result.paste_url,
            "http://example.com/shared-submissions/abc"
        );
    }

    #[test]
    fn obsolete_client_maps_426_to_obsolete_flag() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        server
            .mock("GET", "/api/v0/exercise-services/client/courses")
            .with_status(426)
            .with_body("This client is obsolete")
            .create();
        let err = client.courses().unwrap_err();
        match *err {
            MoocClientError::HttpError {
                status,
                obsolete_client,
                ..
            } => {
                assert_eq!(status.as_u16(), 426);
                assert!(obsolete_client);
            }
            other => panic!("expected HttpError, got {other:?}"),
        }
    }

    #[test]
    fn parses_message_key_off_api_error_body() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let exercise_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/exercises/{exercise_id}").as_str(),
            )
            .with_status(422)
            .with_body(
                serde_json::json!({
                    "errors": [],
                    "message": "not enrolled to this course",
                    "message_key": "not_enrolled",
                    "metadata": null,
                    "type": "validation_error",
                })
                .to_string(),
            )
            .create();
        let err = client
            .exercise(Uuid::parse_str(exercise_id).unwrap())
            .unwrap_err();
        match *err {
            MoocClientError::HttpError {
                status,
                message_key,
                error,
                ..
            } => {
                assert_eq!(status.as_u16(), 422);
                assert_eq!(message_key.as_deref(), Some("not_enrolled"));
                assert!(error.contains("not enrolled to this course"));
            }
            other => panic!("expected HttpError, got {other:?}"),
        }
    }

    #[test]
    fn non_conforming_error_body_yields_no_message_key() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        server
            .mock("GET", "/api/v0/exercise-services/client/courses")
            .with_status(500)
            .with_body("internal server error")
            .create();
        let err = client.courses().unwrap_err();
        match *err {
            MoocClientError::HttpError {
                message_key, error, ..
            } => {
                assert_eq!(message_key, None);
                assert_eq!(error, "internal server error");
            }
            other => panic!("expected HttpError, got {other:?}"),
        }
    }

    #[test]
    fn sends_client_version_header() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        server
            .mock("GET", "/api/v0/exercise-services/client/courses")
            .match_header("x-client-version", env!("CARGO_PKG_VERSION"))
            .with_body(serde_json::json!([]).to_string())
            .create();
        let courses = client.courses().unwrap();
        assert!(courses.is_empty());
    }
}
