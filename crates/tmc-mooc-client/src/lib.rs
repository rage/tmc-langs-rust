#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Used to communicate with the Courses MOOC server. See the `MoocClient` struct for more details.

mod error;

use crate::error::{MoocClientError, MoocClientResult};
use bytes::Bytes;
pub use mooc_langs_api::*;
use oauth2::TokenResponse;
use reqwest::{
    blocking::{Client, RequestBuilder, Response},
    Method,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{borrow::Cow, sync::Arc};
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
        log::debug!("building a request to {url}");
        let mut builder = self.0.client.request(method.clone(), url.clone());
        if let Some(token) = self.0.token.as_ref() {
            log::debug!("setting bearer token");
            builder = builder.bearer_auth(token.access_token().secret());
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

    pub fn course_exercises(&self, course: Uuid) -> MoocClientResult<Vec<Exercise>> {
        let res = self
            .request(Method::GET, &format!("courses/{course}/exercises"))
            .send_expect_json()?;
        Ok(res)
    }

    pub fn exercise(&self, exercise: Uuid) -> MoocClientResult<ExerciseSlide> {
        let res = self
            .request(Method::GET, &format!("exercises/{exercise}"))
            .send_expect_json()?;
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
        exercise_slide_submission: &ExerciseSlideSubmission,
    ) -> MoocClientResult<ExerciseSlideSubmissionResult> {
        let res = self
            .request(Method::POST, &format!("exercises/{exercise_id}"))
            .json(exercise_slide_submission)
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

    fn send(self) -> MoocClientResult<Response> {
        match self.builder.send() {
            Ok(res) => {
                if res.status().is_success() {
                    Ok(res)
                } else {
                    let status = res.status();
                    let body = res
                        .text()
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
            Err(err) => Err(MoocClientError::SendingRequest {
                method: self.method,
                url: self.url,
                error: Box::new(err),
            }),
        }
    }

    fn send_expect_text(self) -> MoocClientResult<String> {
        let method = self.method.clone();
        let url = self.url.clone();
        let res = self.send()?;
        let body = res
            .json()
            .map_err(|err| MoocClientError::ReadingResponseBody {
                method,
                url,
                error: Box::new(err),
            })?;
        Ok(body)
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
