#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Used to communicate with the Courses MOOC server. See the `MoocClient` struct for more details.

mod auth;
mod error;
mod exercise;
mod grading;

use self::error::redact_query;
pub use self::{
    auth::{
        AUTH_REQUEST_TIMEOUT, DEFAULT_CLIENT_ID, DEFAULT_POLL_INTERVAL_SECS, DEVICE_GRANT_TYPE,
        DEVICE_SCOPE, DeviceAuthorizationResponse, DeviceTokenPoll, device_authorization,
        poll_device_token, refresh_token, revoke_refresh_token,
    },
    error::{MoocClientError, MoocClientResult},
    exercise::{
        BrowserTestRuntime, BrowserTestSpec, ChapterLockingStatus, ExerciseChapter, ExerciseType,
        ModelSolutionSpec, PublicSpec, TMC_SERVICE_SLUG, TmcExerciseSlide, TmcExerciseTask,
    },
    grading::{GradingTestResult, GradingTestRun, GradingTestRunStatus},
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
/// destinations. Off by default so production never sends the token to a local
/// host; tests and local development opt in.
pub const TRUST_LOCALHOST_VAR: &str = "TMC_LANGS_MOOC_TRUST_LOCALHOST";

/// Public because the same access token is also accepted by tmc-server, so the
/// decision to hand it to a local host has to be made identically on both paths.
pub fn trust_localhost() -> bool {
    std::env::var(TRUST_LOCALHOST_VAR).as_deref() == Ok("1")
}

/// Timeout for ordinary metadata requests, and the connect timeout for every
/// request. Mirrors `auth.rs`: without it a wedged host hangs the CLI forever
/// instead of surfacing a retryable `ConnectionError`.
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

/// Backend `message_key` of a 429: the user stored too many pastes of the exercise recently.
pub const RATE_LIMITED_MESSAGE_KEY: &str = "rate_limited";

/// Backend `message_key`s of a refused submission (422 except where noted).
pub const DEADLINE_PASSED_MESSAGE_KEY: &str = "deadline_passed";
pub const OUT_OF_TRIES_MESSAGE_KEY: &str = "out_of_tries";
pub const NOT_ENROLLED_MESSAGE_KEY: &str = "not_enrolled";
/// 403 for a token that lacks the `exercise-services` scope.
pub const INSUFFICIENT_SCOPE_MESSAGE_KEY: &str = "insufficient_scope";
/// 403 for a course or exercise the user may not view.
pub const NO_ACCESS_MESSAGE_KEY: &str = "no_access";
/// 403s from a closed chapter.
pub const CHAPTER_NOT_OPEN_YET_MESSAGE_KEY: &str = "chapter_not_open_yet";
pub const CHAPTER_NOT_UNLOCKED_YET_MESSAGE_KEY: &str = "chapter_not_unlocked_yet";
pub const CHAPTER_COMPLETED_AND_LOCKED_MESSAGE_KEY: &str = "chapter_completed_and_locked";
/// 403 refusing a submit while `ExerciseProgress::exercise_locked` is set.
pub const EXERCISE_LOCKED_MESSAGE_KEY: &str = "exercise_locked";

/// File name sent for the submitted project archive. The host stores it as the
/// upload's `name`; nothing depends on the extension.
const SUBMISSION_ARCHIVE_FILE_NAME: &str = "submission.tar.zst";

/// Content type sent for every uploaded part. Stated because reqwest guesses from the
/// path extension (the archive is an extensionless temp file, and `mime_guess` has no
/// `zst` entry), which would reach the host as `application/octet-stream`. Must equal what the tmc exercise service writes for the
/// same archive made in its IFrame (`services/tmc/src/util/answerArchive.ts`): the host
/// echoes it into `AnswerFile::mime`, from which the teacher-facing answer-file zip
/// derives entry extensions.
const ANSWER_ARCHIVE_MIME: &str = "application/x-zstd-compressed-tar";

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
    /// Fails with [`MoocClientError::InsecureScheme`] if the root URL is the
    /// production host `courses.mooc.fi` over anything but `https`.
    pub fn new(root_url: Url) -> MoocClientResult<Self> {
        // guarantee a trailing slash, otherwise join will drop the last component
        let root_url = if root_url.as_str().ends_with('/') {
            root_url
        } else {
            format!("{root_url}/").parse().expect("invalid root url")
        };

        // The bearer token must never go over plaintext to production.
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
        log::debug!("building a request to {}", redact_query(url.as_str()));

        // Attach the bearer only to trusted hosts, so a (possibly attacker-controlled)
        // URL never receives it. `host_str` rather than `domain` so IP literals count.
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
            // The crate version is the langs version; mirrors the TMC client's `client_version`.
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
    /// Fetches the account the stored token acts for.
    pub fn current_user(&self) -> MoocClientResult<CurrentUser> {
        let url = make_client_api_url(self, "me")?;
        let res = self
            .request(Method::GET, url)
            .send_expect_json::<api::CurrentUser>()?;
        Ok(res.into())
    }

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

    /// Fetches the current user's per-exercise progress for a whole course in one
    /// round-trip: one entry per exercise they can see (open chapters), untouched
    /// ones zeroed. Course-level totals are not sent; sum over the entries.
    pub fn course_progress(&self, course: Uuid) -> MoocClientResult<CourseProgress> {
        let url = make_client_api_url(self, format!("courses/{course}/progress"))?;
        let res = self
            .request(Method::GET, url)
            .send_expect_json::<api::CourseProgress>()?;
        Ok(res.into())
    }

    /// The exams the user has started and can still answer.
    pub fn exams(&self) -> MoocClientResult<Vec<Exam>> {
        let url = make_client_api_url(self, "exams")?;
        let res = self
            .request(Method::GET, url)
            .send_expect_json::<Vec<api::Exam>>()?;
        Ok(res.into_iter().map(Into::into).collect())
    }

    /// The exercises of an exam from [`Self::exams`]. Their gradings read as
    /// [`ExerciseTaskSubmissionStatus::GradingHidden`] while the exam can be answered.
    pub fn exam_exercises(&self, exam: Uuid) -> MoocClientResult<Vec<TmcExerciseSlide>> {
        let url = make_client_api_url(self, format!("exams/{exam}/exercises"))?;
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

    /// Downloads a file the backend handed out a URL for (an exercise stub or a submission's
    /// file). A 404 is [`MoocClientError::FileNotFound`].
    pub fn download(&self, url: Url) -> MoocClientResult<Bytes> {
        self.request(Method::GET, url)
            .transfer_timeout()
            .send_expect_bytes()
            .map_err(|err| match *err {
                MoocClientError::HttpError { url, status, .. }
                    if status == StatusCode::NOT_FOUND =>
                {
                    Box::new(MoocClientError::FileNotFound { url })
                }
                _ => err,
            })
    }

    /// Downloads the stub archive for an exercise.
    ///
    /// There is no `exercises/{id}/download` route; the archive lives at the
    /// editor task public spec's `stub_download_url`. Returns
    /// [`MoocClientError::NoDownloadableExerciseTask`] for a browser-only exercise
    /// with no editor task.
    pub fn download_exercise(&self, exercise: Uuid) -> MoocClientResult<Bytes> {
        let slide = self.exercise(exercise)?;
        if slide.has_several_editor_tasks() {
            return Err(Box::new(MoocClientError::SeveralEditorTasks {
                exercise_id: exercise,
            }));
        }
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
        let task_id = submittable_task_id(&slide)?;
        let mut result = self.submit(exercise_id, slide.slide_id, task_id, archive)?;
        result.exercise_page_url = slide.page_url;
        Ok(result)
    }

    /// Stores files for an exercise, returning the host's record of each in the
    /// order they were sent.
    ///
    /// Each part is keyed by a fresh client-chosen UUID, as the host's upload
    /// handler requires. That UUID is *not* the file's identity: only
    /// [`api::AnswerFile::id`], the host's own file id, may be named in a
    /// submit.
    ///
    /// Every part is typed as a tmc project archive, which is the only thing this
    /// client uploads.
    pub fn upload_files(
        &self,
        exercise_id: Uuid,
        files: &[(&str, &Path)],
    ) -> MoocClientResult<Vec<api::AnswerFile>> {
        self.upload_files_for(exercise_id, files, UploadFor::Submission)
    }

    fn upload_files_for(
        &self,
        exercise_id: Uuid,
        files: &[(&str, &Path)],
        upload_for: UploadFor,
    ) -> MoocClientResult<Vec<api::AnswerFile>> {
        if files.is_empty() {
            // The host rejects an empty multipart body, and there is nothing to record.
            return Ok(Vec::new());
        }

        let mut form = Form::new();
        for (name, path) in files {
            // The host requires a file name on every part.
            let part = Part::file(path)
                .map_err(|err| MoocClientError::AttachFileToForm { error: err.into() })?
                .file_name((*name).to_string())
                .mime_str(ANSWER_ARCHIVE_MIME)
                .map_err(|err| MoocClientError::AttachFileToForm { error: err.into() })?;
            form = form.part(Uuid::new_v4().to_string(), part);
        }

        let mut url = make_client_api_url(self, format!("exercises/{exercise_id}/files"))?;
        if upload_for == UploadFor::Snapshot {
            url.set_query(Some("for=snapshot"));
        }
        let res = self
            .request(Method::POST, url)
            .multipart(form)
            .transfer_timeout()
            .send_expect_json::<api::UploadedFiles>()?;
        Ok(res.data_files)
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

    /// Stores an ungraded snapshot of an archive as the answer to an exercise's editor task and
    /// returns its id, which [`Self::download_submission_files`] and [`Self::share_submission`]
    /// take.
    ///
    /// Unlike a submit, it uses no try and is accepted after the deadline and when out of tries.
    /// A [`api::SnapshotPurpose::Paste`] is rate limited ([`RATE_LIMITED_MESSAGE_KEY`]). Returns
    /// [`MoocClientError::NoSubmittableExerciseTask`] for a browser-only exercise.
    pub fn store_snapshot(
        &self,
        exercise_id: Uuid,
        archive: &Path,
        purpose: api::SnapshotPurpose,
    ) -> MoocClientResult<Uuid> {
        let slide = self.exercise(exercise_id)?;
        let task_id = submittable_task_id(&slide)?;
        let files = [(SUBMISSION_ARCHIVE_FILE_NAME, archive)];
        let store = |uploaded: &[api::AnswerFile]| {
            self.store_uploaded_snapshot(exercise_id, slide.slide_id, task_id, uploaded, purpose)
        };
        let uploaded = self.upload_files_for(exercise_id, &files, UploadFor::Snapshot)?;
        match store(&uploaded) {
            Err(error) if is_upload_expired(&error) => {
                log::warn!(
                    "an upload for exercise {exercise_id} expired before its snapshot was \
                     stored; uploading again"
                );
                let uploaded = self.upload_files_for(exercise_id, &files, UploadFor::Snapshot)?;
                store(&uploaded)
            }
            other => other,
        }
    }

    fn store_uploaded_snapshot(
        &self,
        exercise_id: Uuid,
        slide_id: Uuid,
        task_id: Uuid,
        uploaded: &[api::AnswerFile],
        purpose: api::SnapshotPurpose,
    ) -> MoocClientResult<Uuid> {
        let snapshot = api::NewAnswerSnapshot {
            purpose,
            exercise_slide_id: slide_id,
            exercise_task_id: task_id,
            data_json: None,
            data_files: uploaded.iter().map(|file| file.id).collect(),
        };
        let snapshot = serialize::to_json_vec(&snapshot)
            .map_err(Into::into)
            .map_err(Box::new)?;

        let url = make_client_api_url(self, format!("exercises/{exercise_id}/snapshots"))?;
        let res = self
            .request(Method::POST, url)
            .json_body(snapshot)
            .send_expect_json::<api::AnswerSnapshotResult>()?;
        Ok(res.snapshot_id)
    }

    /// Submits a body naming already-stored files. Split out of [`Self::submit`]
    /// so the upload can be retried without re-entering the retry itself.
    fn submit_uploaded(
        &self,
        exercise_id: Uuid,
        slide_id: Uuid,
        task_id: Uuid,
        uploaded: &[api::AnswerFile],
    ) -> MoocClientResult<ExerciseTaskSubmissionResult> {
        let submission = api::ExerciseSlideSubmission {
            exercise_slide_id: slide_id,
            exercise_task_id: task_id,
            // The host reads an absent `answer_kind` as `json`, then rejects a json answer
            // that names files, so omitting it would 422 every submission.
            answer_kind: Some(api::AnswerKind::File),
            data_json: None,
            data_files: Some(uploaded.iter().map(|file| file.id).collect()),
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

    /// Returns the current user's past submissions and snapshots of an exercise, newest first.
    /// Each item's `id` is the value passed to [`MoocClient::download_submission_files`] and
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

    /// Returns the files a submission's or snapshot's answer is made of (the id
    /// comes from [`MoocClient::get_exercise_submissions`]), in the order the host
    /// grades and displays them.
    ///
    /// Empty when the host has no files for the submission — an exercise type with
    /// none. The host records a file answer the same way whether it was made from a
    /// native client or in the exercise service's IFrame, so where it came from does
    /// not change what comes back.
    pub fn download_submission_files(
        &self,
        submission_id: Uuid,
    ) -> MoocClientResult<Vec<api::AnswerFile>> {
        let url = make_client_api_url(self, format!("submissions/{submission_id}/download"))?;
        let res = self
            .request(Method::GET, url)
            .send_expect_json::<api::SubmissionFiles>()?;
        Ok(res.data_files)
    }

    /// Mints a shareable link to an existing submission or snapshot of the current user and
    /// returns the paste URL.
    pub fn share_submission(&self, submission_id: Uuid) -> MoocClientResult<PasteResult> {
        let url = make_client_api_url(self, format!("submissions/{submission_id}/share"))?;
        let res = self
            .request(Method::POST, url)
            .send_expect_json::<api::PasteResult>()?;
        Ok(res.into())
    }
}

/// Which request will name an upload; a snapshot's upload skips the deadline and try checks.
#[derive(Clone, Copy, PartialEq, Eq)]
enum UploadFor {
    Submission,
    Snapshot,
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

    /// Uses [`TRANSFER_TIMEOUT`] instead of the metadata timeout, for large payloads.
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
                                    error: Box::new(err.without_url()),
                                })?;
                        // Controlled errors carry an `ApiErrorResponse` `message_key`
                        // (e.g. `not_enrolled`); lift it so the CLI can map it to a typed
                        // error kind.
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
                error.without_url(),
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
                error: Box::new(err.without_url()),
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

/// The one field of the backend's `ApiErrorResponse` the client needs: the stable
/// `message_key` of a controlled error. Lenient (missing/null = `None`) so a
/// non-conforming body yields no key rather than failing.
#[derive(Deserialize)]
struct ApiErrorBody {
    #[serde(default)]
    message_key: Option<String>,
}

/// The task a native client answers on the slide.
fn submittable_task_id(slide: &TmcExerciseSlide) -> MoocClientResult<Uuid> {
    let exercise_id = slide.exercise_id;
    if slide.has_several_editor_tasks() {
        return Err(Box::new(MoocClientError::SeveralEditorTasks {
            exercise_id,
        }));
    }
    slide
        .editor_task_id()
        .ok_or_else(|| Box::new(MoocClientError::NoSubmittableExerciseTask { exercise_id }))
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

/// The courses.mooc.fi account a token acts for; a changed `id` after login means the
/// student switched accounts.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
#[cfg_attr(feature = "ts-rs", ts(rename = "MoocCurrentUser"))]
#[schemars(rename = "MoocCurrentUser")]
pub struct CurrentUser {
    pub id: Uuid,
    /// For showing which account is logged in. `None` when the user gave none, and from a host
    /// that predates the field.
    #[serde(default)]
    pub name: Option<String>,
    /// `None` from a host that predates the field.
    #[serde(default)]
    pub email: Option<String>,
}

impl From<api::CurrentUser> for CurrentUser {
    fn from(value: api::CurrentUser) -> Self {
        Self {
            id: value.id,
            name: value.name,
            email: value.email,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
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
    /// `not_set` from a host that predates the field.
    #[serde(default)]
    pub ai_policy: CourseAiPolicy,
    /// The course material's front page. `None` from a host that predates the field.
    #[serde(default)]
    pub url: Option<String>,
    /// IETF BCP 47 tag such as `fi-FI`. `None` from a host that predates the field.
    #[serde(default)]
    pub language_code: Option<String>,
    /// Shared by every language version of the course; each version is a separate course with
    /// its own exercises.
    #[serde(default)]
    pub language_group_id: Option<Uuid>,
    /// The language version the user moved to from this course, if they did.
    #[serde(default)]
    pub moved_to_course_id: Option<Uuid>,
    /// When the course run ends or ended; it takes no new students from then on.
    #[serde(default)]
    pub closed_at: Option<DateTime<Utc>>,
    /// The course that continues this one after it closed, if its teacher named one.
    #[serde(default)]
    pub successor_course_id: Option<Uuid>,
    /// `Some(false)` for a course the user only teaches: submits to it fail with the
    /// `not-enrolled` error kind. `None` from a host that predates the field.
    #[serde(default)]
    pub is_enrolled: Option<bool>,
    /// Staff get exercises of unopened chapters and model solutions whatever their progress.
    #[serde(default)]
    pub is_staff: bool,
}

impl From<api::Course> for Course {
    fn from(value: api::Course) -> Self {
        Self {
            id: value.id,
            slug: value.slug,
            name: value.name,
            description: value.description,
            organization_name: value.organization_name,
            ai_policy: value.ai_policy.into(),
            url: value.url,
            language_code: value.language_code,
            language_group_id: value.language_group_id,
            moved_to_course_id: value.moved_to_course_id,
            closed_at: value.closed_at,
            successor_course_id: value.successor_course_id,
            is_enrolled: value.is_enrolled,
            is_staff: value.is_staff,
        }
    }
}

/// A course's AI-use policy as chosen by its teacher, uninterpreted: what each value permits
/// is the client's decision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum CourseAiPolicy {
    /// The teacher has not chosen a policy.
    #[default]
    NotSet,
    /// AI may not be used at any point.
    NoAi,
    /// AI may be used for planning, such as brainstorming or outlining, but not in the final work.
    PlanningOnly,
    /// AI may be used for specific tasks only, with disclosure.
    Limited,
    /// AI may be used freely, with disclosure.
    FullUse,
    /// AI use is expected.
    Required,
    /// A value newer than this client.
    Unrecognized,
}

// Hand-written rather than `#[serde(other)]`, which ts-rs cannot parse.
impl<'de> Deserialize<'de> for CourseAiPolicy {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match String::deserialize(deserializer)?.as_str() {
            "not_set" => Self::NotSet,
            "no_ai" => Self::NoAi,
            "planning_only" => Self::PlanningOnly,
            "limited" => Self::Limited,
            "full_use" => Self::FullUse,
            "required" => Self::Required,
            _ => Self::Unrecognized,
        })
    }
}

impl From<api::CourseAiPolicy> for CourseAiPolicy {
    fn from(value: api::CourseAiPolicy) -> Self {
        match value {
            api::CourseAiPolicy::NotSet => Self::NotSet,
            api::CourseAiPolicy::NoAi => Self::NoAi,
            api::CourseAiPolicy::PlanningOnly => Self::PlanningOnly,
            api::CourseAiPolicy::Limited => Self::Limited,
            api::CourseAiPolicy::FullUse => Self::FullUse,
            api::CourseAiPolicy::Required => Self::Required,
            api::CourseAiPolicy::Unrecognized => Self::Unrecognized,
        }
    }
}

/// The current user's progress across every exercise they can see in a course.
/// Course-level totals (awarded/available points, passed count, percentage) are
/// not sent separately; derive them by summing over `exercises`, guarding the
/// percentage against a zero total.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct CourseProgress {
    pub course_id: Uuid,
    pub exercises: Vec<ExerciseProgress>,
    /// Chapters that have not opened yet. Their exercises are missing from `exercises` and
    /// from the course's exercise list until then; they are closed, not removed.
    #[serde(default)]
    pub closed_chapters: Vec<ClosedChapter>,
}

impl From<api::CourseProgress> for CourseProgress {
    fn from(value: api::CourseProgress) -> Self {
        Self {
            course_id: value.course_id,
            exercises: value.exercises.into_iter().map(Into::into).collect(),
            closed_chapters: value.closed_chapters.into_iter().map(Into::into).collect(),
        }
    }
}

/// A chapter of the course that opens later, with the user's progress on its exercises.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct ClosedChapter {
    pub id: Uuid,
    pub name: String,
    /// The chapter's position in the course, from 1.
    pub chapter_number: i32,
    pub opens_at: DateTime<Utc>,
    pub exercises: Vec<ExerciseProgress>,
}

impl From<api::ClosedChapter> for ClosedChapter {
    fn from(value: api::ClosedChapter) -> Self {
        Self {
            id: value.id,
            name: value.name,
            chapter_number: value.chapter_number,
            opens_at: value.opens_at,
            exercises: value.exercises.into_iter().map(Into::into).collect(),
        }
    }
}

/// The current user's progress on a single exercise. `standing` is the "passed" signal;
/// `completed` is only the activity stage, which any graded submission can reach.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
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
    /// `None` from a host that predates the field.
    pub standing: Option<ExerciseStanding>,
    /// How many graded submissions the exercise accepts per slide; `None` when unlimited.
    #[serde(default)]
    pub max_tries: Option<i32>,
    /// How many of `max_tries` are left on the slide the user answers; `None` when unlimited.
    #[serde(default)]
    pub tries_left: Option<i32>,
    /// What the user's points wait for when a teacher must review their answer first. While set,
    /// `score_given` is `0.0` and `standing` is `Attempted`, even after a fully passing grading.
    #[serde(default)]
    pub pending_teacher_review: Option<PendingTeacherReview>,
    /// `true` once the exercise takes no more submissions from the user, whatever its chapter's
    /// locking status says. A submit fails with the `chapter-completed-and-locked` error kind
    /// while the chapter is locked for the user, and with `exercise-locked` otherwise.
    #[serde(default)]
    pub exercise_locked: bool,
    /// Where the answer is in the exercise's peer, self or teacher review, which happens in the
    /// course material. `None` while the user can still answer, and from a host that predates
    /// the field.
    #[serde(default)]
    pub reviewing_stage: Option<ReviewingStage>,
    /// A teacher's grading that overrides the automatic score. `None` when there is none to show.
    #[serde(default)]
    pub teacher_review: Option<TeacherReview>,
}

/// Where an answer is in an exercise's review process. No stage takes new answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum ReviewingStage {
    /// The user must give peer reviews in the course material.
    PeerReview,
    /// The user must review their own answer in the course material.
    SelfReview,
    /// The user has given their reviews and waits for others to review their answer.
    WaitingForPeerReviews,
    WaitingForTeacher,
    /// Reviews are done and points awarded; the answer is final.
    ReviewedAndLocked,
    /// The answer was locked with its chapter before any review.
    Locked,
    /// The chapter was locked before the user answered.
    NotAnsweredAndLocked,
    /// A value newer than this client.
    Unrecognized,
}

// Hand-written rather than `#[serde(other)]`, which ts-rs cannot parse.
impl<'de> Deserialize<'de> for ReviewingStage {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match String::deserialize(deserializer)?.as_str() {
            "peer_review" => Self::PeerReview,
            "self_review" => Self::SelfReview,
            "waiting_for_peer_reviews" => Self::WaitingForPeerReviews,
            "waiting_for_teacher" => Self::WaitingForTeacher,
            "reviewed_and_locked" => Self::ReviewedAndLocked,
            "locked" => Self::Locked,
            "not_answered_and_locked" => Self::NotAnsweredAndLocked,
            _ => Self::Unrecognized,
        })
    }
}

impl From<api::ReviewingStage> for ReviewingStage {
    fn from(value: api::ReviewingStage) -> Self {
        match value {
            api::ReviewingStage::PeerReview => Self::PeerReview,
            api::ReviewingStage::SelfReview => Self::SelfReview,
            api::ReviewingStage::WaitingForPeerReviews => Self::WaitingForPeerReviews,
            api::ReviewingStage::WaitingForTeacher => Self::WaitingForTeacher,
            api::ReviewingStage::ReviewedAndLocked => Self::ReviewedAndLocked,
            api::ReviewingStage::Locked => Self::Locked,
            api::ReviewingStage::NotAnsweredAndLocked => Self::NotAnsweredAndLocked,
            api::ReviewingStage::Unrecognized => Self::Unrecognized,
        }
    }
}

/// A teacher's grading of the user's answer.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct TeacherReview {
    pub decision: TeacherDecision,
    /// Replaces the automatic score.
    pub score_given: f32,
    pub justification: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl From<api::TeacherReview> for TeacherReview {
    fn from(value: api::TeacherReview) -> Self {
        Self {
            decision: value.decision.into(),
            score_given: value.score_given,
            justification: value.justification,
            created_at: value.created_at,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum TeacherDecision {
    ZeroPoints,
    CustomPoints,
    SuspectedPlagiarism,
    /// The answer was rejected and the exercise reset, so the user can answer again.
    RejectAndReset,
    UnauthorizedAiUse,
    BadAnswer,
    Other,
    /// A value newer than this client.
    Unrecognized,
}

// Hand-written rather than `#[serde(other)]`, which ts-rs cannot parse.
impl<'de> Deserialize<'de> for TeacherDecision {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match String::deserialize(deserializer)?.as_str() {
            "zero_points" => Self::ZeroPoints,
            "custom_points" => Self::CustomPoints,
            "suspected_plagiarism" => Self::SuspectedPlagiarism,
            "reject_and_reset" => Self::RejectAndReset,
            "unauthorized_ai_use" => Self::UnauthorizedAiUse,
            "bad_answer" => Self::BadAnswer,
            "other" => Self::Other,
            _ => Self::Unrecognized,
        })
    }
}

impl From<api::TeacherDecision> for TeacherDecision {
    fn from(value: api::TeacherDecision) -> Self {
        match value {
            api::TeacherDecision::ZeroPoints => Self::ZeroPoints,
            api::TeacherDecision::CustomPoints => Self::CustomPoints,
            api::TeacherDecision::SuspectedPlagiarism => Self::SuspectedPlagiarism,
            api::TeacherDecision::RejectAndReset => Self::RejectAndReset,
            api::TeacherDecision::UnauthorizedAiUse => Self::UnauthorizedAiUse,
            api::TeacherDecision::BadAnswer => Self::BadAnswer,
            api::TeacherDecision::Other => Self::Other,
            api::TeacherDecision::Unrecognized => Self::Unrecognized,
        }
    }
}

/// What a user's points on an exercise wait for when a teacher reviews answers before awarding
/// points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum PendingTeacherReview {
    /// The answer is graded, but its points come only after the user completes and locks the
    /// exercise's chapter and a teacher reviews the answer. Submitting stays possible until then.
    AwaitsChapterLock,
    /// The answer waits for a teacher's review, and the exercise is locked.
    AwaitsTeacher,
    /// A value newer than this client.
    Unrecognized,
}

// Hand-written rather than `#[serde(other)]`, which ts-rs cannot parse.
impl<'de> Deserialize<'de> for PendingTeacherReview {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match String::deserialize(deserializer)?.as_str() {
            "awaits_chapter_lock" => Self::AwaitsChapterLock,
            "awaits_teacher" => Self::AwaitsTeacher,
            _ => Self::Unrecognized,
        })
    }
}

impl From<api::PendingTeacherReview> for PendingTeacherReview {
    fn from(value: api::PendingTeacherReview) -> Self {
        match value {
            api::PendingTeacherReview::AwaitsChapterLock => Self::AwaitsChapterLock,
            api::PendingTeacherReview::AwaitsTeacher => Self::AwaitsTeacher,
            api::PendingTeacherReview::Unrecognized => Self::Unrecognized,
        }
    }
}

/// Where the user stands on an exercise, as the host decides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub enum ExerciseStanding {
    /// No graded submission and no try used.
    NotAttempted,
    /// Below full points with tries left, still being graded, in peer or self review, or waiting
    /// for a teacher's review (`pending_teacher_review`).
    Attempted,
    /// A grading awarded full points.
    Passed,
    /// Below full points (0 included), the try limit used up and no review pending, so the score
    /// is final.
    OutOfTries,
    /// A value newer than this client.
    Unrecognized,
}

// Hand-written rather than `#[serde(other)]`, which ts-rs cannot parse.
impl<'de> Deserialize<'de> for ExerciseStanding {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match String::deserialize(deserializer)?.as_str() {
            "NotAttempted" => Self::NotAttempted,
            "Attempted" => Self::Attempted,
            "Passed" => Self::Passed,
            "OutOfTries" => Self::OutOfTries,
            _ => Self::Unrecognized,
        })
    }
}

impl From<api::ExerciseStanding> for ExerciseStanding {
    fn from(value: api::ExerciseStanding) -> Self {
        match value {
            api::ExerciseStanding::NotAttempted => Self::NotAttempted,
            api::ExerciseStanding::Attempted => Self::Attempted,
            api::ExerciseStanding::Passed => Self::Passed,
            api::ExerciseStanding::OutOfTries => Self::OutOfTries,
            api::ExerciseStanding::Unrecognized => Self::Unrecognized,
        }
    }
}

impl From<api::ExerciseProgress> for ExerciseProgress {
    fn from(value: api::ExerciseProgress) -> Self {
        Self {
            exercise_id: value.exercise_id,
            score_given: value.score_given,
            score_maximum: value.score_maximum,
            completed: value.completed,
            attempted: value.attempted,
            standing: value.standing.map(Into::into),
            max_tries: value.max_tries,
            tries_left: value.tries_left,
            pending_teacher_review: value.pending_teacher_review.map(Into::into),
            exercise_locked: value.exercise_locked,
            reviewing_stage: value.reviewing_stage.map(Into::into),
            teacher_review: value.teacher_review.map(Into::into),
        }
    }
}

/// An exam the user has started and can still answer.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
#[cfg_attr(feature = "ts-rs", ts(rename = "MoocExam"))]
#[schemars(rename = "MoocExam")]
pub struct Exam {
    pub id: Uuid,
    pub name: String,
    pub started_at: DateTime<Utc>,
    /// Answers are accepted until then: the end of the user's time or of the exam, whichever
    /// comes first.
    pub answerable_until: DateTime<Utc>,
}

impl From<api::Exam> for Exam {
    fn from(value: api::Exam) -> Self {
        Self {
            id: value.id,
            name: value.name,
            started_at: value.started_at,
            answerable_until: value.answerable_until,
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
    /// The exercise's course material page, where the submission's grading also shows. Set by
    /// [`MoocClient::submit_exercise`], which has the exercise at hand; `None` otherwise, or when
    /// the host reports no page.
    pub exercise_page_url: Option<String>,
}

impl From<api::ExerciseTaskSubmissionResult> for ExerciseTaskSubmissionResult {
    fn from(value: api::ExerciseTaskSubmissionResult) -> Self {
        Self {
            task_submission_id: value.task_submission_id,
            slide_submission_id: value.slide_submission_id,
            exercise_page_url: None,
        }
    }
}

/// The grading status of a task submission, as polled after `submit`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "status", rename_all = "kebab-case")]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub enum ExerciseTaskSubmissionStatus {
    /// The submission has no grading record yet.
    NoGradingYet,
    /// A grading record exists. It may still be in progress; `grading_progress`
    /// says how far it has got.
    Grading { grading: Box<Grading> },
    /// The submission belongs to an exam the user can still answer; its grading shows only
    /// after that.
    GradingHidden,
    /// A status newer than this client.
    Unrecognized,
}

/// A task submission's grading record.
#[derive(Debug, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct Grading {
    pub grading_progress: GradingProgress,
    /// Absent until grading has produced a value; a partial value while
    /// `grading_progress` is still pending.
    pub score_given: Option<f32>,
    pub grading_started_at: Option<DateTime<Utc>>,
    pub grading_completed_at: Option<DateTime<Utc>>,
    /// Human-readable feedback, for a client to display as-is.
    pub feedback_text: Option<String>,
    /// The user's progress on the whole exercise as of this poll; its `completed` is the
    /// authoritative "passed" signal. `None` for an exam exercise or from a host that does not
    /// report it.
    pub exercise_progress: Option<ExerciseProgress>,
    /// The test run the grading was based on. `None` until grading has produced one, or when the
    /// exercise service reported something else.
    pub test_results: Option<GradingTestRun>,
    /// The exercise in the course material, where the latest submission and its grading show.
    /// `None` for an exam exercise, and from a host that predates the field.
    pub submission_page_url: Option<String>,
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
                exercise_progress,
                submission_page_url,
            } => Self::Grading {
                grading: Box::new(Grading {
                    grading_progress: grading_progress.into(),
                    score_given,
                    grading_started_at,
                    grading_completed_at,
                    feedback_text,
                    exercise_progress: exercise_progress.map(|progress| (*progress).into()),
                    test_results: feedback_json.and_then(GradingTestRun::from_feedback_json),
                    submission_page_url,
                }),
            },
            api::ExerciseTaskSubmissionStatus::GradingHidden => Self::GradingHidden,
            api::ExerciseTaskSubmissionStatus::Unrecognized => Self::Unrecognized,
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
    /// A value newer than this client.
    Unrecognized,
}

impl From<api::GradingProgress> for GradingProgress {
    fn from(value: api::GradingProgress) -> Self {
        match value {
            api::GradingProgress::Failed => Self::Failed,
            api::GradingProgress::NotReady => Self::NotReady,
            api::GradingProgress::PendingManual => Self::PendingManual,
            api::GradingProgress::Pending => Self::Pending,
            api::GradingProgress::FullyGraded => Self::FullyGraded,
            api::GradingProgress::Unrecognized => Self::Unrecognized,
        }
    }
}

/// A past submission or snapshot of the current user to an exercise. `id` is what
/// [`MoocClient::download_submission_files`] and [`MoocClient::share_submission`] take.
#[derive(Debug, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct ExerciseSlideSubmissionListItem {
    pub id: Uuid,
    pub exercise_id: Uuid,
    pub created_at: DateTime<Utc>,
    /// Always `None` for a snapshot.
    pub score_given: Option<f32>,
    /// Always `None` for a snapshot.
    pub grading_progress: Option<GradingProgress>,
    /// `submission` from a host that predates the field, which lists only submissions.
    pub kind: SubmissionKind,
    /// Whether the entry has a live share link, whatever its kind. `false` from a host that
    /// predates the field.
    pub is_shared: bool,
}

/// What an entry in a submissions list is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub enum SubmissionKind {
    /// A graded submission.
    Submission,
    /// An ungraded snapshot stored to be shared, by `mooc paste`.
    Paste,
    /// An ungraded snapshot stored before local changes were discarded, by `--save-old-state`.
    Backup,
    /// A value newer than this client.
    Unrecognized,
}

impl From<api::SubmissionKind> for SubmissionKind {
    fn from(value: api::SubmissionKind) -> Self {
        match value {
            api::SubmissionKind::Submission => Self::Submission,
            api::SubmissionKind::Paste => Self::Paste,
            api::SubmissionKind::Backup => Self::Backup,
            api::SubmissionKind::Unrecognized => Self::Unrecognized,
        }
    }
}

impl From<api::ExerciseSlideSubmissionListItem> for ExerciseSlideSubmissionListItem {
    fn from(value: api::ExerciseSlideSubmissionListItem) -> Self {
        Self {
            id: value.id,
            exercise_id: value.exercise_id,
            created_at: value.created_at,
            score_given: value.score_given,
            grading_progress: value.grading_progress.map(Into::into),
            kind: value.kind.into(),
            is_shared: value.is_shared,
        }
    }
}

/// A shareable URL for a submission or snapshot.
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

    /// Pins the client-facing grading shape: internally tagged and kebab-cased
    /// like every other output kind, and without `feedback_json`, which only the
    /// exercise service that produced it can interpret.
    #[test]
    fn submission_status_is_internally_tagged_without_plugin_feedback() {
        let no_grading = serde_json::to_value(ExerciseTaskSubmissionStatus::NoGradingYet).unwrap();
        assert_eq!(
            no_grading,
            serde_json::json!({ "status": "no-grading-yet" })
        );

        let grading =
            ExerciseTaskSubmissionStatus::from(api::ExerciseTaskSubmissionStatus::Grading {
                grading_progress: api::GradingProgress::FullyGraded,
                score_given: Some(1.0),
                grading_started_at: None,
                grading_completed_at: None,
                feedback_json: Some(serde_json::json!({ "plugin_private": "sentinel" })),
                feedback_text: Some("All tests passed".to_string()),
                exercise_progress: Some(Box::new(api::ExerciseProgress {
                    exercise_id: Uuid::nil(),
                    score_given: 1.0,
                    score_maximum: 1,
                    completed: true,
                    attempted: true,
                    standing: Some(api::ExerciseStanding::Passed),
                    max_tries: Some(3),
                    tries_left: Some(2),
                    pending_teacher_review: Some(api::PendingTeacherReview::AwaitsChapterLock),
                    exercise_locked: false,
                    reviewing_stage: Some(api::ReviewingStage::PeerReview),
                    teacher_review: None,
                })),
                submission_page_url: Some("https://example.com/page#ex".to_string()),
            });
        let grading = serde_json::to_value(&grading).unwrap();

        assert_eq!(grading["status"], "grading");
        assert_eq!(grading["grading"]["grading_progress"], "FullyGraded");
        assert_eq!(grading["grading"]["score_given"], 1.0);
        assert_eq!(grading["grading"]["feedback_text"], "All tests passed");
        assert_eq!(grading["grading"]["exercise_progress"]["completed"], true);
        assert_eq!(
            grading["grading"]["exercise_progress"]["standing"],
            "Passed"
        );
        assert_eq!(grading["grading"]["exercise_progress"]["max_tries"], 3);
        assert_eq!(grading["grading"]["exercise_progress"]["tries_left"], 2);
        assert_eq!(
            grading["grading"]["exercise_progress"]["pending_teacher_review"],
            "awaits_chapter_lock"
        );
        assert_eq!(
            grading["grading"]["exercise_progress"]["exercise_locked"],
            false
        );
        assert_eq!(
            grading["grading"]["exercise_progress"]["reviewing_stage"],
            "peer_review"
        );
        assert_eq!(
            grading["grading"]["submission_page_url"],
            "https://example.com/page#ex"
        );
        assert!(
            !grading.to_string().contains("sentinel"),
            "plugin-private feedback must not be relayed: {grading}"
        );
        assert_eq!(grading["grading"]["test_results"], serde_json::Value::Null);
    }

    #[test]
    fn grading_relays_the_test_run_in_feedback_json() {
        let grading =
            ExerciseTaskSubmissionStatus::from(api::ExerciseTaskSubmissionStatus::Grading {
                grading_progress: api::GradingProgress::FullyGraded,
                score_given: Some(0.0),
                grading_started_at: None,
                grading_completed_at: None,
                feedback_json: Some(serde_json::json!({
                    "status": "TESTS_FAILED",
                    "testResults": [{
                        "name": "Test fails",
                        "successful": false,
                        "points": ["1.1"],
                        "message": "expected 1",
                        "exception": ["at Test"]
                    }],
                    "logs": { "stdout": "program output" }
                })),
                feedback_text: Some("Tests failed".to_string()),
                exercise_progress: None,
                submission_page_url: None,
            });
        let grading = serde_json::to_value(&grading).unwrap();

        assert_eq!(
            grading["grading"]["test_results"],
            serde_json::json!({
                "status": "TESTS_FAILED",
                "test_results": [{
                    "name": "Test fails",
                    "successful": false,
                    "points": ["1.1"],
                    "message": "expected 1",
                    "exception": ["at Test"]
                }],
                "logs": {}
            })
        );
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
        // mockito serves on 127.0.0.1, an untrusted host, so by default no bearer
        // is sent; the knob opts localhost in.
        init();
        let _env = env_lock();
        // SAFETY: all reads/writes of this var in tests are serialized by ENV_LOCK.
        unsafe { std::env::remove_var(TRUST_LOCALHOST_VAR) };

        let mut server = Server::new();
        let client = make_client_with_token(&server, "test-token");

        let no_auth = server
            .mock("GET", "/api/v0/exercise-services/client/courses")
            .match_header("authorization", Matcher::Missing)
            .with_body("[]")
            .expect(1)
            .create();
        client.courses().unwrap();
        no_auth.assert();

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
    fn a_redirect_to_another_origin_drops_the_bearer_token() {
        // `MoocClient::new` sets no redirect policy, so reqwest's default
        // `remove_sensitive_headers` alone drops `Authorization` when the scheme, host
        // or port changes. A file URL answers 302 to the object store, so a change of
        // that default would hand the store the token.
        init();
        let _env = env_lock();
        // SAFETY: all reads/writes of this var in tests are serialized by ENV_LOCK.
        unsafe { std::env::set_var(TRUST_LOCALHOST_VAR, "1") };

        let mut origin = Server::new();
        let mut elsewhere = Server::new();
        // Same host, different port: a different origin as far as reqwest is concerned.
        assert_ne!(origin.url(), elsewhere.url());

        let redirect = origin
            .mock("GET", "/api/v0/files/claimed/an-id")
            .match_header("authorization", "Bearer test-token")
            .with_status(302)
            .with_header("location", &format!("{}/objects/an-id", elsewhere.url()))
            .expect(1)
            .create();
        let followed = elsewhere
            .mock("GET", "/objects/an-id")
            .match_header("authorization", Matcher::Missing)
            .with_body("archive bytes")
            .expect(1)
            .create();

        let client = make_client_with_token(&origin, "test-token");
        let url = format!("{}/api/v0/files/claimed/an-id", origin.url());
        let downloaded = client.download(url.parse().unwrap());
        // Clear the var before any assertion that could panic and leak it.
        unsafe { std::env::remove_var(TRUST_LOCALHOST_VAR) };

        assert_eq!(&downloaded.unwrap()[..], b"archive bytes");
        redirect.assert();
        followed.assert();
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
    fn download_reports_a_missing_file() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        server
            .mock("GET", "/files/archive.tar.zst")
            .match_query(Matcher::Any)
            .with_status(404)
            .create();
        let url = format!("{}/files/archive.tar.zst?claim=secret", server.url())
            .parse()
            .unwrap();
        let err = client.download(url).unwrap_err();
        assert!(matches!(*err, MoocClientError::FileNotFound { .. }));
        let message = err.to_string();
        assert!(message.starts_with("The exercise's files are missing on courses.mooc.fi"));
        assert!(!message.contains("secret"), "{message}");
    }

    #[test]
    fn http_error_carries_the_backend_message() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let exercise_id = Uuid::new_v4();
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/exercises/{exercise_id}").as_str(),
            )
            .with_status(422)
            .with_body(
                serde_json::json!({
                    "type": "validation_error",
                    "message_key": "deadline_passed",
                    "message": "Exercise deadline passed."
                })
                .to_string(),
            )
            .create();
        let message = client.exercise(exercise_id).unwrap_err().to_string();
        assert!(
            message.starts_with("422 Unprocessable Entity from http"),
            "{message}"
        );
        assert!(
            message.ends_with(": Exercise deadline passed."),
            "{message}"
        );
        assert!(!message.contains("Obsolete"), "{message}");
    }

    #[test]
    fn http_error_with_an_empty_body_ends_at_the_url() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let exercise_id = Uuid::new_v4();
        let path = format!("/api/v0/exercise-services/client/exercises/{exercise_id}");
        server.mock("GET", path.as_str()).with_status(404).create();
        let message = client.exercise(exercise_id).unwrap_err().to_string();
        assert_eq!(
            message,
            format!("404 Not Found from {}{path}", server.url())
        );
    }

    /// Fetches a course whose body is the base course plus `extra_fields`.
    fn course_with(extra_fields: serde_json::Value) -> Course {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let course_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
        let mut body = serde_json::json!({
            "id": course_id,
            "slug": "mockslug",
            "name": "mockname",
            "description": "mockdesc",
            "organization_name": "mockorg",
        });
        body.as_object_mut()
            .unwrap()
            .extend(extra_fields.as_object().unwrap().clone());
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/courses/{course_id}").as_str(),
            )
            .with_body(body.to_string())
            .create();
        client.course(Uuid::parse_str(course_id).unwrap()).unwrap()
    }

    #[test]
    fn gets_single_course() {
        let course = course_with(serde_json::json!({ "ai_policy": "full_use" }));
        assert_eq!(course.name, "mockname");
        assert_eq!(course.organization_name, "mockorg");
        assert_eq!(course.ai_policy, CourseAiPolicy::FullUse);
    }

    #[test]
    fn course_without_ai_policy_reads_as_not_set() {
        let course = course_with(serde_json::json!({}));
        assert_eq!(course.ai_policy, CourseAiPolicy::NotSet);
    }

    #[test]
    fn course_with_unknown_ai_policy_reads_as_unrecognized() {
        let course = course_with(serde_json::json!({ "ai_policy": "some_future_policy" }));
        assert_eq!(course.ai_policy, CourseAiPolicy::Unrecognized);
    }

    #[test]
    fn course_ai_policy_wire_strings() {
        for (policy, wire) in [
            (CourseAiPolicy::NotSet, "not_set"),
            (CourseAiPolicy::NoAi, "no_ai"),
            (CourseAiPolicy::PlanningOnly, "planning_only"),
            (CourseAiPolicy::Limited, "limited"),
            (CourseAiPolicy::FullUse, "full_use"),
            (CourseAiPolicy::Required, "required"),
            (CourseAiPolicy::Unrecognized, "unrecognized"),
        ] {
            assert_eq!(serde_json::to_value(policy).unwrap(), wire);
            assert_eq!(
                serde_json::from_value::<CourseAiPolicy>(serde_json::json!(wire)).unwrap(),
                policy
            );
        }
    }

    #[test]
    fn pending_teacher_review_wire_strings() {
        for (review, wire) in [
            (
                PendingTeacherReview::AwaitsChapterLock,
                "awaits_chapter_lock",
            ),
            (PendingTeacherReview::AwaitsTeacher, "awaits_teacher"),
            (PendingTeacherReview::Unrecognized, "unrecognized"),
        ] {
            assert_eq!(serde_json::to_value(review).unwrap(), wire);
            assert_eq!(
                serde_json::from_value::<PendingTeacherReview>(serde_json::json!(wire)).unwrap(),
                review
            );
        }
    }

    /// The host's own type cannot serialize a value newer than its crate; ours must, since the
    /// CLI writes it out again.
    #[test]
    fn unknown_pending_teacher_review_from_the_host_serializes_as_unrecognized() {
        let progress: api::ExerciseProgress = serde_json::from_value(serde_json::json!({
            "exercise_id": "a1b2c3d4-0000-4000-8000-000000000001",
            "score_given": 0.0,
            "score_maximum": 1,
            "completed": true,
            "attempted": true,
            "pending_teacher_review": "some_future_review",
        }))
        .unwrap();
        let progress = serde_json::to_value(ExerciseProgress::from(progress)).unwrap();
        assert_eq!(progress["pending_teacher_review"], "unrecognized");
        assert_eq!(progress["exercise_locked"], false);
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
        assert_eq!(progress.exercises[1].score_given, 0.5);
        assert!(!progress.exercises[1].completed);
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
                    "data_files": [{
                        "id": FILE_UPLOAD_ID,
                        "name": "submission.tar.zst",
                        "mime": ANSWER_ARCHIVE_MIME,
                        "url": "http://example.com/archive.tar.zst",
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
    fn submits_a_file_answer_naming_the_stored_file_ids() {
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
                "answer_kind": "file",
                "data_files": [FILE_UPLOAD_ID],
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
            .with_body(serde_json::json!({ "data_files": [] }).to_string())
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
    fn upload_files_types_each_part_as_an_answer_archive() {
        // reqwest guesses from the path extension, and the archive `submit` uploads is
        // an extensionless temp file, so an untyped part would arrive as
        // `application/octet-stream`.
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let body = std::sync::Arc::new(Mutex::new(String::new()));
        let captured = body.clone();
        let upload = server
            .mock(
                "POST",
                format!("/api/v0/exercise-services/client/exercises/{EXERCISE_ID}/files").as_str(),
            )
            .match_request(move |request| {
                *captured.lock().unwrap_or_else(|e| e.into_inner()) =
                    String::from_utf8_lossy(request.body().map_or(&[][..], |b| &b[..])).to_string();
                true
            })
            .with_body(serde_json::json!({ "data_files": [] }).to_string())
            .create();

        client
            .upload_files(
                Uuid::parse_str(EXERCISE_ID).unwrap(),
                &[(SUBMISSION_ARCHIVE_FILE_NAME, Path::new("./tests/data/file"))],
            )
            .unwrap();

        upload.assert();
        let body = body.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let lowercased = body.to_ascii_lowercase();
        assert!(
            lowercased.contains(&format!("content-type: {ANSWER_ARCHIVE_MIME}")),
            "part must declare the answer archive type: {body:?}"
        );
        assert!(
            !lowercased.contains("application/octet-stream"),
            "no part may fall back to an unknown type: {body:?}"
        );
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
                    "data_files": [{
                        "id": FILE_UPLOAD_ID,
                        "name": "submission.tar.zst",
                        "mime": ANSWER_ARCHIVE_MIME,
                        "url": "http://example.com/archive.tar.zst",
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
        let snapshot_id = Uuid::new_v4();
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/exercises/{exercise_id}/submissions")
                    .as_str(),
            )
            .with_body(
                serde_json::json!([
                    {
                        "id": snapshot_id,
                        "exercise_id": exercise_id,
                        "created_at": "2026-07-22T00:00:00Z",
                        "score_given": null,
                        "grading_progress": null,
                        "kind": "paste",
                        "is_shared": true
                    },
                    {
                        "id": submission_id,
                        "exercise_id": exercise_id,
                        "created_at": "2026-07-21T00:00:00Z",
                        "score_given": 1.0,
                        "grading_progress": "FullyGraded"
                    }
                ])
                .to_string(),
            )
            .create();
        let submissions = client
            .get_exercise_submissions(Uuid::parse_str(exercise_id).unwrap())
            .unwrap();
        assert_eq!(submissions.len(), 2);
        assert_eq!(submissions[0].id, snapshot_id);
        assert_eq!(submissions[0].kind, SubmissionKind::Paste);
        assert!(submissions[0].is_shared);
        assert_eq!(submissions[1].id, submission_id);
        assert_eq!(submissions[1].score_given, Some(1.0));
        assert!(matches!(
            submissions[1].grading_progress,
            Some(GradingProgress::FullyGraded)
        ));
        // A host that predates `kind` and `is_shared` lists only unshared submissions.
        assert_eq!(submissions[1].kind, SubmissionKind::Submission);
        assert!(!submissions[1].is_shared);
        let json = serde_json::to_value(&submissions).unwrap();
        assert_eq!(json[0]["kind"], "paste");
        assert_eq!(json[1]["kind"], "submission");
    }

    /// Mocks `GET exercises/{id}` for [`EXERCISE_ID`] with one editor task [`TASK_ID`].
    fn mock_editor_exercise(server: &mut Server) -> mockito::Mock {
        server
            .mock(
                "GET",
                format!("/api/v0/exercise-services/client/exercises/{EXERCISE_ID}").as_str(),
            )
            .with_body(
                serde_json::json!({
                    "slide_id": SLIDE_ID,
                    "exercise_id": EXERCISE_ID,
                    "course_id": Uuid::new_v4(),
                    "exercise_name": "mockname",
                    "exercise_order_number": 0,
                    "tasks": [{
                        "task_id": TASK_ID,
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
            .create()
    }

    fn snapshots_path() -> String {
        format!("/api/v0/exercise-services/client/exercises/{EXERCISE_ID}/snapshots")
    }

    /// Mocks the `?for=snapshot` upload, returning one stored file with [`FILE_UPLOAD_ID`].
    fn mock_snapshot_upload(server: &mut Server) -> mockito::Mock {
        server
            .mock(
                "POST",
                format!("/api/v0/exercise-services/client/exercises/{EXERCISE_ID}/files").as_str(),
            )
            .match_query(Matcher::UrlEncoded("for".into(), "snapshot".into()))
            .with_body(
                serde_json::json!({
                    "data_files": [{
                        "id": FILE_UPLOAD_ID,
                        "name": "submission.tar.zst",
                        "mime": ANSWER_ARCHIVE_MIME,
                        "url": "http://example.com/archive.tar.zst",
                    }]
                })
                .to_string(),
            )
            .create()
    }

    #[test]
    fn stores_a_snapshot_from_a_snapshot_upload() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        let snapshot_id = Uuid::new_v4();
        mock_editor_exercise(&mut server);
        let upload = mock_snapshot_upload(&mut server);
        let snapshot = server
            .mock("POST", snapshots_path().as_str())
            .match_header("content-type", "application/json")
            .match_body(Matcher::Json(serde_json::json!({
                "purpose": "backup",
                "exercise_slide_id": SLIDE_ID,
                "exercise_task_id": TASK_ID,
                "data_files": [FILE_UPLOAD_ID],
            })))
            .with_body(serde_json::json!({ "snapshot_id": snapshot_id }).to_string())
            .create();

        let stored = client
            .store_snapshot(
                Uuid::parse_str(EXERCISE_ID).unwrap(),
                Path::new("./tests/data/file"),
                api::SnapshotPurpose::Backup,
            )
            .unwrap();

        assert_eq!(stored, snapshot_id);
        upload.assert();
        snapshot.assert();
    }

    #[test]
    fn store_snapshot_retries_the_upload_once_on_upload_expired() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        mock_editor_exercise(&mut server);
        let upload = mock_snapshot_upload(&mut server).expect(2);
        server
            .mock("POST", snapshots_path().as_str())
            .with_status(422)
            .with_body(upload_expired_body())
            .expect(1)
            .create();
        server
            .mock("POST", snapshots_path().as_str())
            .with_body(serde_json::json!({ "snapshot_id": Uuid::new_v4() }).to_string())
            .expect(1)
            .create();

        client
            .store_snapshot(
                Uuid::parse_str(EXERCISE_ID).unwrap(),
                Path::new("./tests/data/file"),
                api::SnapshotPurpose::Paste,
            )
            .unwrap();
        upload.assert();
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
            .with_body(serde_json::json!({ "data_files": files }).to_string())
            .create()
    }

    fn submission_file(name: &str, url: &str) -> serde_json::Value {
        serde_json::json!({
            "id": Uuid::new_v4(),
            "name": name,
            "mime": ANSWER_ARCHIVE_MIME,
            "url": url,
        })
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
        assert_eq!(files[1].url, "http://example.com/b.txt");
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
        // A 2xx with a non-JSON body passes `send_expect_bytes` (it only errors on
        // read failure) and must surface as `DeserializingResponse`.
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
        let _clone_a = client.clone();
        let _clone_b = client.clone();
        let token = Token::new(
            AccessToken::new("after-clone".to_string()),
            BasicTokenType::Bearer,
            EmptyExtraTokenFields {},
        );
        client.set_token(token);
    }

    #[test]
    fn set_token_on_clone_is_visible_to_that_clone() {
        // The token lives behind the shared `Arc`, so every clone observes it.
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
        // http to production is refused so the bearer can't go out in plaintext.
        init();
        match MoocClient::new("http://courses.mooc.fi/".parse().unwrap()) {
            Err(err) => assert!(matches!(*err, MoocClientError::InsecureScheme { .. })),
            Ok(_) => panic!("http://courses.mooc.fi should be rejected"),
        }
        assert!(MoocClient::new("https://courses.mooc.fi/".parse().unwrap()).is_ok());
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
