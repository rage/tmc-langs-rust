mod api;

use crate::error::{CoreError, Result};
use crate::request::*;
use crate::response::*;
use crate::response::{Course, CourseDetails, Organization};

use isolang::Language;
use oauth2::basic::BasicClient;
use oauth2::prelude::*;
use oauth2::{
    AuthUrl, ClientId, ClientSecret, ResourceOwnerPassword, ResourceOwnerUsername, TokenResponse,
    TokenUrl,
};
use reqwest::{blocking::Client, Url};
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::fmt::Debug;
use std::path::Path;
use std::path::PathBuf;
use tempfile::NamedTempFile;
use tmc_langs_util::task_executor;
use tmc_langs_util::{RunResult, ValidationResult};
use url1::Url as Url1;

pub type Token =
    oauth2::StandardTokenResponse<oauth2::EmptyExtraTokenFields, oauth2::basic::BasicTokenType>;

/// A struct for interacting with the TestMyCode service, including authentication
pub struct TmcCore {
    client: Client,
    config_dir: PathBuf,
    api_url: Url,
    auth_url: Url,
    token: Option<Token>,
}

// TODO: cache API results?
impl TmcCore {
    pub fn new(config_dir: PathBuf, root_url: String) -> Result<Self> {
        // guarantee a trailing slash, otherwise join will drop the last component
        let tmc_url = Url::parse(&format!("{}/", root_url))?;
        let api_url = tmc_url.join("api/v8/")?;
        let auth_url = tmc_url.join("oauth/token")?;
        Ok(Self {
            client: Client::new(),
            config_dir,
            api_url,
            auth_url,
            token: None,
        })
    }

    pub fn new_in_config(root_url: String) -> Result<Self> {
        let config_dir = dirs::cache_dir().ok_or(CoreError::CacheDir)?;
        Self::new(config_dir, root_url)
    }

    /// Attempts to log in with the given credentials, returns an error if an authentication token is already present.
    /// Username can be the user's username or email.
    pub fn authenticate(
        &mut self,
        client_name: &str,
        email: String,
        password: String,
    ) -> Result<()> {
        if self.token.is_some() {
            return Err(CoreError::AlreadyAuthenticated);
        }

        let url = self
            .api_url
            .join(&format!("application/{}/credentials", client_name))?;
        let credentials: Credentials = self.request_json(url)?;

        let auth_url = Url1::parse(self.auth_url.as_str())?;
        log::debug!("authenticating at {}", auth_url);
        let client = BasicClient::new(
            ClientId::new(credentials.application_id),
            Some(ClientSecret::new(credentials.secret)),
            AuthUrl::new(auth_url.clone()), // not used in the Resource Owner Password Credentials Grant
            Some(TokenUrl::new(auth_url)),
        );

        let token = client
            .exchange_password(
                &ResourceOwnerUsername::new(email),
                &ResourceOwnerPassword::new(password),
            )
            .map_err(|e| CoreError::Token(e))?;
        self.token = Some(token);
        log::debug!("authenticated");
        Ok(())
    }

    pub fn get_organizations(&self) -> Result<Vec<Organization>> {
        self.organizations()
    }

    pub fn send_diagnostics(&self) {
        unimplemented!()
    }

    pub fn download_or_update_exercises(&self, exercises: Vec<(usize, &Path)>) -> Result<()> {
        for (exercise_id, target) in exercises {
            let zip_file = NamedTempFile::new().map_err(|e| CoreError::TempFile(e))?;
            self.download_exercise(exercise_id, zip_file.path())?;
            task_executor::extract_project(zip_file.path(), target)?;
        }
        Ok(())
    }

    pub fn get_course_details(&self, course_id: usize) -> Result<CourseDetails> {
        self.core_course(course_id)
    }

    pub fn list_courses(&self, organization_slug: &str) -> Result<Vec<Course>> {
        self.organization_courses(organization_slug)
    }

    pub fn paste_with_comment(
        &self,
        exercise_id: usize,
        submission_path: &Path,
        paste_message: String,
    ) -> Result<NewSubmission> {
        self.post_submission_to_paste(exercise_id, submission_path, paste_message)
    }

    pub fn run_checkstyle(
        &self,
        path: &Path,
        locale: Language,
    ) -> Result<Option<ValidationResult>> {
        Ok(task_executor::run_check_code_style(path, locale)?)
    }

    pub fn run_tests(&self, path: &Path) -> Result<RunResult> {
        Ok(task_executor::run_tests(path)?)
    }

    pub fn send_feedback(
        &self,
        submission_id: usize,
        feedback: Vec<FeedbackAnswer>,
    ) -> Result<SubmissionFeedbackResponse> {
        self.post_feedback(submission_id, feedback)
    }

    pub fn send_snapshot_events(&self) {
        unimplemented!()
    }

    pub fn submit(&self, exercise_id: usize, submission_path: &Path) -> Result<NewSubmission> {
        self.post_submission(exercise_id, submission_path)
    }

    /// Fetches the course's exercises from the server,
    /// and finds new or updated exercises.
    /// If an exercise's id is not found in the checksum map, it is considered new.
    /// If an id is found, it is compared to the current one. If they are different,
    /// it is considered updated.
    pub fn get_exercise_updates(
        &self,
        course_id: usize,
        checksums: HashMap<usize, String>,
    ) -> Result<(Vec<Exercise>, Vec<Exercise>)> {
        let mut new_exercises = vec![];
        let mut updated_exercises = vec![];

        let course = self.core_course(course_id)?;
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
        Ok((new_exercises, updated_exercises))
    }

    pub fn mark_review_as_read(&self, review_update_url: String) -> Result<()> {
        self.mark_review(review_update_url, true)
    }

    pub fn get_unread_reviews(&self, course_id: usize) -> Result<Vec<Review>> {
        self.reviews(course_id)
    }

    pub fn request_code_review(
        &self,
        exercise_id: usize,
        submission_path: &Path,
        message_for_reviewer: String,
    ) -> Result<NewSubmission> {
        self.post_submission_for_review(exercise_id, submission_path, message_for_reviewer)
    }

    pub fn download_model_solution(&self, exercise_id: usize, target: &Path) -> Result<()> {
        let zip_file = NamedTempFile::new().map_err(|e| CoreError::TempFile(e))?;
        self.download_solution(exercise_id, zip_file.path())?;
        task_executor::extract_project(zip_file.path(), target)?;
        Ok(())
    }

    pub fn check_submission(&self, submission_url: &str) -> Result<SubmissionProcessingStatus> {
        if self.token.is_none() {
            return Err(CoreError::AuthRequired);
        }

        let url = Url::parse(submission_url)?;
        let res: Response<SubmissionProcessingStatus> = self.request_json(url)?;
        let res = res.into_result()?;
        Ok(res)
    }

    fn request_json<T: DeserializeOwned + Debug>(&self, url: Url) -> Result<T> {
        log::debug!("requesting {}", url);
        let mut req = self.client.get(url);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token.access_token().secret());
        }
        let res: Response<T> = req.send()?.json()?;
        log::trace!("received {:?}", res);
        res.into_result()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use mockito::{mock, Matcher};
    use std::env;

    // sets up mock-authenticated TmcCore and logging
    fn init() -> (TmcCore, String) {
        if env::var("RUST_LOG").is_err() {
            env::set_var("RUST_LOG", "debug,hyper=warn,tokio_reactor=warn");
        }
        let _ = env_logger::builder().is_test(true).try_init();

        let _m = mock("GET", "/api/v8/application/client_name/credentials")
            .with_body(
                serde_json::json!({
                    "application_id": "id",
                    "secret": "secret",
                })
                .to_string(),
            )
            .create();
        let _m = mock("POST", "/oauth/token")
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
        let mut core = TmcCore::new_in_config(local_server.to_string()).unwrap();
        core.authenticate("client_name", "email".to_string(), "password".to_string())
            .unwrap();
        (core, local_server)
    }

    #[test]
    fn get_organizations() {
        let (core, addr) = init();
        let _m = mock("GET", "/api/v8/org.json")
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

        let orgs = core.get_organizations().unwrap();
        assert_eq!(orgs[0].name, "MOOC");
    }

    #[test]
    fn download_or_update_exercises() {
        let (core, addr) = init();
        let _m = mock("GET", "/api/v8/core/exercises/1234/download")
            .with_body_from_file(Path::new("tests/data/81842.zip"))
            .create();

        let temp_dir = tempfile::tempdir().unwrap();
        let target = temp_dir.path().join("temp");
        assert!(!target.exists());
        let exercises = vec![(1234, target.as_path())];
        core.download_or_update_exercises(exercises).unwrap();
        assert!(target.join("src/main/java/Hiekkalaatikko.java").exists());
    }

    #[test]
    fn get_course_details() {
        let (core, addr) = init();
        let _m = mock("GET", "/api/v8/core/courses/1234")
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

        let course_details = core.get_course_details(1234).unwrap();
        assert_eq!(course_details.course.name, "mooc-2020-ohjelmointi");
    }

    #[test]
    fn list_courses() {
        let (core, addr) = init();
        let _m = mock("GET", "/api/v8/core/org/slug/courses")
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

        let courses = core.list_courses("slug").unwrap();
        assert_eq!(courses.len(), 1);
        assert_eq!(courses[0].name, "mooc-2013-OOProgrammingWithJava-PART1");
    }

    #[test]
    fn paste_with_comment() {
        let (core, addr) = init();
        let _m = mock("POST", "/api/v8/core/exercises/1234/submissions")
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

        let new_submission = core
            .paste_with_comment(
                1234,
                Path::new("tests/data/exercise"),
                "abcdefg".to_string(),
            )
            .unwrap();
        assert_eq!(
            new_submission.submission_url,
            "https://tmc.mooc.fi/api/v8/core/submissions/7399658"
        );
    }

    #[test]
    fn run_checkstyle() {
        // todo, just calls task executor
    }

    #[test]
    fn run_tests() {
        // todo, just calls task executor
    }

    #[test]
    fn send_feedback() {
        let (core, addr) = init();
        let _m = mock("POST", "/api/v8/core/submissions/1234/feedback")
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

        let submission_feedback_response = core
            .send_feedback(
                1234,
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
    fn submit() {
        let (core, addr) = init();
        let _m = mock("POST", "/api/v8/core/exercises/1234/submissions")
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

        let new_submission = core.submit(1234, Path::new("tests/data/exercise")).unwrap();
        assert_eq!(
            new_submission.submission_url,
            "https://tmc.mooc.fi/api/v8/core/submissions/7400888"
        );
    }

    #[test]
    fn get_exercise_updates() {
        let (core, addr) = init();
        let _m = mock("GET", "/api/v8/core/courses/1234")
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
        let (new, updated) = core.get_exercise_updates(1234, checksums).unwrap();

        assert_eq!(new.len(), 1);
        assert_eq!(new[0].id, 34);

        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].checksum, "zz");
    }

    //#[test]
    fn mark_review_as_read() {
        // todo
        let (core, addr) = init();
        let update_url = Url::parse(&addr).unwrap().join("update-url").unwrap();

        let _m = mock("POST", "/update-url.json").create();

        core.mark_review_as_read(update_url.to_string()).unwrap();
    }

    #[test]
    fn get_unread_reviews() {
        let (core, addr) = init();
        let _m = mock("GET", "/api/v8/core/courses/1234/reviews")
            .with_body(
                serde_json::json!([
                    {
                        "submission_id": "5678",
                        "exercise_name": "en",
                        "id": 90,
                        "marked_as_read": true,
                        "reviewer_name": "rn",
                        "review_body": "rb",
                        "points": ["1.1", "1.2"],
                        "points_not_awarded": ["1.3"],
                        "url": "ur",
                        "update_url": "uu",
                        "created_at": "ca",
                        "updated_at": "ua",
                    }
                ])
                .to_string(),
            )
            .create();

        let reviews = core.get_unread_reviews(1234).unwrap();
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].submission_id, "5678");
    }

    #[test]
    fn request_code_review() {
        let (core, addr) = init();
        let _m = mock("POST", "/api/v8/core/exercises/1234/submissions")
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

        let new_submission = core
            .request_code_review(
                1234,
                Path::new("tests/data/exercise"),
                "abcdefg".to_string(),
            )
            .unwrap();
        assert_eq!(
            new_submission.submission_url,
            "https://tmc.mooc.fi/api/v8/core/submissions/7402793"
        );
    }

    #[test]
    fn download_model_solution() {
        let (core, addr) = init();
        let _m = mock("GET", "/api/v8/core/exercises/1234/solution/download")
            .with_body_from_file(Path::new("tests/data/81842.zip"))
            .create();

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("temp");
        assert!(!target.exists());
        core.download_model_solution(1234, &target).unwrap();
        assert!(target.join("src/main/java/Hiekkalaatikko.java").exists());
    }

    #[test]
    fn check_submission_processing() {
        let (core, addr) = init();
        let _m = mock("GET", "/submission-url")
            .with_body(
                serde_json::json!({
                  "status": "processing",
                  "sandbox_status": "processing_on_sandbox"
                })
                .to_string(),
            )
            .create();

        let sub_url = format!("{}/submission-url", addr);
        let submission_processing_status = core.check_submission(&sub_url).unwrap();
        match submission_processing_status {
            SubmissionProcessingStatus::Finished(_) => panic!("wrong status"),
            SubmissionProcessingStatus::Processing(p) => {
                assert_eq!(p.sandbox_status, SandboxStatus::ProcessingOnSandbox)
            }
        }
    }

    #[test]
    fn check_submission_finished() {
        let (core, addr) = init();
        let _m = mock("GET", "/submission-url").with_body(serde_json::json!({
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
        let submission_processing_status = core.check_submission(&sub_url).unwrap();
        match submission_processing_status {
            SubmissionProcessingStatus::Finished(f) => {
                assert_eq!(f.all_tests_passed, Some(true));
            }
            SubmissionProcessingStatus::Processing(_) => panic!("wrong status"),
        }
    }
}
