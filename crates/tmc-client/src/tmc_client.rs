//! Contains the TmcClient struct for communicating with the TMC server.

pub mod api_v8;

use self::api_v8::{PasteData, ReviewData};
use crate::{error::ClientError, request::*, response::*};
use oauth2::{
    basic::BasicClient, AuthUrl, ClientId, ClientSecret, ResourceOwnerPassword,
    ResourceOwnerUsername, TokenUrl,
};
use reqwest::{blocking::Client, Url};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::Duration,
    u32,
};
use tmc_langs_plugins::{Compression, Language};
use tmc_langs_util::progress_reporter;

/// Authentication token.
pub type Token =
    oauth2::StandardTokenResponse<oauth2::EmptyExtraTokenFields, oauth2::basic::BasicTokenType>;

/// The update data type for the progress reporter.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "client-update-data-kind")]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub enum ClientUpdateData {
    ExerciseDownload { id: u32, path: PathBuf },
    PostedSubmission(NewSubmission),
}

/// Updated exercises.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[cfg_attr(feature = "ts-rs", derive(ts_rs::TS))]
pub struct UpdateResult {
    pub created: Vec<Exercise>,
    pub updated: Vec<Exercise>,
}

/// A struct for interacting with the TestMyCode service, including authentication.
#[derive(Clone)]
pub struct TmcClient(Arc<TmcCore>);

struct TmcCore {
    client: Client,
    root_url: Url,
    token: Option<Token>,
    client_name: String,
    client_version: String,
}

// TODO: cache API results?
impl TmcClient {
    /// Convenience function for checking authentication.
    pub fn require_authentication(&self) -> Result<(), ClientError> {
        if self.0.token.is_some() {
            Ok(())
        } else {
            Err(ClientError::NotAuthenticated)
        }
    }

    /// Creates a new TmcClient with the given config directory and root URL.
    ///
    /// # Panics
    /// If the root URL does not have a trailing slash and is not a valid URL with an appended trailing slash.
    ///
    /// # Examples
    /// ```rust,no_run
    /// use tmc_client::TmcClient;
    ///
    /// let client = TmcClient::new("https://tmc.mooc.fi".parse().unwrap(), "some_client".to_string(), "some_version".to_string());
    /// ```
    pub fn new(root_url: Url, client_name: String, client_version: String) -> Self {
        // guarantee a trailing slash, otherwise join will drop the last component
        let root_url = if root_url.as_str().ends_with('/') {
            root_url
        } else {
            format!("{root_url}/").parse().expect("invalid root url")
        };

        TmcClient(Arc::new(TmcCore {
            client: Client::new(),
            root_url,
            token: None,
            client_name,
            client_version,
        }))
    }

    /// Sets the authentication token, which may for example have been read from a file.
    ///
    /// # Panics
    /// If called when multiple clones of the client exist. Call this function before cloning.
    pub fn set_token(&mut self, token: Token) {
        Arc::get_mut(&mut self.0)
            .expect("called when multiple clones exist")
            .token = Some(token);
    }

    /// Attempts to log in with the given credentials, returns an error if an authentication token is already present.
    /// Username can be the user's username or email.
    ///
    /// # Errors
    /// This function will return an error if the client has already been authenticated,
    /// if the client_name is malformed and leads to a malformed URL,
    /// or if there is some error during the token exchange (see oauth2::Client::excange_password).
    ///
    /// # Panics
    /// If called when multiple clones exist. Call this function before cloning.
    ///
    /// # Examples
    /// ```rust,no_run
    /// use tmc_client::TmcClient;
    ///
    /// let mut client = TmcClient::new("https://tmc.mooc.fi".parse().unwrap(), "some_client".to_string(), "some_version".to_string());
    /// client.authenticate("client", "user".to_string(), "pass".to_string()).unwrap();
    /// ```
    pub fn authenticate(
        &mut self,
        client_name: &str,
        email: String,
        password: String,
    ) -> Result<Token, ClientError> {
        if self.0.token.is_some() {
            return Err(ClientError::AlreadyAuthenticated);
        }

        let auth_url = self
            .0
            .root_url
            .join("/oauth/token")
            .map_err(|e| ClientError::UrlParse("oauth/token".to_string(), e))?;

        let credentials = api_v8::get_credentials(self, client_name)?;

        log::debug!("authenticating at {}", auth_url);
        let client = BasicClient::new(
            ClientId::new(credentials.application_id),
            Some(ClientSecret::new(credentials.secret)),
            AuthUrl::from_url(auth_url.clone()),
            Some(TokenUrl::from_url(auth_url)),
        );

        let token = client
            .exchange_password(
                &ResourceOwnerUsername::new(email),
                &ResourceOwnerPassword::new(password),
            )
            .request(oauth2::reqwest::http_client)?;
        Arc::get_mut(&mut self.0)
            .expect("called when multiple clones exist")
            .token = Some(token.clone());
        log::debug!("authenticated");
        Ok(token)
    }

    /// Fetches all organizations. Does not require authentication.
    ///
    /// # Errors
    /// Returns an error if there's some problem reaching the API, or if the API returns an error.
    pub fn get_organizations(&self) -> Result<Vec<Organization>, ClientError> {
        api_v8::organization::get_organizations(self)
    }

    /// Fetches an organization. Does not require authentication.
    ///
    /// # Errors
    /// Returns an error if there's some problem reaching the API, or if the API returns an error.
    pub fn get_organization(&self, organization_slug: &str) -> Result<Organization, ClientError> {
        api_v8::organization::get_organization(self, organization_slug)
    }

    /// Fetches the course's information. Requires authentication.
    ///
    /// # Errors
    /// If not authenticated, or if there's some problem reaching the API, or if the API returns an error.
    pub fn get_course_details(&self, course_id: u32) -> Result<CourseDetails, ClientError> {
        self.require_authentication()?;
        api_v8::core::get_course(self, course_id)
    }

    /// Fetches the exercise's details. Requires authentication.
    ///
    /// # Errors
    /// If not authenticated, or if there's some problem reaching the API, or if the API returns an error.
    pub fn get_exercise_details(&self, exercise_id: u32) -> Result<ExerciseDetails, ClientError> {
        self.require_authentication()?;
        api_v8::core::get_exercise(self, exercise_id)
    }

    /// Fetches the course's information. Does not require authentication.
    ///
    /// # Errors
    /// If there's some problem reaching the API, or if the API returns an error.
    pub fn get_exercises_details(
        &self,
        exercise_ids: &[u32],
    ) -> Result<Vec<ExercisesDetails>, ClientError> {
        api_v8::core::get_exercise_details(self, exercise_ids)
    }

    /// Fetches all courses under the given organization. Requires authentication.
    ///
    /// # Errors
    /// If not authenticated, there's some problem reaching the API, or if the API returns an error.
    pub fn list_courses(&self, organization_slug: &str) -> Result<Vec<Course>, ClientError> {
        self.require_authentication()?;
        api_v8::core::get_organization_courses(self, organization_slug)
    }

    /// Fetches the given course's data. Requires authentication.
    ///
    /// # Errors
    /// If not authenticated, there's some problem reaching the API, or if the API returns an error.
    pub fn get_course(&self, course_id: u32) -> Result<CourseData, ClientError> {
        self.require_authentication()?;
        api_v8::course::get_by_id(self, course_id)
    }

    /// Fetches the given course's exercises. Requires authentication.
    ///
    /// # Errors
    /// If not authenticated, there's some problem reaching the API, or if the API returns an error.
    pub fn get_course_exercises(&self, course_id: u32) -> Result<Vec<CourseExercise>, ClientError> {
        self.require_authentication()?;
        api_v8::exercise::get_course_exercises_by_id(self, course_id)
    }

    /// Sends the given submission as a paste. Requires authentication.
    ///
    /// # Errors
    /// If not authenticated, there's some problem reaching the API, or if the API returns an error.
    ///
    /// # Examples
    /// ```rust,no_run
    /// use tmc_client::{TmcClient, Language};
    /// use url::Url;
    /// use std::path::Path;
    ///
    /// let client = TmcClient::new("https://tmc.mooc.fi".parse().unwrap(), "some_client".to_string(), "some_version".to_string());
    /// // authenticate
    /// let new_submission = client.paste(
    ///     123,
    ///     Path::new("./exercises/python/123"),
    ///     Some("my python solution".to_string()),
    ///     Some(Language::Eng)).unwrap();
    /// ```
    pub fn paste(
        &self,
        exercise_id: u32,
        submission_path: &Path,
        paste_message: Option<String>,
        locale: Option<Language>,
    ) -> Result<NewSubmission, ClientError> {
        self.require_authentication()?;

        // compress
        start_stage(2, "Compressing paste submission...", None);
        let compressed =
            tmc_langs_plugins::compress_project(submission_path, Compression::Zip, false)?;
        progress_stage(
            "Compressed paste submission. Posting paste submission...",
            None,
        );

        let paste = if let Some(message) = paste_message {
            PasteData::WithMessage(message)
        } else {
            PasteData::WithoutMessage
        };

        let result = api_v8::core::submit_exercise(
            self,
            exercise_id,
            Cursor::new(compressed),
            Some(paste),
            None,
            locale,
        )?;

        finish_stage(
            format!("Paste finished, running at {0}", result.paste_url),
            ClientUpdateData::PostedSubmission(result.clone()),
        );
        Ok(result)
    }

    /// Sends feedback. Requires authentication.
    ///
    /// # Errors
    /// If not authenticated, there's some problem reaching the API, or if the API returns an error.
    pub fn send_feedback(
        &self,
        submission_id: u32,
        feedback: Vec<FeedbackAnswer>,
    ) -> Result<SubmissionFeedbackResponse, ClientError> {
        self.require_authentication()?;
        api_v8::core::post_submission_feedback(self, submission_id, feedback)
    }

    /// Posts feedback to the given URL. Requires authentication.
    ///
    /// # Errors
    /// If not authenticated, there's some problem reaching the API, or if the API returns an error.
    pub fn send_feedback_to_url(
        &self,
        feedback_url: Url,
        feedback: Vec<FeedbackAnswer>,
    ) -> Result<SubmissionFeedbackResponse, ClientError> {
        self.require_authentication()?;
        let form = api_v8::prepare_feedback_form(feedback);
        api_v8::post_form(self, feedback_url, &form)
    }

    /// Sends the submission to the server. Requires authentication.
    ///
    /// # Errors
    /// If not authenticated, there's some problem reaching the API, or if the API returns an error.
    pub fn submit(
        &self,
        exercise_id: u32,
        submission_path: &Path,
        locale: Option<Language>,
    ) -> Result<NewSubmission, ClientError> {
        self.require_authentication()?;

        start_stage(2, "Compressing submission...", None);
        let compressed =
            tmc_langs_plugins::compress_project(submission_path, Compression::Zip, false)?;
        progress_stage("Compressed submission. Posting submission...", None);

        let result = api_v8::core::submit_exercise(
            self,
            exercise_id,
            Cursor::new(compressed),
            None,
            None,
            locale,
        )?;
        finish_stage(
            format!(
                "Submission finished, running at {0}",
                result.show_submission_url
            ),
            ClientUpdateData::PostedSubmission(result.clone()),
        );
        Ok(result)
    }

    /// Downloads an old submission. Requires authentication.
    ///
    /// # Errors
    /// If not authenticated, there's some problem reaching the API, or if the API returns an error.
    pub fn download_old_submission(
        &self,
        submission_id: u32,
        target: impl Write,
    ) -> Result<(), ClientError> {
        self.require_authentication()?;
        log::info!("downloading old submission {}", submission_id);
        api_v8::core::download_submission(self, submission_id, target)
    }

    /// Fetches exercise submissions for the authenticated user. Requires authentication.
    ///
    /// # Errors
    /// If not authenticated, there's some problem reaching the API, or if the API returns an error.
    pub fn get_exercise_submissions_for_current_user(
        &self,
        exercise_id: u32,
    ) -> Result<Vec<Submission>, ClientError> {
        self.require_authentication()?;
        api_v8::submission::get_exercise_submissions_for_current_user(self, exercise_id)
    }

    // abstracts waiting for submission over different functions for getting the submission status
    fn wait_for_submission_inner(
        &self,
        f: impl Fn() -> Result<SubmissionProcessingStatus, ClientError>,
    ) -> Result<SubmissionFinished, ClientError> {
        start_stage(4, "Waiting for submission", None);

        let mut previous_status = None;
        loop {
            match f()? {
                SubmissionProcessingStatus::Finished(f) => {
                    finish_stage("Submission finished processing!", None);
                    return Ok(*f);
                }
                SubmissionProcessingStatus::Processing(p) => {
                    if p.status == SubmissionStatus::Hidden {
                        // hidden status, return constructed status
                        finish_stage("Submission status hidden, stopping waiting.", None);
                        let finished = SubmissionFinished {
                            api_version: 8,
                            all_tests_passed: Some(true),
                            user_id: 0,
                            login: "0".to_string(),
                            course: "0".to_string(),
                            exercise_name: "string".to_string(),
                            status: SubmissionStatus::Hidden,
                            points: vec![],
                            validations: None,
                            valgrind: None,
                            submission_url: "".to_string(),
                            solution_url: None,
                            submitted_at: "string".to_string(),
                            processing_time: None,
                            reviewed: false,
                            requests_review: false,
                            paste_url: None,
                            message_for_paste: None,
                            missing_review_points: vec![],
                            test_cases: Some(vec![TestCase {
                                name: "Hidden Exam Test: hidden_test".to_string(),
                                successful: true,
                                message: Some("Exam exercise sent to server successfully, you can now continue.".to_string()),
                                exception: None,
                                detailed_message: None,
                            }]),
                            error: None,
                            feedback_answer_url: None,
                            feedback_questions: None,
                        };
                        return Ok(finished);
                    }

                    match (&mut previous_status, p.sandbox_status) {
                        (Some(previous), status) if status == *previous => {} // no change, ignore
                        (_, status) => {
                            // new status, update progress
                            match status {
                                SandboxStatus::Created => {
                                    progress_stage("Created on sandbox", None)
                                }
                                SandboxStatus::SendingToSandbox => {
                                    progress_stage("Sending to sandbox", None);
                                }
                                SandboxStatus::ProcessingOnSandbox => {
                                    progress_stage("Processing on sandbox", None);
                                }
                            }
                            previous_status = Some(status);
                        }
                    }
                    thread::sleep(Duration::from_secs(1));
                }
            }
        }
    }

    /// Waits for a submission to finish at the given URL. May require authentication
    ///
    /// # Errors
    /// If authentication is required but the client is not authenticated, there's some problem reaching the API, or if the API returns an error.
    pub fn wait_for_submission_at(
        &self,
        submission_url: Url,
    ) -> Result<SubmissionFinished, ClientError> {
        self.wait_for_submission_inner(|| api_v8::get_json(self, submission_url.clone(), &[]))
    }

    /// Waits for a submission to finish. Requires authentication.
    ///
    /// # Errors
    /// If not authenticated, there's some problem reaching the API, or if the API returns an error.
    pub fn wait_for_submission(
        &self,
        submission_id: u32,
    ) -> Result<SubmissionFinished, ClientError> {
        self.wait_for_submission_inner(|| api_v8::get_submission(self, submission_id))
    }

    /// Fetches the course's exercises from the server,
    /// and finds new or updated exercises. Requires authentication.
    /// If an exercise's id is not found in the checksum map, it is considered new.
    /// If an id is found, it is compared to the current one. If they are different,
    /// it is considered updated.
    ///
    /// # Errors
    /// If not authenticated, there's some problem reaching the API, or if the API returns an error.
    ///
    /// # Examples
    /// ```rust,no_run
    /// use tmc_client::TmcClient;
    ///
    /// let client = TmcClient::new("https://tmc.mooc.fi".parse().unwrap(), "some_client".to_string(), "some_version".to_string());
    /// // authenticate
    /// let mut checksums = std::collections::HashMap::new();
    /// checksums.insert(1234, "exercisechecksum".to_string());
    /// let update_result = client.get_exercise_updates(600, checksums).unwrap();
    /// ```
    pub fn get_exercise_updates(
        &self,
        course_id: u32,
        checksums: HashMap<u32, String>,
    ) -> Result<UpdateResult, ClientError> {
        self.require_authentication()?;

        let mut new_exercises = vec![];
        let mut updated_exercises = vec![];

        let course = self.get_course_details(course_id)?;
        for exercise in course.exercises {
            if let Some(old_checksum) = checksums.get(&exercise.id) {
                if &exercise.checksum != old_checksum {
                    // updated
                    updated_exercises.push(exercise);
                }
            } else {
                // new
                new_exercises.push(exercise);
            }
        }
        Ok(UpdateResult {
            created: new_exercises,
            updated: updated_exercises,
        })
    }

    /// Mark the review as read on the server. Requires authentication.
    ///
    /// # Errors
    /// If not authenticated, there's some problem reaching the API, or if the API returns an error.
    pub fn mark_review_as_read(&self, course_id: u32, review_id: u32) -> Result<(), ClientError> {
        self.require_authentication()?;
        api_v8::core::update_course_review(self, course_id, review_id, None, Some(true))
    }

    /// Fetches unread reviews. Requires authentication.
    ///
    /// # Errors
    /// If not authenticated, there's some problem reaching the API, or if the API returns an error.
    pub fn get_unread_reviews(&self, course_id: u32) -> Result<Vec<Review>, ClientError> {
        self.require_authentication()?;
        api_v8::core::get_course_reviews(self, course_id)
    }

    /// Request code review. Requires authentication.
    ///
    /// # Errors
    /// If not authenticated, there's some problem reaching the API, or if the API returns an error.
    pub fn request_code_review(
        &self,
        exercise_id: u32,
        submission_path: &Path,
        message_for_reviewer: Option<String>,
        locale: Option<Language>,
    ) -> Result<NewSubmission, ClientError> {
        self.require_authentication()?;

        let compressed =
            tmc_langs_plugins::compress_project(submission_path, Compression::Zip, false)?;
        let review = if let Some(message) = message_for_reviewer {
            ReviewData::WithMessage(message)
        } else {
            ReviewData::WithoutMessage
        };
        api_v8::core::submit_exercise(
            self,
            exercise_id,
            Cursor::new(compressed),
            None,
            Some(review),
            locale,
        )
    }

    /// Downloads the model solution from the given url. Requires authentication.
    ///
    /// # Errors
    /// If not authenticated, there's some problem reaching the API, or if the API returns an error.
    /// The method extracts the downloaded model solution archive, which may fail.
    pub fn download_model_solution(
        &self,
        exercise_id: u32,
        target: &Path,
    ) -> Result<(), ClientError> {
        self.require_authentication()?;

        let mut buf = vec![];
        api_v8::core::download_exercise_solution(self, exercise_id, &mut buf)?;
        tmc_langs_plugins::extract_project(Cursor::new(buf), target, Compression::Zip, false)?;
        Ok(())
    }

    /// Request code review. Requires authentication.
    ///
    /// # Errors
    /// If not authenticated, there's some problem reaching the API, or if the API returns an error.
    pub fn download_exercise(
        &self,
        exercise_id: u32,
        target: impl Write,
    ) -> Result<(), ClientError> {
        self.require_authentication()?;
        api_v8::core::download_exercise(self, exercise_id, target)
    }
}

// convenience functions to make sure the progress report type is correct for tmc-client
fn start_stage(steps: u32, message: impl Into<String>, data: impl Into<Option<ClientUpdateData>>) {
    progress_reporter::start_stage(steps, message.into(), data.into())
}

fn progress_stage(message: impl Into<String>, data: impl Into<Option<ClientUpdateData>>) {
    progress_reporter::progress_stage(message.into(), data.into())
}

fn finish_stage(message: impl Into<String>, data: impl Into<Option<ClientUpdateData>>) {
    progress_reporter::finish_stage(message.into(), data.into())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    // many of TmcClient's functions simply call already tested functions from api_v8 and don't need testing
    use super::*;
    use mockito::Matcher;
    use oauth2::{basic::BasicTokenType, AccessToken, EmptyExtraTokenFields};
    use std::sync::atomic::AtomicBool;

    // sets up mock-authenticated TmcClient and logging
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

    fn make_client() -> TmcClient {
        let mut client = TmcClient::new(
            mockito::server_url().parse().unwrap(),
            "some_client".to_string(),
            "some_ver".to_string(),
        );
        let token = Token::new(
            AccessToken::new("".to_string()),
            BasicTokenType::Bearer,
            EmptyExtraTokenFields {},
        );
        client.set_token(token);
        client
    }

    #[test]
    fn gets_exercise_updates() {
        init();
        let client = make_client();
        let _m = mockito::mock("GET", "/api/v8/core/courses/1234")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("client".into(), "some_client".into()),
                Matcher::UrlEncoded("client_version".into(), "some_ver".into()),
            ]))
            .with_body(serde_json::json!({
                "course": {
                    "id": 588,
                    "name": "mooc-2020-ohjelmointi",
                    "title": "Ohjelmoinnin MOOC 2020, Ohjelmoinnin perusteet",
                    "description": "Aikataulutettu Ohjelmoinnin MOOC 2020. Kurssin ensimmäinen puolisko. Tästä kurssista voi hakea opinto-oikeutta Helsingin yliopiston tietojenkäsittelytieteen osastolle.",
                    "details_url": "https://tmc.mooc.fi/api/v8/core/courses/588",
                    "unlock_url": "https://tmc.mooc.fi/api/v8/core/courses/588/unlock",
                    "reviews_url": "https://tmc.mooc.fi/api/v8/core/courses/588/reviews",
                    "comet_url": "https://tmc.mooc.fi:8443/comet",
                    "spyware_urls": [
                    "http://snapshots01.mooc.fi/"
                    ],
                    "unlockables": [],
                    "exercises": [
                    {
                        "id": 12,
                        "name": "unchanged",
                        "locked": false,
                        "deadline_description": "2020-01-20 23:59:59 +0200",
                        "deadline": "2020-01-20T23:59:59.999+02:00",
                        "soft_deadline": null,
                        "soft_deadline_description": null,
                        "checksum": "ab",
                        "return_url": "https://tmc.mooc.fi/api/v8/core/exercises/81842/submissions",
                        "zip_url": "https://tmc.mooc.fi/api/v8/core/exercises/81842/download",
                        "returnable": true,
                        "requires_review": false,
                        "attempted": false,
                        "completed": false,
                        "reviewed": false,
                        "all_review_points_given": true,
                        "memory_limit": null,
                        "runtime_params": [],
                        "valgrind_strategy": "fail",
                        "code_review_requests_enabled": false,
                        "run_tests_locally_action_enabled": true
                    },
                    {
                        "id": 23,
                        "name": "updated",
                        "locked": false,
                        "deadline_description": "2020-01-20 23:59:59 +0200",
                        "deadline": "2020-01-20T23:59:59.999+02:00",
                        "soft_deadline": null,
                        "soft_deadline_description": null,
                        "checksum": "zz",
                        "return_url": "https://tmc.mooc.fi/api/v8/core/exercises/81842/submissions",
                        "zip_url": "https://tmc.mooc.fi/api/v8/core/exercises/81842/download",
                        "returnable": true,
                        "requires_review": false,
                        "attempted": false,
                        "completed": false,
                        "reviewed": false,
                        "all_review_points_given": true,
                        "memory_limit": null,
                        "runtime_params": [],
                        "valgrind_strategy": "fail",
                        "code_review_requests_enabled": false,
                        "run_tests_locally_action_enabled": true
                    },
                    {
                        "id": 34,
                        "name": "new",
                        "locked": false,
                        "deadline_description": "2020-01-20 23:59:59 +0200",
                        "deadline": "2020-01-20T23:59:59.999+02:00",
                        "soft_deadline": null,
                        "soft_deadline_description": null,
                        "checksum": "cd",
                        "return_url": "https://tmc.mooc.fi/api/v8/core/exercises/81842/submissions",
                        "zip_url": "https://tmc.mooc.fi/api/v8/core/exercises/81842/download",
                        "returnable": true,
                        "requires_review": false,
                        "attempted": false,
                        "completed": false,
                        "reviewed": false,
                        "all_review_points_given": true,
                        "memory_limit": null,
                        "runtime_params": [],
                        "valgrind_strategy": "fail",
                        "code_review_requests_enabled": false,
                        "run_tests_locally_action_enabled": true
                    },]
                }
            }).to_string())
            .create();

        let mut checksums = HashMap::new();
        checksums.insert(12, "ab".to_string());
        checksums.insert(23, "bc".to_string());
        let update_result = client.get_exercise_updates(1234, checksums).unwrap();

        assert_eq!(update_result.created.len(), 1);
        assert_eq!(update_result.created[0].id, 34);

        assert_eq!(update_result.updated.len(), 1);
        assert_eq!(update_result.updated[0].checksum, "zz");
    }

    #[test]
    fn waits_for_submission() {
        init();

        let client = make_client();
        let m = mockito::mock("GET", "/api/v8/core/submission/0")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("client".into(), "some_client".into()),
                Matcher::UrlEncoded("client_version".into(), "some_ver".into()),
            ]))
            .with_body_from_fn(|w| {
                static CALLED: AtomicBool = AtomicBool::new(false);
                if !CALLED.load(std::sync::atomic::Ordering::SeqCst) {
                    CALLED.store(true, std::sync::atomic::Ordering::SeqCst);
                    w.write_all(
                        br#"
                    {
                        "status": "processing",
                        "sandbox_status": "created"
                    }
                    "#,
                    )
                    .unwrap();
                } else {
                    w.write_all(
                        br#"
                    {
                        "api_version": 0,
                        "user_id": 1,
                        "login": "",
                        "course": "",
                        "exercise_name": "",
                        "status": "processing",
                        "points": [],
                        "submission_url": "",
                        "submitted_at": "",
                        "reviewed": false,
                        "requests_review": false,
                        "missing_review_points": []
                    }
                    "#,
                    )
                    .unwrap();
                }
                Ok(())
            })
            .expect(2)
            .create();

        let _res = client.wait_for_submission(0).unwrap();
        m.assert();
    }
}
