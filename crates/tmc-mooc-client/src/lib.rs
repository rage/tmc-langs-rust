#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Used to communicate with the Courses MOOC server. See the `MoocClient` struct for more details.

mod auth;
mod error;
mod exercise;

pub use self::{
    auth::{
        AUTH_REQUEST_TIMEOUT, DEFAULT_CLIENT_ID, DEFAULT_POLL_INTERVAL_SECS, DEVICE_GRANT_TYPE,
        DEVICE_SCOPE, DeviceAuthorizationResponse, DeviceTokenPoll, device_authorization,
        poll_device_token, refresh_token,
    },
    error::{MoocClientError, MoocClientResult},
    exercise::{ExerciseType, ModelSolutionSpec, PublicSpec, TmcExerciseSlide, TmcExerciseTask},
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
use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::Duration,
};
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
pub const TRUST_LOCALHOST_VAR: &str = "TMC_LANGS_MOOC_TRUST_LOCALHOST";

/// Public because the same access token is also accepted by tmc-server, so the
/// decision to hand it to a local host has to be made identically on both paths.
pub fn trust_localhost() -> bool {
    std::env::var(TRUST_LOCALHOST_VAR).as_deref() == Ok("1")
}

/// Timeout for ordinary metadata requests, and the connect timeout for every
/// request. Mirrors the 30s bound in `auth.rs`: without it a wedged host or
/// dropped connection hangs the CLI forever instead of surfacing a retryable
/// `ConnectionError`.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Timeout for large-payload transfers (archive download/upload), which can
/// legitimately outlast a metadata round-trip. The connect timeout is still
/// [`REQUEST_TIMEOUT`].
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(60 * 5);

/// Backend `message_key` for a submit naming an upload whose retention window
/// elapsed. Recoverable by uploading again, so [`MoocClient::submit`] matches on
/// it; also mapped to a CLI error kind for the case where the retry fails too.
pub const UPLOAD_EXPIRED_MESSAGE_KEY: &str = "upload_expired";

/// Backend `message_key` for a submit naming an upload that was never made for
/// this exercise by this user. Never a race, so it is never retried.
pub const UNKNOWN_UPLOAD_MESSAGE_KEY: &str = "unknown_upload";

/// File name sent for the submitted project archive. The host stores it as the
/// upload's `name`; nothing depends on the extension.
const SUBMISSION_ARCHIVE_FILE_NAME: &str = "submission.tar.zst";

/// Client for accessing the Courses MOOC API.
/// Uses an `Arc` internally so it is cheap to clone.
#[derive(Clone)]
pub struct MoocClient(Arc<MoocClientInner>);

struct MoocClientInner {
    client: Client,
    root_url: Url,
    // RwLock so `set_token` works on any clone: `MoocClient` is cloned across
    // download threads, so `Arc::get_mut` (unique-ownership only) won't do.
    token: RwLock<Option<api::Token>>,
}

/// Non-API methods.
impl MoocClient {
    /// Creates a new client.
    ///
    /// Fails with [`MoocClientError::InsecureScheme`] if the root URL points at
    /// the production host `courses.mooc.fi` over anything but `https`; the
    /// local-dev host `project-331.local` (and other hosts, e.g. test mocks) may
    /// use `http`.
    pub fn new(root_url: Url) -> MoocClientResult<Self> {
        // guarantee a trailing slash, otherwise join will drop the last component
        let root_url = if root_url.as_str().ends_with('/') {
            root_url
        } else {
            format!("{root_url}/").parse().expect("invalid root url")
        };

        // Bearer token must never go over plaintext to production; mirrors the
        // trusted-domain split used below when attaching the token.
        if root_url.host_str() == Some("courses.mooc.fi") && root_url.scheme() != "https" {
            return Err(Box::new(MoocClientError::InsecureScheme { url: root_url }));
        }

        let client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(REQUEST_TIMEOUT)
            .build()
            .expect("failed to build MoocClient HTTP client");

        Ok(Self(Arc::new(MoocClientInner {
            client,
            root_url,
            token: RwLock::new(None),
        })))
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
        // A poisoned lock still yields the last-written token, which is fine:
        // it's plain data, never left half-updated.
        let token_guard = self
            .0
            .token
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(token) = token_guard.as_ref() {
            if is_trusted_domain {
                log::debug!("setting bearer token");
                builder = builder.bearer_auth(token.access_token().secret());
            } else {
                log::debug!("leaving out bearer token due to untrusted domain");
            }
        } else {
            log::debug!("no bearer token");
        }
        drop(token_guard);
        MoocRequest {
            url,
            method,
            builder,
        }
    }

    /// Sets (or replaces) the bearer token used for authenticated requests.
    ///
    /// Works on any clone via the internal `RwLock`; `&mut self` is kept only
    /// for source compatibility, not because unique ownership is required.
    pub fn set_token(&mut self, token: api::Token) {
        *self
            .0
            .token
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(token);
    }

    /// The access token secret currently set on this client, if any -- e.g. so a caller
    /// that just got rejected knows which token to pass to a refresh call.
    pub fn access_token(&self) -> Option<String> {
        self.0
            .token
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(|token| token.access_token().secret().clone())
    }
}

/// API methods.
impl MoocClient {
    pub fn course(&self, course_id: Uuid) -> MoocClientResult<Course> {
        let url = make_client_api_url(self, format!("courses/{course_id}"))?;
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

    /// Fetches the current user's per-exercise progress for a whole course in a
    /// single round-trip. Returns one entry per exercise the user can see in the
    /// course (open chapters); untouched exercises come back with zeroed
    /// progress. Course-level totals are not sent; the caller derives them by
    /// summing over the entries.
    pub fn course_progress(&self, course: Uuid) -> MoocClientResult<CourseProgress> {
        let url = make_client_api_url(self, format!("courses/{course}/progress"))?;
        let res = self
            .request(Method::GET, url)
            .send_expect_json::<api::CourseProgress>()?;
        Ok(res.into())
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
        // Archive downloads can be large: use the more generous transfer timeout.
        let res = self
            .request(Method::GET, url)
            .transfer_timeout()
            .send_expect_bytes()?;
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

    /// Stores files for an exercise, returning the host's record of each in the
    /// order they were sent.
    ///
    /// Each part is keyed by a fresh client-chosen UUID, as the host's upload
    /// handler requires. That UUID is *not* the file's identity: only
    /// [`api::UploadedFile::id`], the host's own file id, may be named in a
    /// submit.
    pub fn upload_files(
        &self,
        exercise_id: Uuid,
        files: &[(&str, &Path)],
    ) -> MoocClientResult<Vec<api::UploadedFile>> {
        if files.is_empty() {
            // The host rejects an empty multipart body, and there is nothing to record.
            return Ok(Vec::new());
        }

        let mut form = Form::new();
        for (name, path) in files {
            // The host requires a file name on every part.
            let part = Part::file(path)
                .map_err(|err| MoocClientError::AttachFileToForm { error: err.into() })?
                .file_name((*name).to_string());
            form = form.part(Uuid::new_v4().to_string(), part);
        }

        let url = make_client_api_url(self, format!("exercises/{exercise_id}/files"))?;
        let res = self
            .request(Method::POST, url)
            .multipart(form)
            // Uploads can be large: use the more generous transfer timeout.
            .transfer_timeout()
            .send_expect_json::<api::UploadedFiles>()?;
        Ok(res.files)
    }

    /// Submits an archive as the answer to an exercise task: uploads it, then
    /// submits a body naming the stored file.
    ///
    /// The host may reap an upload between the two calls, so an `upload_expired`
    /// submit re-uploads and submits once more. Nothing above this call can
    /// recover from it — the archive is the only input, and it is still on disk
    /// here.
    pub fn submit(
        &self,
        exercise_id: Uuid,
        slide_id: Uuid,
        task_id: Uuid,
        archive: &Path,
    ) -> MoocClientResult<ExerciseTaskSubmissionResult> {
        let files = [(SUBMISSION_ARCHIVE_FILE_NAME, archive)];
        let uploaded = self.upload_files(exercise_id, &files)?;
        match self.submit_uploaded(exercise_id, slide_id, task_id, &uploaded) {
            Err(error) if is_upload_expired(&error) => {
                log::warn!(
                    "an upload for exercise {exercise_id} expired before it could be submitted; \
                     uploading again"
                );
                let uploaded = self.upload_files(exercise_id, &files)?;
                self.submit_uploaded(exercise_id, slide_id, task_id, &uploaded)
            }
            other => other,
        }
    }

    /// Submits a body naming already-stored files. Split out of [`Self::submit`]
    /// so the upload can be retried without re-entering the retry itself.
    fn submit_uploaded(
        &self,
        exercise_id: Uuid,
        slide_id: Uuid,
        task_id: Uuid,
        uploaded: &[api::UploadedFile],
    ) -> MoocClientResult<ExerciseTaskSubmissionResult> {
        let submission = api::ExerciseSlideSubmission {
            exercise_slide_id: slide_id,
            exercise_task_id: task_id,
            uploaded_file_ids: uploaded.iter().map(|file| file.id).collect(),
        };
        let submission = serialize::to_json_vec(&submission)
            .map_err(Into::into)
            .map_err(Box::new)?;

        let url = make_client_api_url(self, format!("exercises/{exercise_id}/submit"))?;
        let res = self
            .request(Method::POST, url)
            .json_body(submission)
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

    /// Returns the files that were uploaded for an exercise-slide submission (an
    /// id from [`MoocClient::get_exercise_submissions`]), in submit order. Empty
    /// for a submission whose answer needed no files.
    pub fn download_submission_files(
        &self,
        submission_id: Uuid,
    ) -> MoocClientResult<Vec<api::UploadedFile>> {
        let url = make_client_api_url(self, format!("submissions/{submission_id}/download"))?;
        let res = self
            .request(Method::GET, url)
            .send_expect_json::<api::SubmissionFiles>()?;
        Ok(res.files)
    }

    /// Resolves an exercise-slide-submission id to the file-store URL of the
    /// single project archive it was made from, so an old submission can be
    /// re-downloaded.
    ///
    /// `Ok(None)` for a submission with no downloadable files: the exercise's
    /// submission list includes browser-iframe answers, which have no uploads at
    /// all, and the wire contract blesses `{"files": []}` for them.
    ///
    /// More than one file means the submission came from elsewhere, and restoring
    /// it would silently produce the wrong project — so that is an error rather
    /// than a guess at which file to take.
    pub fn download_submission_archive_url(
        &self,
        submission_id: Uuid,
    ) -> MoocClientResult<Option<String>> {
        let mut files = self.download_submission_files(submission_id)?;
        match files.len() {
            0 => Ok(None),
            1 => Ok(Some(files.remove(0).download_url)),
            count => Err(Box::new(MoocClientError::UnexpectedSubmissionFileCount {
                submission_id,
                count,
            })),
        }
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
    fn multipart(mut self, form: Form) -> Self {
        self.builder = self.builder.multipart(form);
        self
    }

    /// Sends pre-serialized JSON, so the caller keeps control of the serializer
    /// (and its error type) rather than deferring to reqwest's.
    fn json_body(mut self, body: Vec<u8>) -> Self {
        self.builder = self
            .builder
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body);
        self
    }

    /// Overrides the timeout with the more generous [`TRANSFER_TIMEOUT`], for
    /// large-payload transfers that can outlast a metadata round-trip.
    fn transfer_timeout(mut self) -> Self {
        self.builder = self.builder.timeout(TRANSFER_TIMEOUT);
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
/// (missing/null key = `None`) so a non-conforming error body yields no key
/// rather than failing.
#[derive(Deserialize)]
struct ApiErrorBody {
    #[serde(default)]
    message_key: Option<String>,
}

fn is_upload_expired(error: &MoocClientError) -> bool {
    matches!(
        error,
        MoocClientError::HttpError {
            message_key: Some(key),
            ..
        } if key == UPLOAD_EXPIRED_MESSAGE_KEY
    )
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

/// The current user's progress across every exercise they can see in a course.
/// Course-level totals (awarded/available points, passed count, percentage) are
/// not sent separately; derive them by summing over `exercises`, guarding the
/// percentage against a zero total.
#[derive(Debug, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct CourseProgress {
    pub course_id: Uuid,
    pub exercises: Vec<ExerciseProgress>,
}

impl From<api::CourseProgress> for CourseProgress {
    fn from(value: api::CourseProgress) -> Self {
        Self {
            course_id: value.course_id,
            exercises: value.exercises.into_iter().map(Into::into).collect(),
        }
    }
}

/// The current user's progress on a single exercise. The authoritative "passed"
/// signal is `completed`; `attempted` distinguishes "not started" from "started
/// but not passed".
#[derive(Debug, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct ExerciseProgress {
    pub exercise_id: Uuid,
    /// Points awarded to the user; `0.0` when the user has no state for the
    /// exercise. Can be fractional (partial credit).
    pub score_given: f32,
    /// The maximum points obtainable from the exercise; can be `0`.
    pub score_maximum: i32,
    /// `true` once the exercise reached the `Completed` activity stage.
    pub completed: bool,
    /// `true` once the user has started or submitted the exercise.
    pub attempted: bool,
}

impl From<api::ExerciseProgress> for ExerciseProgress {
    fn from(value: api::ExerciseProgress) -> Self {
        Self {
            exercise_id: value.exercise_id,
            score_given: value.score_given,
            score_maximum: value.score_maximum,
            completed: value.completed,
            attempted: value.attempted,
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
    /// Identifies the task submission; what grading is polled for.
    pub task_submission_id: Uuid,
    /// Identifies the slide submission; what downloading and sharing take.
    pub slide_submission_id: Uuid,
}

impl From<api::ExerciseTaskSubmissionResult> for ExerciseTaskSubmissionResult {
    fn from(value: api::ExerciseTaskSubmissionResult) -> Self {
        Self {
            task_submission_id: value.task_submission_id,
            slide_submission_id: value.slide_submission_id,
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

/// Per-exercise download progress, mirroring TMC's `ClientUpdateData`. Surfaced
/// by the CLI as a `mooc-client-update-data` status update; `id` is a [`Uuid`]
/// since mooc exercises are UUID-keyed.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "client-update-data-kind")]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub enum MoocClientUpdateData {
    ExerciseDownload { id: Uuid, path: PathBuf },
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
        let mut client = MoocClient::new(server.url().parse().unwrap()).unwrap();
        let token = Token::new(
            AccessToken::new("".to_string()),
            BasicTokenType::Bearer,
            EmptyExtraTokenFields {},
        );
        client.set_token(token);
        client
    }

    fn make_client_with_token(server: &Server, access_token: &str) -> MoocClient {
        let mut client = MoocClient::new(server.url().parse().unwrap()).unwrap();
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
    fn gets_course_progress() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let course_id = "5f9e0a1c-3b2d-4e6f-8a9b-0c1d2e3f4a5b";
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/courses/{course_id}/progress").as_str(),
            )
            .with_body(
                serde_json::json!({
                    "course_id": "5f9e0a1c-3b2d-4e6f-8a9b-0c1d2e3f4a5b",
                    "exercises": [
                        {
                            "exercise_id": "a1b2c3d4-0000-4000-8000-000000000001",
                            "score_given": 1.0,
                            "score_maximum": 1,
                            "completed": true,
                            "attempted": true
                        },
                        {
                            "exercise_id": "a1b2c3d4-0000-4000-8000-000000000002",
                            "score_given": 0.5,
                            "score_maximum": 2,
                            "completed": false,
                            "attempted": true
                        },
                        {
                            "exercise_id": "a1b2c3d4-0000-4000-8000-000000000003",
                            "score_given": 0.0,
                            "score_maximum": 3,
                            "completed": false,
                            "attempted": false
                        }
                    ]
                })
                .to_string(),
            )
            .create();
        let progress = client
            .course_progress(Uuid::parse_str(course_id).unwrap())
            .unwrap();
        assert_eq!(progress.course_id, Uuid::parse_str(course_id).unwrap());
        assert_eq!(progress.exercises.len(), 3);
        assert_eq!(progress.exercises[0].score_given, 1.0);
        assert_eq!(progress.exercises[0].score_maximum, 1);
        assert!(progress.exercises[0].completed);
        assert!(progress.exercises[0].attempted);
        // partial credit stays fractional
        assert_eq!(progress.exercises[1].score_given, 0.5);
        assert!(!progress.exercises[1].completed);
        // untouched exercise: zeroed, not attempted
        assert_eq!(progress.exercises[2].score_given, 0.0);
        assert!(!progress.exercises[2].attempted);
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

    const EXERCISE_ID: &str = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
    const SLIDE_ID: &str = "e7bd5a07-1b83-4c97-91f2-e48cccf66b2a";
    const TASK_ID: &str = "816ac03a-a713-4804-9ea6-3eb5e278ec2b";
    /// The host's file id, deliberately unequal to the UUID the client picks as
    /// the multipart field name — a submit naming the field name is a bug.
    const FILE_UPLOAD_ID: &str = "8f0a3c6d-2f1e-4f5b-9c7a-0d1e2f3a4b5c";

    /// Mocks `POST exercises/{id}/files` returning one stored file with
    /// [`FILE_UPLOAD_ID`], asserting the part carries a file name.
    fn mock_upload(server: &mut Server) -> mockito::Mock {
        server
            .mock(
                "POST",
                format!("/api/v0/exercise-services/client/exercises/{EXERCISE_ID}/files").as_str(),
            )
            .match_body(Matcher::Regex(
                r#"filename="submission.tar.zst""#.to_string(),
            ))
            .with_body(
                serde_json::json!({
                    "files": [{
                        "id": FILE_UPLOAD_ID,
                        "name": "submission.tar.zst",
                        "download_url": "http://example.com/archive.tar.zst",
                    }]
                })
                .to_string(),
            )
            .create()
    }

    fn submit_path() -> String {
        format!("/api/v0/exercise-services/client/exercises/{EXERCISE_ID}/submit")
    }

    fn upload_expired_body() -> String {
        serde_json::json!({
            "errors": [],
            "message": "the upload expired",
            "message_key": UPLOAD_EXPIRED_MESSAGE_KEY,
            "metadata": null,
            "type": "validation_error",
        })
        .to_string()
    }

    fn submit_test_archive(client: &MoocClient) -> MoocClientResult<ExerciseTaskSubmissionResult> {
        client.submit(
            Uuid::parse_str(EXERCISE_ID).unwrap(),
            Uuid::parse_str(SLIDE_ID).unwrap(),
            Uuid::parse_str(TASK_ID).unwrap(),
            Path::new("./tests/data/file"),
        )
    }

    #[test]
    fn submits_uploaded_file_ids_as_json() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let task_submission_id = Uuid::new_v4();
        let slide_submission_id = Uuid::new_v4();
        let upload = mock_upload(&mut server);
        // The body must name the host's file id, not the field name the client chose.
        let submit = server
            .mock("POST", submit_path().as_str())
            .match_header("content-type", "application/json")
            .match_body(Matcher::Json(serde_json::json!({
                "exercise_slide_id": SLIDE_ID,
                "exercise_task_id": TASK_ID,
                "uploaded_file_ids": [FILE_UPLOAD_ID],
            })))
            .with_body(
                serde_json::json!({
                    "task_submission_id": task_submission_id,
                    "slide_submission_id": slide_submission_id,
                })
                .to_string(),
            )
            .create();

        let result = submit_test_archive(&client).unwrap();

        upload.assert();
        submit.assert();
        assert_eq!(result.task_submission_id, task_submission_id);
        assert_eq!(result.slide_submission_id, slide_submission_id);
    }

    #[test]
    fn submit_retries_the_upload_once_on_upload_expired() {
        // The host can reap an upload between the two calls, and only this client
        // can recover: it still holds the archive.
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let upload = mock_upload(&mut server).expect(2);
        let expired = server
            .mock("POST", submit_path().as_str())
            .with_status(422)
            .with_body(upload_expired_body())
            .expect(1)
            .create();
        let accepted = server
            .mock("POST", submit_path().as_str())
            .with_body(
                serde_json::json!({
                    "task_submission_id": Uuid::new_v4(),
                    "slide_submission_id": Uuid::new_v4(),
                })
                .to_string(),
            )
            .expect(1)
            .create();

        submit_test_archive(&client).unwrap();

        upload.assert();
        expired.assert();
        accepted.assert();
    }

    #[test]
    fn submit_surfaces_upload_expired_when_the_retry_fails_too() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let upload = mock_upload(&mut server).expect(2);
        // Both submits expire, so there is nothing left to recover from.
        let expired = server
            .mock("POST", submit_path().as_str())
            .with_status(422)
            .with_body(upload_expired_body())
            .expect(2)
            .create();

        let err = submit_test_archive(&client).unwrap_err();

        upload.assert();
        expired.assert();
        assert!(is_upload_expired(&err), "unexpected error: {err}");
    }

    #[test]
    fn submit_surfaces_unknown_upload_without_retrying() {
        // An unknown upload is a client bug or tampering, never a race, so
        // re-uploading could only mask it.
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let upload = mock_upload(&mut server).expect(1);
        let unknown = server
            .mock("POST", submit_path().as_str())
            .with_status(422)
            .with_body(
                serde_json::json!({
                    "errors": [],
                    "message": "unknown upload",
                    "message_key": UNKNOWN_UPLOAD_MESSAGE_KEY,
                    "metadata": null,
                    "type": "validation_error",
                })
                .to_string(),
            )
            .expect(1)
            .create();

        let err = submit_test_archive(&client).unwrap_err();

        upload.assert();
        unknown.assert();
        assert!(matches!(
            *err,
            MoocClientError::HttpError {
                ref message_key,
                ..
            } if message_key.as_deref() == Some(UNKNOWN_UPLOAD_MESSAGE_KEY)
        ));
    }

    #[test]
    fn upload_files_keys_each_part_by_a_distinct_uuid() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let ids = std::sync::Arc::new(Mutex::new(Vec::new()));
        let captured = ids.clone();
        let upload = server
            .mock(
                "POST",
                format!("/api/v0/exercise-services/client/exercises/{EXERCISE_ID}/files").as_str(),
            )
            .match_request(move |request| {
                let body =
                    String::from_utf8_lossy(request.body().map_or(&[][..], |b| &b[..])).to_string();
                // `; name="` and not `name="`, which `filename="` also ends with.
                let names = body
                    .split("; name=\"")
                    .skip(1)
                    .filter_map(|rest| rest.split('"').next().map(str::to_string))
                    .collect::<Vec<_>>();
                captured
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .extend(names);
                true
            })
            .with_body(serde_json::json!({ "files": [] }).to_string())
            .create();

        client
            .upload_files(
                Uuid::parse_str(EXERCISE_ID).unwrap(),
                &[
                    ("a.tar.zst", Path::new("./tests/data/file")),
                    ("b.tar.zst", Path::new("./tests/data/file")),
                ],
            )
            .unwrap();

        upload.assert();
        let names = ids.lock().unwrap_or_else(|e| e.into_inner()).clone();
        assert_eq!(names.len(), 2, "one part per file: {names:?}");
        assert_ne!(names[0], names[1], "field names must be distinct");
        for name in &names {
            Uuid::parse_str(name).expect("every field name must be a UUID");
        }
    }

    #[test]
    fn upload_files_sends_no_request_for_an_empty_list() {
        // The host rejects an empty multipart body, so an empty list must not
        // become a doomed request.
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let never = server
            .mock(
                "POST",
                format!("/api/v0/exercise-services/client/exercises/{EXERCISE_ID}/files").as_str(),
            )
            .expect(0)
            .create();

        let uploaded = client
            .upload_files(Uuid::parse_str(EXERCISE_ID).unwrap(), &[])
            .unwrap();

        assert!(uploaded.is_empty());
        never.assert();
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
                format!("/api/v0/exercise-services/client/exercises/{exercise_id}/files").as_str(),
            )
            .with_body(
                serde_json::json!({
                    "files": [{
                        "id": FILE_UPLOAD_ID,
                        "name": "submission.tar.zst",
                        "download_url": "http://example.com/archive.tar.zst",
                    }]
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
            .with_body(
                serde_json::json!({
                    "task_submission_id": Uuid::new_v4(),
                    "slide_submission_id": Uuid::new_v4(),
                })
                .to_string(),
            )
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

    /// Mocks `GET submissions/{id}/download` with the given file list.
    fn mock_submission_download(
        server: &mut Server,
        submission_id: &str,
        files: serde_json::Value,
    ) -> mockito::Mock {
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/submissions/{submission_id}/download")
                    .as_str(),
            )
            .with_body(serde_json::json!({ "files": files }).to_string())
            .create()
    }

    fn submission_file(name: &str, download_url: &str) -> serde_json::Value {
        serde_json::json!({
            "id": Uuid::new_v4(),
            "name": name,
            "download_url": download_url,
        })
    }

    #[test]
    fn downloads_submission_archive_url() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let submission_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
        mock_submission_download(
            &mut server,
            submission_id,
            serde_json::json!([submission_file(
                "submission.tar.zst",
                "http://example.com/archive.tar.zst"
            )]),
        );
        let url = client
            .download_submission_archive_url(Uuid::parse_str(submission_id).unwrap())
            .unwrap();
        assert_eq!(url.as_deref(), Some("http://example.com/archive.tar.zst"));
    }

    #[test]
    fn download_submission_archive_url_rejects_a_multi_file_submission() {
        // Restoring an editor submission overlays exactly one archive. Picking one
        // of several would silently restore the wrong project, so it must fail.
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let submission_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
        mock_submission_download(
            &mut server,
            submission_id,
            serde_json::json!([
                submission_file("a.tar.zst", "http://example.com/a.tar.zst"),
                submission_file("b.tar.zst", "http://example.com/b.tar.zst"),
            ]),
        );
        let err = client
            .download_submission_archive_url(Uuid::parse_str(submission_id).unwrap())
            .unwrap_err();
        assert!(matches!(
            *err,
            MoocClientError::UnexpectedSubmissionFileCount { count: 2, .. }
        ));
    }

    #[test]
    fn download_submission_archive_url_reports_a_submission_with_no_files() {
        // The exercise's submission list includes browser answers, which have no
        // uploads: `{"files": []}` is the contract's blessed response for them,
        // not an error.
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let submission_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
        mock_submission_download(&mut server, submission_id, serde_json::json!([]));
        let url = client
            .download_submission_archive_url(Uuid::parse_str(submission_id).unwrap())
            .unwrap();
        assert_eq!(url, None);
    }

    #[test]
    fn download_submission_files_returns_the_whole_list() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let submission_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
        mock_submission_download(
            &mut server,
            submission_id,
            serde_json::json!([
                submission_file("a.txt", "http://example.com/a.txt"),
                submission_file("b.txt", "http://example.com/b.txt"),
            ]),
        );
        let files = client
            .download_submission_files(Uuid::parse_str(submission_id).unwrap())
            .unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].name, "a.txt");
        assert_eq!(files[1].download_url, "http://example.com/b.txt");
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
    fn malformed_json_on_success_status_yields_deserializing_response_error() {
        // `send_expect_bytes` only errors on a read failure, not on status, so a
        // 2xx response with a non-JSON body reaches `serde_json::from_slice` in
        // `send_expect_json` and must surface as `DeserializingResponse`, not
        // silently succeed or panic.
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let path = "/api/v0/exercise-services/client/courses";
        server
            .mock("GET", path)
            .with_body("not valid json")
            .create();
        let err = client.courses().unwrap_err();
        match *err {
            MoocClientError::DeserializingResponse { url, .. } => {
                assert_eq!(url.path(), path);
            }
            other => panic!("expected DeserializingResponse, got {other:?}"),
        }
    }

    #[test]
    fn set_token_works_after_cloning_client() {
        // Regression: `set_token` used `Arc::get_mut().expect(...)`, which panics
        // if any clone exists — and `MoocClient` is cloned across download threads.
        init();
        let server = Server::new();
        let mut client = MoocClient::new(server.url().parse().unwrap()).unwrap();
        // Keep several clones alive so `Arc::get_mut` would fail.
        let _clone_a = client.clone();
        let _clone_b = client.clone();
        let token = Token::new(
            AccessToken::new("after-clone".to_string()),
            BasicTokenType::Bearer,
            EmptyExtraTokenFields {},
        );
        // Must not panic.
        client.set_token(token);
    }

    #[test]
    fn set_token_on_clone_is_visible_to_that_clone() {
        // A token set on one handle is stored behind the shared `Arc`, so a clone
        // observes it too (the interior mutability is shared, not per-handle).
        init();
        let _env = env_lock();
        // SAFETY: serialized by ENV_LOCK.
        unsafe { std::env::set_var(TRUST_LOCALHOST_VAR, "1") };

        let mut server = Server::new();
        let mut client = MoocClient::new(server.url().parse().unwrap()).unwrap();
        let clone = client.clone();
        let token = Token::new(
            AccessToken::new("shared-token".to_string()),
            BasicTokenType::Bearer,
            EmptyExtraTokenFields {},
        );
        client.set_token(token);

        let with_auth = server
            .mock("GET", "/api/v0/exercise-services/client/courses")
            .match_header("authorization", "Bearer shared-token")
            .with_body("[]")
            .expect(1)
            .create();
        let res = clone.courses();
        unsafe { std::env::remove_var(TRUST_LOCALHOST_VAR) };
        res.unwrap();
        with_auth.assert();
    }

    #[test]
    fn rejects_insecure_scheme_for_production_host() {
        // courses.mooc.fi over http is refused so the bearer token can never be
        // sent in plaintext; https is accepted.
        init();
        match MoocClient::new("http://courses.mooc.fi/".parse().unwrap()) {
            Err(err) => assert!(matches!(*err, MoocClientError::InsecureScheme { .. })),
            Ok(_) => panic!("http://courses.mooc.fi should be rejected"),
        }
        assert!(MoocClient::new("https://courses.mooc.fi/".parse().unwrap()).is_ok());
        // The local-dev host may use http.
        assert!(MoocClient::new("http://project-331.local/".parse().unwrap()).is_ok());
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
