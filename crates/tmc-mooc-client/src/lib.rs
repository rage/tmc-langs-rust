#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Used to communicate with the Courses MOOC server. See the `MoocClient` struct for more details.

mod error;
mod exercise;

pub use self::exercise::{
    ExerciseFile, ModelSolutionSpec, PublicSpec, TmcExerciseSlide, TmcExerciseTask,
};
use crate::error::{MoocClientError, MoocClientResult};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use exercise::UserAnswer;
pub use mooc_langs_api as api;
use oauth2::TokenResponse;
use reqwest::{
    blocking::{
        multipart::{Form, Part},
        Client, RequestBuilder, Response,
    },
    Method, StatusCode,
};
use schemars::JsonSchema;
use serde::{de::DeserializeOwned, Serialize};
use std::{borrow::Cow, path::Path, sync::Arc};
use tmc_langs_util::{serialize, JsonError};
#[cfg(feature = "ts-rs")]
use ts_rs::TS;
use uuid::Uuid;

/// Client for accessing the Courses MOOC API.
/// Uses an `Arc` internally so it is cheap to clone.
#[derive(Clone)]
pub struct MoocClient(Arc<MoocClientInner>);

struct MoocClientInner {
    client: Client,
    root_addr: Cow<'static, str>,
    token: Option<api::Token>,
}

/// Non-API methods.
impl MoocClient {
    /// Creates a new client.
    pub fn new(addr: impl Into<Cow<'static, str>>) -> Self {
        Self(Arc::new(MoocClientInner {
            client: Client::new(),
            root_addr: addr.into(),
            token: None,
        }))
    }

    /// Helper for creating a request to an endpoint.
    /// # Panics
    /// If the root URL or endpoint are malformed.
    fn request(&self, method: Method, endpoint: &str) -> MoocRequest {
        let url = format!("{}/api/v0/langs/{endpoint}", self.0.root_addr);
        self.request_to_url(method, url)
    }

    fn request_to_url(&self, method: Method, url: String) -> MoocRequest {
        log::debug!("building a request to {url}");

        let trusted_urls = &["https://courses.mooc.fi/", "http://project-331.local/"];
        let include_bearer_token = trusted_urls.iter().any(|tu| url.starts_with(tu));
        let mut builder = self.0.client.request(method.clone(), url.clone());
        if let Some(token) = self.0.token.as_ref() {
            if include_bearer_token {
                log::debug!("setting bearer token");
                builder = builder.bearer_auth(token.access_token().secret());
            } else {
                log::debug!("leaving out bearer token due to untrusted url");
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
    pub fn course_instances(&self) -> MoocClientResult<Vec<CourseInstance>> {
        let res = self
            .request(Method::GET, "course-instances")
            .send_expect_json::<Vec<api::CourseInstance>>()?;
        Ok(res.into_iter().map(Into::into).collect())
    }

    pub fn course_instance_exercise_slides(
        &self,
        course_instance: Uuid,
    ) -> MoocClientResult<Vec<TmcExerciseSlide>> {
        let url = format!("course-instances/{course_instance}/exercises");
        let res = self
            .request(Method::GET, &url)
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
        let url = format!("exercises/{exercise}");
        let res = self
            .request(Method::GET, &url)
            .send_expect_json::<api::ExerciseSlide>()?
            .try_into()
            .map_err(|err: JsonError| MoocClientError::DeserializingResponse {
                url,
                error: err.into(),
            })?;
        Ok(res)
    }

    pub fn download(&self, url: String) -> MoocClientResult<Bytes> {
        let res = self.request_to_url(Method::GET, url).send_expect_bytes()?;
        Ok(res)
    }

    pub fn download_exercise(&self, exercise: Uuid) -> MoocClientResult<Bytes> {
        let res = self
            .request(Method::GET, &format!("exercises/{exercise}/download"))
            .send_expect_bytes()?;
        Ok(res)
    }

    pub fn submit(
        &self,
        exercise_id: Uuid,
        slide_id: Uuid,
        task_id: Uuid,
        archive: &Path,
    ) -> MoocClientResult<ExerciseTaskSubmissionResult> {
        // upload archive
        let metadata = api::UploadMetadata { slide_id, task_id };
        let metadata = serialize::to_json_vec(&metadata)?;
        let form = Form::new()
            .part(
                "metadata",
                Part::bytes(metadata)
                    .mime_str("application/json")
                    .expect("known to work"),
            )
            .file("file", archive)
            .map_err(|err| MoocClientError::AttachFileToForm { error: err.into() })?;
        let res = self
            .request(Method::POST, &format!("exercises/{exercise_id}/upload"))
            .multipart(form)
            .send_expect_json::<api::UploadResult>()?;

        // send submission
        let user_answer = UserAnswer::Editor {
            download_url: res.download_url,
        };
        let data_json = serialize::to_json_value(&user_answer)?;
        let exercise_slide_submission = api::ExerciseSlideSubmission {
            exercise_slide_id: slide_id,
            exercise_task_id: task_id,
            data_json,
        };
        let res = self
            .request(Method::POST, &format!("exercises/{exercise_id}/submit"))
            .json(&exercise_slide_submission)
            .send_expect_json::<api::ExerciseTaskSubmissionResult>()?;
        Ok(res.into())
    }

    pub fn get_submission_grading(
        &self,
        submission_id: Uuid,
    ) -> MoocClientResult<ExerciseTaskSubmissionStatus> {
        let res = self
            .request(Method::GET, &format!("submissions/{submission_id}/grading"))
            .send_expect_json::<api::ExerciseTaskSubmissionStatus>()?;
        Ok(res.into())
    }
}

/// Helper for creating and sending requests.
struct MoocRequest {
    url: String,
    method: Method,
    builder: RequestBuilder,
}

impl MoocRequest {
    fn json<T>(mut self, value: &T) -> Self
    where
        T: Serialize,
    {
        self.builder = self.builder.json(value);
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
                    StatusCode::UNAUTHORIZED => Err(MoocClientError::NotAuthenticated),
                    _other => {
                        let status = res.status();
                        let body =
                            res.text()
                                .map_err(|err| MoocClientError::ReadingResponseBody {
                                    method: self.method,
                                    url: self.url.clone(),
                                    error: Box::new(err),
                                })?;
                        Err(MoocClientError::ErrorResponseFromServer {
                            url: self.url,
                            status,
                            error: body,
                            obsolete_client: false,
                        })
                    }
                }
            }
            Err(err) => Err(MoocClientError::SendingRequest {
                method: self.method,
                url: self.url,
                error: Box::new(err),
            }),
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

#[derive(Debug, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct CourseInstance {
    pub id: Uuid,
    pub course_id: Uuid,
    pub course_slug: String,
    pub course_name: String,
    pub course_description: Option<String>,
    pub instance_name: Option<String>,
    pub instance_description: Option<String>,
}

impl From<api::CourseInstance> for CourseInstance {
    fn from(value: api::CourseInstance) -> Self {
        Self {
            id: value.id,
            course_id: value.course_id,
            course_slug: value.course_slug,
            course_name: value.course_name,
            course_description: value.course_description,
            instance_name: value.instance_name,
            instance_description: value.instance_description,
        }
    }
}

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
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

#[derive(Debug, Clone, Copy, Serialize)]
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
