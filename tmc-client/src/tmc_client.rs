//! Contains the TmcClient struct for communicating with the TMC server.
pub mod api_v8;

use crate::error::ClientError;
use crate::request::*;
use crate::response::*;
use crate::Language;
use oauth2::{
    basic::BasicClient, AuthUrl, ClientId, ClientSecret, ResourceOwnerPassword,
    ResourceOwnerUsername, TokenUrl,
};
use reqwest::{blocking::Client, Url};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::{collections::HashMap, io::Cursor};
use std::{io::Write, u32};
use tmc_langs_util::progress_reporter;

/// Authentication token.
pub type Token =
    oauth2::StandardTokenResponse<oauth2::EmptyExtraTokenFields, oauth2::basic::BasicTokenType>;

/// The update data type for the progress reporter.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "client-update-data-kind")]
pub enum ClientUpdateData {
    ExerciseDownload { id: u32, path: PathBuf },
    PostedSubmission(NewSubmission),
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
    fn require_authentication(&self) -> Result<(), ClientError> {
        if self.0.token.is_some() {
            Ok(())
        } else {
            Err(ClientError::NotLoggedIn)
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
    /// let client = TmcClient::new("https://tmc.mooc.fi".to_string(), "some_client".to_string(), "some_version".to_string()).unwrap();
    /// ```
    pub fn new(root_url: Url, client_name: String, client_version: String) -> Self {
        // guarantee a trailing slash, otherwise join will drop the last component
        let root_url = if root_url.as_str().ends_with('/') {
            root_url
        } else {
            format!("{}/", root_url).parse().expect("invalid root url")
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
    /// let mut client = TmcClient::new("https://tmc.mooc.fi".to_string(), "some_client".to_string(), "some_version".to_string()).unwrap();
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

    /// Fetches the course's information. Requires authentication.
    ///
    /// # Errors
    /// If not authenticated, there's some problem reaching the API, or if the API returns an error.
    pub fn get_course_submissions(&self, course_id: u32) -> Result<Vec<Submission>, ClientError> {
        self.require_authentication()?;
        api_v8::submission::get_course_submissions_by_id(self, course_id)
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
    /// let client = TmcClient::new_in_config("https://tmc.mooc.fi".to_string(), "some_client".to_string(), "some_version".to_string()).unwrap();
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
        let compressed = tmc_langs_plugins::compress_project_to_zip(submission_path)?;
        progress_stage(
            "Compressed paste submission. Posting paste submission...",
            None,
        );

        let result = api_v8::core::submit_exercise(
            self,
            exercise_id,
            Cursor::new(compressed),
            paste_message,
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
        let compressed = tmc_langs_plugins::compress_project_to_zip(submission_path)?;
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

    /// Waits for a submission to finish. May require authentication.
    ///
    /// # Errors
    /// If authentication is required but the client is not authenticated, there's some problem reaching the API, or if the API returns an error.
    pub fn wait_for_submission(
        &self,
        submission_url: &str,
    ) -> Result<SubmissionFinished, ClientError> {
        start_stage(4, "Waiting for submission", None);

        let mut previous_status = None;
        loop {
            match self.check_submission(submission_url)? {
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
    /// let client = TmcClient::new_in_config("https://tmc.mooc.fi".to_string(), "some_client".to_string(), "some_version".to_string()).unwrap();
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
        message_for_reviewer: String,
        locale: Option<Language>,
    ) -> Result<NewSubmission, ClientError> {
        self.require_authentication()?;

        // compress
        let compressed = tmc_langs_plugins::compress_project_to_zip(submission_path)?;

        api_v8::core::submit_exercise(
            self,
            exercise_id,
            Cursor::new(compressed),
            None,
            Some(message_for_reviewer),
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
        tmc_langs_plugins::extract_project(Cursor::new(buf), target, false)?;
        Ok(())
    }

    /// Checks the status of a submission on the server. May require authentication.
    ///
    /// # Errors
    /// If authentication is required but the client is not authenticated, if there's some problem reaching the API, or if the API returns an error.
    pub fn check_submission(
        &self,
        submission_url: &str,
    ) -> Result<SubmissionProcessingStatus, ClientError> {
        let url = Url::parse(submission_url)
            .map_err(|e| ClientError::UrlParse(submission_url.to_string(), e))?;
        api_v8::get_submission_processing_status(self, url)
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

impl AsRef<TmcCore> for TmcClient {
    fn as_ref(&self) -> &TmcCore {
        &self.0
    }
}

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
#[allow(clippy::clippy::unwrap_used)]
mod test {
    use super::*;
    use mockito::{mock, Matcher};

    // sets up mock-authenticated TmcClient and logging
    fn init() -> (TmcClient, String) {
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

        let _m = mock("GET", "/api/v8/application/client_name/credentials")
            .match_query(Matcher::Any)
            .with_body(
                serde_json::json!({
                    "application_id": "id",
                    "secret": "secret",
                })
                .to_string(),
            )
            .create();
        let _m = mock("POST", "/oauth/token")
            .match_query(Matcher::Any)
            .with_body(
                serde_json::json!({
                    "access_token": "token",
                    "token_type": "bearer",
                })
                .to_string(),
            )
            .create();
        let local_server = mockito::server_url();
        log::debug!("local {}", local_server);
        let mut client = TmcClient::new(
            local_server.parse().unwrap(),
            "some_client".to_string(),
            "some_ver".to_string(),
        );
        client
            .authenticate("client_name", "email".to_string(), "password".to_string())
            .unwrap();
        (client, local_server)
    }

    #[test]
    fn gets_organizations() {
        let (client, _addr) = init();
        let _m = mock("GET", "/api/v8/org.json")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("client".into(), "some_client".into()),
                Matcher::UrlEncoded("client_version".into(), "some_ver".into()),
            ]))
            .with_body(
                serde_json::json!([
                    {
                        "information": "University of Helsinki's Massive Open Online Courses. All new courses from mooc.fi live here.",
                        "logo_path": "/system/organizations/logos/000/000/021/original/mooc-logo.png?1513356394",
                        "name": "MOOC",
                        "pinned": true,
                        "slug": "mooc"
                    }
                ]).to_string()
            )
            .create();

        let orgs = client.get_organizations().unwrap();
        assert_eq!(orgs[0].name, "MOOC");
    }

    #[test]
    fn gets_course_details() {
        let (client, _addr) = init();
        let _m = mock("GET", "/api/v8/core/courses/1234")
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
                        "id": 81842,
                        "name": "osa01-Osa01_01.Hiekkalaatikko",
                        "locked": false,
                        "deadline_description": "2020-01-20 23:59:59 +0200",
                        "deadline": "2020-01-20T23:59:59.999+02:00",
                        "soft_deadline": null,
                        "soft_deadline_description": null,
                        "checksum": "cb78336824109de610ce3d91d43a9954",
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
                    }]
                }
            }).to_string())
            .create();

        let course_details = client.get_course_details(1234).unwrap();
        assert_eq!(course_details.course.name, "mooc-2020-ohjelmointi");
    }

    #[test]
    fn lists_courses() {
        let (client, _addr) = init();
        let _m = mock("GET", "/api/v8/core/org/slug/courses")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("client".into(), "some_client".into()),
                Matcher::UrlEncoded("client_version".into(), "some_ver".into()),
            ]))
            .with_body(serde_json::json!([
                    {
                        "id": 277,
                        "name": "mooc-2013-OOProgrammingWithJava-PART1",
                        "title": "2013 Object-oriented programming, part 1",
                        "description": "2013 Object Oriented Programming with Java.  If you're not finding your old submissions, they're probably on our old server https://tmc.mooc.fi/mooc.",
                        "details_url": "https://tmc.mooc.fi/api/v8/core/courses/277",
                        "unlock_url": "https://tmc.mooc.fi/api/v8/core/courses/277/unlock",
                        "reviews_url": "https://tmc.mooc.fi/api/v8/core/courses/277/reviews",
                        "comet_url": "https://tmc.mooc.fi:8443/comet",
                        "spyware_urls": [
                        "http://snapshots01.mooc.fi/"
                        ]
                    },
                ]).to_string())
            .create();

        let courses = client.list_courses("slug").unwrap();
        assert_eq!(courses.len(), 1);
        assert_eq!(courses[0].name, "mooc-2013-OOProgrammingWithJava-PART1");
    }

    #[test]
    fn pastes_with_comment() {
        let (client, url) = init();
        let _m = mock("POST", "/submission")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("client".into(), "some_client".into()),
                Matcher::UrlEncoded("client_version".into(), "some_ver".into()),
            ]))
            .match_body(Matcher::Regex("paste".to_string()))
            .match_body(Matcher::Regex("message_for_paste".to_string()))
            .match_body(Matcher::Regex("abcdefg".to_string()))
            .with_body(
                serde_json::json!({
                    "submission_url": "https://tmc.mooc.fi/api/v8/core/submissions/7399658",
                    "paste_url": "https://tmc.mooc.fi/paste/dYbznt87x_00deB9LBSuNQ",
                    "show_submission_url": "https://tmc.mooc.fi/submissions/7399658"
                })
                .to_string(),
            )
            .create();

        let new_submission = client
            .paste(
                1,
                Path::new("tests/data/exercise"),
                Some("abcdefg".to_string()),
                Some(Language::Eng),
            )
            .unwrap();
        assert_eq!(
            new_submission.submission_url,
            "https://tmc.mooc.fi/api/v8/core/submissions/7399658"
        );
    }

    #[test]
    fn sends_feedback() {
        let (client, url) = init();
        let _m = mock("POST", "/feedback")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("client".into(), "some_client".into()),
                Matcher::UrlEncoded("client_version".into(), "some_ver".into()),
            ]))
            .match_body(Matcher::AllOf(vec![
                Matcher::Regex(r#"answers\[0\]\[question_id\]"#.to_string()),
                Matcher::Regex(r#"answers\[0\]\[answer\]"#.to_string()),
                Matcher::Regex(r#"answers\[1\]\[question_id\]"#.to_string()),
                Matcher::Regex(r#"answers\[1\]\[answer\]"#.to_string()),
                Matcher::Regex("1234".to_string()),
                Matcher::Regex("5678".to_string()),
                Matcher::Regex("ans0".to_string()),
                Matcher::Regex("ans1".to_string()),
            ]))
            .with_body(
                serde_json::json!({
                    "api_version": 8,
                    "status": "ok",
                })
                .to_string(),
            )
            .create();

        let submission_feedback_response = client
            .send_feedback(
                1,
                vec![
                    FeedbackAnswer {
                        question_id: 1234,
                        answer: "ans0".to_string(),
                    },
                    FeedbackAnswer {
                        question_id: 5678,
                        answer: "ans1".to_string(),
                    },
                ],
            )
            .unwrap();
        assert_eq!(submission_feedback_response.api_version, 8);
    }

    #[test]
    fn submits() {
        let (client, url) = init();
        let _m = mock("POST", "/submission")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("client".into(), "some_client".into()),
                Matcher::UrlEncoded("client_version".into(), "some_ver".into()),
            ]))
            .match_body(Matcher::Regex(r#"submission\[file\]"#.to_string()))
            .with_body(
                serde_json::json!({
                    "submission_url": "https://tmc.mooc.fi/api/v8/core/submissions/7400888",
                    "paste_url": "",
                    "show_submission_url": "https://tmc.mooc.fi/submissions/7400888"
                })
                .to_string(),
            )
            .create();

        let new_submission = client
            .submit(1, Path::new("tests/data/exercise"), Some(Language::Eng))
            .unwrap();
        assert_eq!(
            new_submission.submission_url,
            "https://tmc.mooc.fi/api/v8/core/submissions/7400888"
        );
    }

    #[test]
    fn gets_exercise_updates() {
        let (client, _addr) = init();
        let _m = mock("GET", "/api/v8/core/courses/1234")
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
    #[ignore]
    fn marks_review_as_read() {
        let (client, addr) = init();
        let _m = mock("POST", "/update-url.json").create();
        client.mark_review_as_read(1, 2).unwrap();
    }

    #[test]
    fn gets_unread_reviews() {
        let (client, addr) = init();
        let _m = mock("GET", "/reviews")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("client".into(), "some_client".into()),
                Matcher::UrlEncoded("client_version".into(), "some_ver".into()),
            ]))
            .with_body(
                serde_json::json!([
                    {
                        "submission_id": 5678,
                        "exercise_name": "en",
                        "id": 90,
                        "marked_as_read": true,
                        "reviewer_name": "rn",
                        "review_body": "rb",
                        "points": ["1.1", "1.2"],
                        "points_not_awarded": ["1.3"],
                        "url": "ur",
                        "update_url": "uu",
                        "created_at": "2021-03-24T11:31:55+00:00",
                        "updated_at": "2021-03-24T11:31:55+00:00",
                    }
                ])
                .to_string(),
            )
            .create();

        let reviews = client.get_unread_reviews(1).unwrap();
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].submission_id, 5678);
    }

    #[test]
    fn requests_code_review() {
        let (client, url) = init();
        let _m = mock("POST", "/submission")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("client".into(), "some_client".into()),
                Matcher::UrlEncoded("client_version".into(), "some_ver".into()),
            ]))
            .match_body(Matcher::Regex("request_review".to_string()))
            .match_body(Matcher::Regex("message_for_reviewer".to_string()))
            .match_body(Matcher::Regex("abcdefg".to_string()))
            .with_body(
                serde_json::json!({
                    "submission_url": "https://tmc.mooc.fi/api/v8/core/submissions/7402793",
                    "paste_url": "",
                    "show_submission_url": "https://tmc.mooc.fi/submissions/7402793"
                })
                .to_string(),
            )
            .create();

        let new_submission = client
            .request_code_review(
                1,
                Path::new("tests/data/exercise"),
                "abcdefg".to_string(),
                Some(Language::Eng),
            )
            .unwrap();
        assert_eq!(
            new_submission.submission_url,
            "https://tmc.mooc.fi/api/v8/core/submissions/7402793"
        );
    }

    #[test]
    fn downloads_model_solution() {
        let (client, addr) = init();
        let _m = mock("GET", "/solution")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("client".into(), "some_client".into()),
                Matcher::UrlEncoded("client_version".into(), "some_ver".into()),
            ]))
            .with_body_from_file(Path::new("tests/data/81842.zip"))
            .create();

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("temp");
        assert!(!target.exists());
        client.download_model_solution(1, &target).unwrap();
        assert!(target.join("src/main/java/Hiekkalaatikko.java").exists());
    }

    #[test]
    fn checks_submission_processing() {
        let (client, addr) = init();
        let _m = mock("GET", "/submission-url")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("client".into(), "some_client".into()),
                Matcher::UrlEncoded("client_version".into(), "some_ver".into()),
            ]))
            .with_body(
                serde_json::json!({
                  "status": "processing",
                  "sandbox_status": "processing_on_sandbox"
                })
                .to_string(),
            )
            .create();

        let sub_url = format!("{}/submission-url", addr);
        let submission_processing_status = client.check_submission(&sub_url).unwrap();
        match submission_processing_status {
            SubmissionProcessingStatus::Finished(_) => panic!("wrong status"),
            SubmissionProcessingStatus::Processing(p) => {
                assert_eq!(p.sandbox_status, SandboxStatus::ProcessingOnSandbox)
            }
        }
    }

    #[test]
    fn checks_submission_finished() {
        let (client, addr) = init();
        let _m = mock("GET", "/submission-url")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("client".into(), "some_client".into()),
                Matcher::UrlEncoded("client_version".into(), "some_ver".into()),
            ]))
            .with_body(serde_json::json!({
            "api_version": 7,
            "all_tests_passed": true,
            "user_id": 3232,
            "login": "014464865",
            "course": "mooc-java-programming-i",
            "exercise_name": "part01-Part01_01.Sandbox",
            "status": "ok",
            "points": [
                "01-01"
            ],
            "validations": {
                "strategy": "DISABLED",
                "validationErrors": {}
            },
            "valgrind": "",
            "submission_url": "https://tmc.mooc.fi/submissions/7402793",
            "solution_url": "https://tmc.mooc.fi/exercises/83113/solution",
            "submitted_at": "2020-06-15T16:05:08.105+03:00",
            "processing_time": 13,
            "reviewed": false,
            "requests_review": true,
            "paste_url": null,
            "message_for_paste": null,
            "missing_review_points": [],
            "test_cases": [
                {
                    "name": "SandboxTest freePoints",
                    "successful": true,
                    "message": "",
                    "exception": [],
                    "detailed_message": null
                }
            ],
            "feedback_questions": [
                {
                    "id": 389,
                    "question": "How well did you concentrate doing this exercise? (1: not at all, 5: very well)",
                    "kind": "intrange[1..5]"
                },
                {
                    "id": 390,
                    "question": "How much do you feel you learned doing this exercise? (1: Did not learn anything, 5: Learned a lot)",
                    "kind": "intrange[1..5]"
                },
            ],
            "feedback_answer_url": "https://tmc.mooc.fi/api/v8/core/submissions/7402793/feedback"
        }).to_string()).create();

        let sub_url = format!("{}/submission-url", addr);
        let submission_processing_status = client.check_submission(&sub_url).unwrap();
        match submission_processing_status {
            SubmissionProcessingStatus::Finished(f) => {
                assert_eq!(f.all_tests_passed, Some(true));
            }
            SubmissionProcessingStatus::Processing(_) => panic!("wrong status"),
        }
    }
}
