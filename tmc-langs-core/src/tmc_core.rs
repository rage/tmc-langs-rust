use crate::response::*;
use crate::*;

use tmc_langs_util::task_executor;

use oauth2::basic::BasicClient;
use oauth2::prelude::*;
use oauth2::{
    AuthUrl, ClientId, ClientSecret, ResourceOwnerPassword, ResourceOwnerUsername, TokenResponse,
    TokenUrl,
};
use reqwest::{blocking::multipart::Form, blocking::Client, Url};
use serde::de::DeserializeOwned;
use std::fmt::Debug;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
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
    pub fn new(config_dir: PathBuf, root_url: &'static str) -> Result<Self, CoreError> {
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

    pub fn new_in_config(root_url: &'static str) -> Result<Self, CoreError> {
        let config_dir = dirs::cache_dir().ok_or(CoreError::HomeDir)?;
        Self::new(config_dir, root_url)
    }

    /// Attempts to log in with the given credentials, returns an error if an authentication token is already present.
    /// Username can be the user's username or email.
    pub fn authenticate(
        &mut self,
        client_name: &str,
        username: String,
        password: String,
    ) -> Result<(), CoreError> {
        if self.token.is_some() {
            return Err(CoreError::AlreadyAuthenticated);
        }

        let url = self
            .api_url
            .join(&format!("application/{}/credentials", client_name))
            .unwrap();
        let credentials: Credentials = self.request_json(url)?;

        let auth_url = Url1::parse(self.auth_url.as_str()).unwrap();
        let client = BasicClient::new(
            ClientId::new(credentials.application_id),
            Some(ClientSecret::new(credentials.secret)),
            AuthUrl::new(auth_url.clone()), // not used in the Resource Owner Password Credentials Grant
            Some(TokenUrl::new(auth_url)),
        );

        let token = client
            .exchange_password(
                &ResourceOwnerUsername::new(username),
                &ResourceOwnerPassword::new(password),
            )
            .map_err(|e| CoreError::Token(e))?;
        self.token = Some(token);
        log::debug!("authenticated");
        Ok(())
    }

    pub fn unauthenticate(&mut self) {
        self.token = None
    }

    pub fn is_authenticated(&self) -> bool {
        self.token.is_some()
    }

    pub fn get_user_info(&self) -> Result<User, CoreError> {
        let url = self.api_url.join("users/current")?;
        self.request_json(url)
    }

    /// Returns a list of organizations.
    pub fn get_organizations(&self) -> Result<Vec<Organization>, CoreError> {
        let url = self.api_url.join("org.json")?;
        self.request_json(url)
    }

    /// Returns an organization.
    pub fn get_organization(&self, organization_slug: &str) -> Result<Organization, CoreError> {
        let url = self
            .api_url
            .join(&format!("org/{}.json", organization_slug))?;
        self.request_json(url)
    }

    pub fn get_courses(&self, organization_slug: &str) -> Result<Vec<Course>, CoreError> {
        if self.token.is_none() {
            return Err(CoreError::AuthRequired);
        }

        let url = self
            .api_url
            .join(&format!("core/org/{}/courses", organization_slug))
            .unwrap();
        self.request_json(url)
    }

    pub fn get_course_details(&self, course_id: usize) -> Result<CourseDetails, CoreError> {
        if self.token.is_none() {
            return Err(CoreError::AuthRequired);
        }

        let url = self
            .api_url
            .join(&format!("core/courses/{}", course_id))
            .unwrap();
        let course_details: CourseDetails = self.request_json(url)?;
        Ok(course_details)
    }

    pub fn get_course_exercises(&self, course_id: usize) -> Result<Vec<CourseExercise>, CoreError> {
        if self.token.is_none() {
            return Err(CoreError::AuthRequired);
        }

        let url = self
            .api_url
            .join(&format!("courses/{}/exercises", course_id))
            .unwrap();
        self.request_json(url)
    }

    pub fn get_exercise_details(&self, course_id: usize) -> Result<ExerciseDetails, CoreError> {
        let url = self
            .api_url
            .join(&format!("core/exercises/{}", course_id))
            .unwrap();
        self.request_json(url)
    }

    pub fn download_exercise(
        &self,
        exercise_id: u32,
        target: &Path, // TODO: get target from settings?
    ) -> Result<(), CoreError> {
        // download zip
        let mut zip_file = NamedTempFile::new()?;
        let url = self
            .api_url
            .join(&format!("core/exercises/{}/download", exercise_id))
            .unwrap();

        log::debug!("requesting {}", url);
        let mut res = self.client.get(url).send()?;
        res.copy_to(&mut zip_file)?; // write response to target file

        // extract
        task_executor::extract_project(&zip_file.path(), target)?;
        Ok(())
    }

    pub fn get_submissions(&self, course_id: usize) -> Result<Vec<Submission>, CoreError> {
        let url = self
            .api_url
            .join(&format!("courses/{}/users/current/submissions", course_id))?;
        let res: Response<Vec<Submission>> = self.request_json(url)?;
        let res = res.into_result()?;
        Ok(res)
    }

    pub fn download_submission(
        &self,
        submission_id: usize,
        target: &Path,
    ) -> Result<(), CoreError> {
        // download zip
        let mut zip_file = NamedTempFile::new()?;
        let url = self
            .api_url
            .join(&format!("core/submissions/{}/download", submission_id))
            .unwrap();

        log::debug!("requesting {}", url);
        let mut res = self.client.get(url).send()?;
        res.copy_to(&mut zip_file)?; // write response to target file

        // extract
        task_executor::extract_project(&zip_file.path(), target)?;
        Ok(())
    }

    pub fn submit_exercise(
        &self,
        exercise_id: usize,
        exercise_path: &Path, // TODO: get from settings?
    ) -> Result<NewSubmission, CoreError> {
        // compress
        let compressed = task_executor::compress_project(exercise_path)?;
        let mut file = NamedTempFile::new()?;
        file.write_all(&compressed)?;

        let url = self
            .api_url
            .join(&format!("core/exercises/{}/submissions", exercise_id))
            .unwrap();

        // send
        let form = Form::new().file("submission[file]", file.path())?;

        log::debug!("posting {}", url);
        let mut req = self.client.post(url).multipart(form);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token.access_token().secret());
        }
        let res: Response<NewSubmission> = req.send()?.json()?;
        log::debug!("received {:?}", res);

        let submission = res.into_result()?;
        Ok(submission)
    }

    pub fn check_submission(
        &self,
        submission_url: &str,
    ) -> Result<SubmissionProcessingStatus, CoreError> {
        let url = Url::parse(submission_url)?;
        let res: Response<SubmissionProcessingStatus> = self.request_json(url)?;
        let res = res.into_result()?;
        Ok(res)
    }

    pub fn submit_feedback(
        &self,
        feedback_url: &str,
        feedback_answers: Vec<FeedbackAnswer>,
    ) -> Result<SubmissionFeedbackResponse, CoreError> {
        let mut req = self.client.post(feedback_url);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token.access_token().secret());
        }
        for (i, answer) in feedback_answers.into_iter().enumerate() {
            req = req
                .query(&[(format!("answers[{}][question_id]", i), answer.question_id)])
                .query(&[(format!("answers[{}][answer]", i), answer.answer)]);
        }
        log::debug!("posting {}", feedback_url);
        let res: Response<SubmissionFeedbackResponse> = req.send()?.json()?;
        log::trace!("received {:?}", res);
        let res = res.into_result()?;
        Ok(res)
    }

    fn request_json<T: DeserializeOwned + Debug>(&self, url: Url) -> Result<T, CoreError> {
        log::debug!("requesting {}", url);
        let mut req = self.client.get(url);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token.access_token().secret());
        }
        let res: Response<T> = req.send()?.json()?;
        log::trace!("received {:?}", res);
        res.into_result()
    }

    fn download_model_solution(&self) {
        todo!()
    }

    fn get_unread_reviews(&self) {
        todo!()
    }

    fn get_updateable_exercises(&self) {
        todo!()
    }
}

// TODO: use mock server
#[cfg(test)]
mod test {
    use super::*;
    use serde_json::Value;

    const ROOT_URL: &'static str = "https://tmc.mooc.fi";

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    fn authenticated_core() -> TmcCore {
        dotenv::dotenv().ok();
        let user = std::env::var("TMC_USER").unwrap();
        let pass = std::env::var("TMC_PASS").unwrap();
        let mut core = TmcCore::new_in_config(ROOT_URL).unwrap();
        core.authenticate("vscode_plugin", user, pass).unwrap();
        core
    }

    #[test]
    #[ignore]
    fn authenticates() {
        init();

        dotenv::dotenv().ok();
        let user = std::env::var("TMC_USER").unwrap();
        let pass = std::env::var("TMC_PASS").unwrap();

        let mut core = TmcCore::new_in_config(ROOT_URL).unwrap();
        assert!(core.token.is_none());
        core.authenticate("vscode_plugin", user, pass).unwrap();
        assert!(core.token.is_some());
    }

    #[test]
    #[ignore]
    fn downloads_exercise() {
        init();

        let core = authenticated_core();
        let target = Path::new("test-targ");
        core.download_exercise(81843, target).unwrap();
        assert!(target.exists());
    }

    #[test]
    #[ignore]
    fn submits_exercise() {
        init();

        let core = authenticated_core();
        let exercise_path = Path::new("tests/data/exercise");
        let submission = core.submit_exercise(83114, exercise_path).unwrap();
    }

    #[test]
    //#[ignore]
    fn submits_feedback() {
        init();

        let core = authenticated_core();
        let exercise_path = Path::new("tests/data/exercise");
        let submission = core.submit_exercise(83114, exercise_path).unwrap();
        let submission_url = submission.submission_url;
        let f = loop {
            let submission_status = core.check_submission(&submission_url).unwrap();
            match submission_status {
                SubmissionProcessingStatus::Processing(p) => {
                    log::debug!("{:?}", p);
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    continue;
                }
                SubmissionProcessingStatus::Finished(f) => {
                    log::debug!("{:?}", f);
                    break f;
                }
            }
        };
        let questions = f.feedback_questions.unwrap();
        let mut answers = vec![];
        for q in questions {
            match q.kind {
                SubmissionFeedbackKind::Text => answers.push(FeedbackAnswer {
                    answer: "test".to_string(),
                    question_id: q.id,
                }),
                SubmissionFeedbackKind::IntRange { upper, lower } => answers.push(FeedbackAnswer {
                    answer: ((upper + lower) / 2).to_string(),
                    question_id: q.id,
                }),
            }
        }
        let res = core
            .submit_feedback(&f.feedback_answer_url.unwrap(), answers)
            .unwrap();
        log::debug!("{:?}", res);
        panic!()
    }

    #[test]
    #[ignore]
    fn gets_organizations() {
        init();

        let core = TmcCore::new_in_config(ROOT_URL).unwrap();
        let orgs = core.get_organization("mooc").unwrap();
        assert_eq!(orgs.name, "MOOC");
    }

    #[test]
    #[ignore]
    fn gets_courses() {
        init();

        let core = authenticated_core();
        let courses = core.get_courses("mooc").unwrap();
        assert!(!courses.is_empty());
    }

    #[test]
    #[ignore]
    fn gets_course_details() {
        init();

        let core = authenticated_core();
        let course = core.get_course_details(588).unwrap();
        assert_eq!(course.course.name, "mooc-2020-ohjelmointi");
    }

    #[test]
    #[ignore]
    fn gets_course_exercises() {
        init();

        let core = authenticated_core();
        let course_exercises = core.get_course_exercises(588).unwrap();
        assert!(!course_exercises.is_empty());
    }

    #[test]
    #[ignore]
    fn gets_exercise_details() {
        init();

        let core = authenticated_core();
        let exercise_details = core.get_exercise_details(81843).unwrap();
        assert_eq!(exercise_details.course_name, "mooc-2020-ohjelmointi");
    }

    //#[test]
    fn test() {
        init();
        let core = authenticated_core();
        let root = Url::parse("https://tmc.mooc.fi/api/v8/").unwrap();

        let url = root
            .join("org/mooc/courses/java-programming-i/exercises")
            .unwrap();
        let res: Value = core.request_json(url.clone()).unwrap();
        println!("auth");
        println!("{:#}", res);

        let core = TmcCore::new_in_config(ROOT_URL).unwrap();
        let res: Value = core.request_json(url).unwrap();
        println!("anon");
        println!("{:#}", res);
        panic!();
    }
}
