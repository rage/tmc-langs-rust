#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Used to communicate with the Courses MOOC server. See the `MoocClient` struct for more details.

mod error;
mod exercise;

pub use self::exercise::{
    ExerciseFile, ModelSolutionSpec, PublicSpec, TmcExerciseSlide, TmcExerciseTask,
};
use crate::error::{MoocClientError, MoocClientResult};
use bytes::Bytes;
use exercise::UserAnswer;
pub use mooc_langs_api::*;
use oauth2::TokenResponse;
use reqwest::{
    blocking::{
        multipart::{Form, Part},
        Client, RequestBuilder, Response,
    },
    Method, StatusCode,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{borrow::Cow, path::Path, sync::Arc};
use tmc_langs_util::{serialize, JsonError};
use uuid::Uuid;

/// Client for accessing the Courses MOOC API.
/// Uses an `Arc` internally so it is cheap to clone.
#[derive(Clone)]
pub struct MoocClient(Arc<MoocClientInner>);

struct MoocClientInner {
    client: Client,
    root_addr: Cow<'static, str>,
    token: Option<Token>,
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

        let trusted_urls = &["https://courses.mooc.fi/", "http://project-331.local"];
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

    pub fn set_token(&mut self, token: Token) {
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
            .send_expect_json()?;
        Ok(res)
    }

    pub fn course_instance_exercise_slides(
        &self,
        course_instance: Uuid,
    ) -> MoocClientResult<Vec<TmcExerciseSlide>> {
        let url = format!("course-instances/{course_instance}/exercises");
        let res = self
            .request(Method::GET, &url)
            .send_expect_json::<Vec<ExerciseSlide>>()?
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
            .send_expect_json::<ExerciseSlide>()?
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
    ) -> MoocClientResult<ExerciseSlideSubmissionResult> {
        // upload archive
        let metadata = UploadMetadata { slide_id, task_id };
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
            .send_expect_json::<UploadResult>()?;

        // send submission
        let user_answer = UserAnswer::Editor {
            download_url: res.download_url,
        };
        let data_json = serialize::to_json_value(&user_answer)?;
        let exercise_slide_submission = ExerciseSlideSubmission {
            exercise_slide_id: slide_id,
            exercise_task_submissions: vec![ExerciseTaskSubmission {
                exercise_task_id: task_id,
                data_json,
            }],
        };
        let res = self
            .request(Method::POST, &format!("exercises/{exercise_id}/submit"))
            .json(&exercise_slide_submission)
            .send_expect_json()?;
        Ok(res)
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
