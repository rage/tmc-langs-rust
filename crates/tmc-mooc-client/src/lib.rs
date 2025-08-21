#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Used to communicate with the Courses MOOC server. See the `MoocClient` struct for more details.

mod error;
mod exercise;

pub use self::{
    error::{MoocClientError, MoocClientResult},
    exercise::{ExerciseFile, ModelSolutionSpec, PublicSpec, TmcExerciseSlide, TmcExerciseTask},
};
use bytes::Bytes;
use chrono::{DateTime, Utc};
pub use mooc_langs_api as api;
use oauth2::TokenResponse;
use reqwest::{
    Method, StatusCode,
    blocking::{
        Client, RequestBuilder, Response,
        multipart::{Form, Part},
    },
};
use schemars::JsonSchema;
use serde::{Serialize, de::DeserializeOwned};
use std::{path::Path, sync::Arc};
use tmc_langs_util::{JsonError, serialize};
#[cfg(feature = "ts-rs")]
use ts_rs::TS;
use url::Url;
use uuid::Uuid;

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

        let trusted_domains = &["courses.mooc.fi", "project-331.local"];
        let is_trusted_domain = url
            .domain()
            .map(|d| trusted_domains.contains(&d))
            .unwrap_or_default();
        let mut builder = self.0.client.request(method.clone(), url.clone());
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
    pub fn course_instance(&self, instance_id: Uuid) -> MoocClientResult<CourseInstance> {
        todo!()
    }

    pub fn course_instances(&self) -> MoocClientResult<Vec<CourseInstance>> {
        let url = make_langs_api_url(self, "course-instances")?;
        let res = self
            .request(Method::GET, url)
            .send_expect_json::<Vec<api::CourseInstance>>()?;
        Ok(res.into_iter().map(Into::into).collect())
    }

    pub fn course_instance_exercises(
        &self,
        course_instance: Uuid,
    ) -> MoocClientResult<Vec<TmcExerciseSlide>> {
        let url = make_langs_api_url(
            self,
            format!("course-instances/{course_instance}/exercises"),
        )?;
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
        let url = make_langs_api_url(self, format!("exercises/{exercise}"))?;
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

    pub fn download_exercise(&self, exercise: Uuid) -> MoocClientResult<Bytes> {
        let url = make_langs_api_url(self, format!("exercises/{exercise}/download"))?;
        let res = self.request(Method::GET, url).send_expect_bytes()?;
        Ok(res)
    }

    pub fn submit(
        &self,
        exercise_id: Uuid,
        slide_id: Uuid,
        task_id: Uuid,
        archive: &Path,
    ) -> MoocClientResult<ExerciseTaskSubmissionResult> {
        let exercise_slide_submission = api::ExerciseSlideSubmission {
            exercise_slide_id: slide_id,
            exercise_task_id: task_id,
            data_json: serde_json::Value::Null,
        };
        let exercise_slide_submission = serialize::to_json_vec(&exercise_slide_submission)
            .map_err(Into::into)
            .map_err(Box::new)?;
        let submission = Form::new()
            .part(
                "metadata",
                Part::bytes(exercise_slide_submission)
                    .mime_str("application/json")
                    .expect("known to work"),
            )
            .file("file", archive)
            .map_err(|err| MoocClientError::AttachFileToForm { error: err.into() })?;

        // send submission
        //let user_answer = UserAnswer::Editor {
        //archive_download_url: res.download_url,
        //};
        //let data_json = serialize::to_json_value(&user_answer)?;
        //let exercise_slide_submission = api::ExerciseSlideSubmission {
        //exercise_slide_id: slide_id,
        //exercise_task_id: task_id,
        //data_json,
        //};
        let url = make_langs_api_url(self, format!("exercises/{exercise_id}/submit"))?;
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
        let url = make_langs_api_url(self, format!("submissions/{submission_id}/grading"))?;
        let res = self
            .request(Method::GET, url)
            .send_expect_json::<api::ExerciseTaskSubmissionStatus>()?;
        Ok(res.into())
    }

    pub fn get_exercises(&self, exercise_ids: &[Uuid]) -> MoocClientResult<Vec<TmcExerciseTask>> {
        todo!()
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
                        let body =
                            res.text()
                                .map_err(|err| MoocClientError::ReadingResponseBody {
                                    method: self.method,
                                    url: self.url.clone(),
                                    error: Box::new(err),
                                })?;
                        Err(Box::new(MoocClientError::HttpError {
                            url: self.url,
                            status,
                            error: body,
                            obsolete_client: false,
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

// joins the URL "tail" with the API url root from the client
fn make_langs_api_url(client: &MoocClient, tail: impl AsRef<str>) -> MoocClientResult<Url> {
    client
        .0
        .root_url
        .join("/api/v0/langs/")
        .and_then(|u| u.join(tail.as_ref()))
        .map_err(|e| MoocClientError::UrlParse(tail.as_ref().to_string(), e))
        .map_err(Box::new)
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

#[derive(Debug, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct CourseInstanceInfo {
    pub id: Uuid,
    pub course_id: Uuid,
    pub course_slug: String,
    pub course_name: String,
    pub course_description: Option<String>,
    pub instance_name: Option<String>,
    pub instance_description: Option<String>,
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

#[derive(Debug, Serialize)]
pub struct ExerciseUpdates {
    pub updated_exercises: Vec<Uuid>,
    pub deleted_exercises: Vec<Uuid>,
}

impl From<api::ExerciseUpdates> for ExerciseUpdates {
    fn from(value: api::ExerciseUpdates) -> Self {
        Self {
            updated_exercises: value.updated_exercises,
            deleted_exercises: value.deleted_exercises,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use mockito::Server;
    use mooc_langs_api::Token;
    use oauth2::{AccessToken, EmptyExtraTokenFields, basic::BasicTokenType};

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

    #[test]
    fn gets_course_instances() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        server
            .mock("GET", "/api/v0/langs/course-instances")
            .with_body(
                serde_json::json!([{
                    "id": Uuid::new_v4(),
                    "course_id": Uuid::new_v4(),
                    "course_slug": "mockslug",
                    "course_name": "mockname",
                    "course_description": "mockdesc",
                }])
                .to_string(),
            )
            .create();
        let course_instances = client.course_instances().unwrap();
        assert_eq!(course_instances[0].course_name, "mockname");
    }

    #[test]
    fn gets_course_instance_exercise_slides() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        server
            .mock(
                "GET",
                "/api/v0/langs/course-instances/df5ee6c1-57d1-43b6-b39e-5d72119edb5f/exercises",
            )
            .with_body(
                serde_json::json!([{
                    "slide_id": Uuid::new_v4(),
                    "exercise_id": Uuid::new_v4(),
                    "exercise_name": "mockname",
                    "exercise_order_number": 0,
                    "tasks": [],
                }])
                .to_string(),
            )
            .create();
        let exercise_sludes = client
            .course_instance_exercises(
                Uuid::parse_str("df5ee6c1-57d1-43b6-b39e-5d72119edb5f").unwrap(),
            )
            .unwrap();
        assert_eq!(exercise_sludes[0].exercise_name, "mockname");
    }

    #[test]
    fn gets_exercise() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        server
            .mock(
                "GET",
                "/api/v0/langs/exercises/df5ee6c1-57d1-43b6-b39e-5d72119edb5f",
            )
            .with_body(
                serde_json::json!({
                    "slide_id": Uuid::new_v4(),
                    "exercise_id": Uuid::new_v4(),
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
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        server
            .mock(
                "GET",
                "/api/v0/langs/exercises/df5ee6c1-57d1-43b6-b39e-5d72119edb5f/download",
            )
            .with_body_from_file("./tests/data/file")
            .create();
        let exercise = client
            .download_exercise(Uuid::parse_str("df5ee6c1-57d1-43b6-b39e-5d72119edb5f").unwrap())
            .unwrap();
        assert_eq!(String::from_utf8(exercise.into()).unwrap(), "hello!");
    }

    #[test]
    fn submits() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        server
            .mock(
                "POST",
                "/api/v0/langs/exercises/df5ee6c1-57d1-43b6-b39e-5d72119edb5f/submit",
            )
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
    fn gets_submission_grading() {
        init();
        let mut server = Server::new();
        let client = make_client(&server);
        server
            .mock(
                "GET",
                "/api/v0/langs/submissions/df5ee6c1-57d1-43b6-b39e-5d72119edb5f/grading",
            )
            .with_body(serde_json::json!("NoGradingYet").to_string())
            .create();
        server
            .mock(
                "GET",
                "/api/v0/langs/submissions/e7bd5a07-1b83-4c97-91f2-e48cccf66b2a/grading",
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
}
