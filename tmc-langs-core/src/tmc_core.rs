mod api;

use crate::error::CoreError;
use crate::request::*;
use crate::response::*;
use crate::response::{Course, CourseDetails, Organization};
use crate::{Language, RunResult, ValidationResult};
use oauth2::basic::BasicClient;
use oauth2::{
    AuthUrl, ClientId, ClientSecret, ResourceOwnerPassword, ResourceOwnerUsername, TokenUrl,
};
use reqwest::{blocking::Client, Url};
use serde::Serialize;
use std::cell::Cell;
use std::collections::HashMap;
use std::error::Error as StdError;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use tempfile::NamedTempFile;
use tmc_langs_util::{file_util, task_executor, FileIo};
use walkdir::WalkDir;

pub type Token =
    oauth2::StandardTokenResponse<oauth2::EmptyExtraTokenFields, oauth2::basic::BasicTokenType>;

#[derive(Debug, Serialize)]
pub struct StatusUpdate {
    pub finished: bool,
    pub message: String,
    pub percent_done: f64,
    pub status_type: StatusType,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StatusType {
    DownloadingExercise { id: usize, path: PathBuf },
    DownloadedExercise { id: usize, path: PathBuf },
    Processing,
    Sending,
    WaitingForResults,
    Finished,
    IntermediateStepFinished,
}

// compatible with anyhow
type DynError = Box<dyn StdError + Send + Sync + 'static>;
type UpdateClosure = Box<dyn Fn(StatusUpdate) -> Result<(), DynError>>;

/// A struct for interacting with the TestMyCode service, including authentication
pub struct TmcCore {
    client: Client,
    #[allow(dead_code)]
    config_dir: PathBuf, // not used yet
    api_url: Url,
    auth_url: String,
    token: Option<Token>,
    progress_report: Option<UpdateClosure>,
    progress_steps_done: Cell<u32>,
    progress_steps_total: u32,
    client_name: String,
    client_version: String,
}

// TODO: cache API results?
impl TmcCore {
    /// Creates a new TmcCore with the given config directory and root URL.
    ///
    /// # Errors
    /// This function will return an error if parsing the root_url fails.
    ///
    /// # Examples
    /// ```rust,no_run
    /// use tmc_langs_core::TmcCore;
    /// use std::path::PathBuf;
    ///
    /// let core = TmcCore::new(PathBuf::from("./config"), "https://tmc.mooc.fi".to_string(), "some_client".to_string(), "some_version".to_string()).unwrap();
    /// ```
    pub fn new(
        config_dir: PathBuf,
        root_url: String,
        client_name: String,
        client_version: String,
    ) -> Result<Self, CoreError> {
        // guarantee a trailing slash, otherwise join will drop the last component
        let root_url = if root_url.ends_with('/') {
            root_url
        } else {
            format!("{}/", root_url)
        };
        let tmc_url = Url::parse(&root_url).map_err(|e| CoreError::UrlParse(root_url, e))?;
        let api_url = tmc_url.join("api/v8/").expect("failed to join api/v8/");
        let auth_url = tmc_url
            .join("oauth/token")
            .expect("failed to join oauth/token")
            .to_string();
        Ok(Self {
            client: Client::new(),
            config_dir,
            api_url,
            auth_url,
            token: None,
            progress_report: None,
            progress_steps_done: Cell::new(0),
            progress_steps_total: 1,
            client_name,
            client_version,
        })
    }

    /// Creates a new TmcCore with the given root URL. The config directory is set according to dirs::cache_dir.
    ///
    /// # Errors
    /// This function will return an error if parsing the root_url fails, or if fetching the cache directory fails (see dirs::cache_dir()).
    ///
    /// # Examples
    /// ```rust,no_run
    /// use tmc_langs_core::TmcCore;
    ///
    /// let core = TmcCore::new_in_config("https://tmc.mooc.fi".to_string(), "some_client".to_string(), "some_ver".to_string()).unwrap();
    /// ```
    pub fn new_in_config(
        root_url: String,
        client_name: String,
        client_version: String,
    ) -> Result<Self, CoreError> {
        let config_dir = dirs::cache_dir().ok_or(CoreError::CacheDir)?;
        Self::new(config_dir, root_url, client_name, client_version)
    }

    pub fn set_token(&mut self, token: Token) {
        self.token = Some(token);
    }

    pub fn set_progress_report<F>(&mut self, progress_report: F)
    where
        F: 'static + Fn(StatusUpdate) -> Result<(), Box<dyn StdError + Send + Sync + 'static>>,
    {
        self.progress_report = Some(Box::new(progress_report));
    }

    pub fn increment_progress_steps(&mut self) {
        self.progress_steps_total += 1;
    }

    fn report_progress(&self, message: String, status_type: StatusType, percent_done: f64) {
        let from_prev_steps = self.progress_steps_done.get() as f64;
        let percent_done = (from_prev_steps + percent_done) / self.progress_steps_total as f64;

        self.progress_report.as_ref().map(|f| {
            f(StatusUpdate {
                finished: false,
                message,
                percent_done,
                status_type,
            })
        });
    }

    fn report_complete(&self, message: String) {
        self.progress_steps_done
            .set(self.progress_steps_done.get() + 1);
        if self.progress_steps_done.get() == self.progress_steps_total {
            self.progress_report.as_ref().map(|f| {
                f(StatusUpdate {
                    finished: true,
                    message,
                    percent_done: 1.0,
                    status_type: StatusType::Finished,
                })
            });
        } else {
            self.report_progress(message, StatusType::IntermediateStepFinished, 0.0);
        }
    }

    /// Attempts to log in with the given credentials, returns an error if an authentication token is already present.
    /// Username can be the user's username or email.
    ///
    /// # Errors
    /// This function will return an error if the core has already been authenticated,
    /// if the client_name is malformed and leads to a malformed URL,
    /// or if there is some error during the token exchange (see oauth2::Client::excange_password).
    ///
    /// # Examples
    /// ```rust,no_run
    /// use tmc_langs_core::TmcCore;
    ///
    /// let mut core = TmcCore::new_in_config("https://tmc.mooc.fi".to_string(), "some_client".to_string(), "some_version".to_string()).unwrap();
    /// core.authenticate("client", "user".to_string(), "pass".to_string()).unwrap();
    /// ```
    pub fn authenticate(
        &mut self,
        client_name: &str,
        email: String,
        password: String,
    ) -> Result<Token, CoreError> {
        if self.token.is_some() {
            return Err(CoreError::AlreadyAuthenticated);
        }

        let tail = format!("application/{}/credentials", client_name);
        let url = self
            .api_url
            .join(&tail)
            .map_err(|e| CoreError::UrlParse(tail, e))?;
        let credentials: Credentials = self.get_json_from_url(url)?;

        log::debug!("authenticating at {}", self.auth_url);
        let client = BasicClient::new(
            ClientId::new(credentials.application_id),
            Some(ClientSecret::new(credentials.secret)),
            AuthUrl::new(self.auth_url.clone())
                .map_err(|e| CoreError::UrlParse(self.auth_url.clone(), e))?, // not used in the Resource Owner Password Credentials Grant
            Some(
                TokenUrl::new(self.auth_url.clone())
                    .map_err(|e| CoreError::UrlParse(self.auth_url.clone(), e))?,
            ),
        );

        let token = client
            .exchange_password(
                &ResourceOwnerUsername::new(email),
                &ResourceOwnerPassword::new(password),
            )
            .request(oauth2::reqwest::http_client)?;
        self.token = Some(token.clone());
        log::debug!("authenticated");
        Ok(token)
    }

    /// Fetches all organizations.
    ///
    /// # Errors
    /// Returns an error if there's some problem reaching the API, or if the API returns an error.
    pub fn get_organizations(&self) -> Result<Vec<Organization>, CoreError> {
        self.organizations()
    }

    /// Fetches an organization.
    ///
    /// # Errors
    /// Returns an error if there's some problem reaching the API, or if the API returns an error.
    pub fn get_organization(&self, organization_slug: &str) -> Result<Organization, CoreError> {
        self.organization(organization_slug)
    }

    /// Unimplemented.
    #[deprecated = "unimplemented"]
    pub fn send_diagnostics(&self) {
        unimplemented!()
    }

    /// Downloads the given exercises. Overwrites existing exercises if they exist.
    ///
    /// # Errors
    /// Returns an error if there's some problem reaching the API, or if the API returns an error.
    /// The method extracts zip archives, which may fail.
    ///
    ///
    /// # Examples
    /// ```rust,no_run
    /// use tmc_langs_core::TmcCore;
    /// use std::path::Path;
    ///
    /// let core = TmcCore::new_in_config("https://tmc.mooc.fi".to_string(), "some_client".to_string(), "some_version".to_string()).unwrap();
    /// // authenticate
    /// core.download_or_update_exercises(vec![
    ///     (1234, Path::new("./exercises/1234")),
    ///     (2345, Path::new("./exercises/2345")),
    /// ]);
    /// ```
    pub fn download_or_update_exercises(
        &self,
        exercises: Vec<(usize, &Path)>,
    ) -> Result<(), CoreError> {
        let exercises_len = exercises.len();
        let step = 1.0 / (2 * exercises_len) as f64;

        let mut progress = 0.0;
        for (n, (exercise_id, target)) in exercises.into_iter().enumerate() {
            // TODO: do in memory without zip_file?
            let zip_file = NamedTempFile::new().map_err(CoreError::TempFile)?;

            self.report_progress(
                format!(
                    "Downloading exercise {} to '{}'. ({} out of {})",
                    exercise_id,
                    target.display(),
                    n,
                    exercises_len
                ),
                StatusType::DownloadingExercise {
                    id: exercise_id,
                    path: target.to_path_buf(),
                },
                progress,
            );
            self.download_exercise(exercise_id, zip_file.path())?;
            progress += step;

            task_executor::extract_project(zip_file.path(), target, true)?;
            self.report_progress(
                format!(
                    "Downloaded exercise {} to '{}'. ({} out of {})",
                    exercise_id,
                    target.display(),
                    n,
                    exercises_len
                ),
                StatusType::DownloadedExercise {
                    id: exercise_id,
                    path: target.to_path_buf(),
                },
                progress,
            );
            progress += step;
        }
        self.report_complete(format!("Finished downloading {} exercises.", exercises_len));
        Ok(())
    }

    /// Fetches the course's information.
    ///
    /// # Errors
    /// Returns an error if there's some problem reaching the API, or if the API returns an error.
    ///
    /// # Examples
    /// ```rust,no_run
    /// use tmc_langs_core::TmcCore;
    ///
    /// let core = TmcCore::new_in_config("https://tmc.mooc.fi".to_string(), "some_client".to_string(), "some_version".to_string()).unwrap();
    /// // authenticate
    /// let course_details = core.get_course_details(600).unwrap();
    /// ```
    pub fn get_course_details(&self, course_id: usize) -> Result<CourseDetails, CoreError> {
        self.core_course(course_id)
    }

    pub fn get_exercise_details(&self, exercise_id: usize) -> Result<ExerciseDetails, CoreError> {
        self.core_exercise(exercise_id)
    }

    pub fn get_course_submissions(&self, course_id: usize) -> Result<Vec<Submission>, CoreError> {
        self.course_submissions(course_id)
    }

    /// Fetches all courses under the given organization.
    ///
    /// # Errors
    /// Returns an error if there's some problem reaching the API, or if the API returns an error.
    ///
    /// # Examples
    /// ```rust,no_run
    /// use tmc_langs_core::TmcCore;
    ///
    /// let core = TmcCore::new_in_config("https://tmc.mooc.fi".to_string(), "some_client".to_string(), "some_version".to_string()).unwrap();
    /// // authenticate
    /// let courses = core.list_courses("hy").unwrap();
    /// ```
    pub fn list_courses(&self, organization_slug: &str) -> Result<Vec<Course>, CoreError> {
        if self.token.is_none() {
            return Err(CoreError::NotLoggedIn);
        }
        self.organization_courses(organization_slug)
    }

    pub fn get_course(&self, course_id: usize) -> Result<CourseData, CoreError> {
        if self.token.is_none() {
            return Err(CoreError::NotLoggedIn);
        }
        self.course(course_id)
    }

    pub fn get_course_exercises(&self, course_id: usize) -> Result<Vec<CourseExercise>, CoreError> {
        if self.token.is_none() {
            return Err(CoreError::NotLoggedIn);
        }
        self.exercises(course_id)
    }

    /// Sends the given submission as a paste.
    ///
    /// # Errors
    /// Returns an error if there's some problem reaching the API, or if the API returns an error.
    ///
    /// # Examples
    /// ```rust,no_run
    /// use tmc_langs_core::{TmcCore, Language};
    /// use url::Url;
    /// use std::path::Path;
    ///
    /// let core = TmcCore::new_in_config("https://tmc.mooc.fi".to_string(), "some_client".to_string(), "some_version".to_string()).unwrap();
    /// // authenticate
    /// let course_details = core.get_course_details(600).unwrap();
    /// let submission_url = &course_details.exercises[0].return_url;
    /// let submission_url = Url::parse(&submission_url).unwrap();
    /// let new_submission = core.paste(
    ///     submission_url,
    ///     Path::new("./exercises/python/123"),
    ///     Some("my python solution".to_string()),
    ///     Some(Language::Eng)).unwrap();
    /// ```
    pub fn paste(
        &self,
        submission_url: Url,
        submission_path: &Path,
        paste_message: Option<String>,
        locale: Option<Language>,
    ) -> Result<NewSubmission, CoreError> {
        // compress
        let compressed = task_executor::compress_project(submission_path)?;
        let mut file = NamedTempFile::new().map_err(CoreError::TempFile)?;
        file.write_all(&compressed)
            .map_err(|e| CoreError::Tmc(FileIo::FileWrite(file.path().to_path_buf(), e).into()))?;

        self.post_submission_to_paste(submission_url, file.path(), paste_message, locale)
    }

    /// Checks the coding style for the project.
    ///
    /// # Errors
    /// Returns an error if no matching language plugin for the project is found,
    /// or if the plugin returns an error while trying to run the style check.
    ///
    /// # Examples
    /// ```rust,no_run
    /// use tmc_langs_core::{TmcCore, Language};
    /// use std::path::Path;
    ///
    /// let core = TmcCore::new_in_config("https://tmc.mooc.fi".to_string(), "some_client".to_string(), "some_version".to_string()).unwrap();
    /// // authenticate
    /// let validation_result = core.run_checkstyle(Path::new("./exercises/python/123"), Language::Eng).unwrap();
    /// match validation_result {
    ///     Some(validation_result) => if let Some(validation_errors) = validation_result.validation_errors {
    ///         println!("found validation errors: {:?}", validation_errors);
    ///     } else {
    ///         println!("no errors");
    ///     }
    ///     None => println!("no style checks"),
    /// }
    /// ```
    pub fn run_checkstyle(
        &self,
        path: &Path,
        locale: Language,
    ) -> Result<Option<ValidationResult>, CoreError> {
        Ok(task_executor::run_check_code_style(path, locale)?)
    }

    /// Runs tests for the project.
    ///
    /// # Errors
    /// Returns an error if no matching language plugin for the project is found,
    /// or if the plugin returns an error while trying to run the tests.
    pub fn run_tests(&self, path: &Path) -> Result<RunResult, CoreError> {
        Ok(task_executor::run_tests(path)?)
    }

    /// Sends feedback.
    ///
    /// # Errors
    /// Returns an error if there's some problem reaching the API, or if the API returns an error.
    pub fn send_feedback(
        &self,
        feedback_url: Url,
        feedback: Vec<FeedbackAnswer>,
    ) -> Result<SubmissionFeedbackResponse, CoreError> {
        self.post_feedback(feedback_url, feedback)
    }

    #[deprecated = "unimplemented"]
    pub fn send_snapshot_events(&self) {
        unimplemented!()
    }

    /// Sends the submission to the server.
    ///
    /// # Errors
    /// Returns an error if there's some problem reaching the API, or if the API returns an error.
    /// The method compresses the submission and writes it into a temporary archive, which may fail.
    pub fn submit(
        &self,
        submission_url: Url,
        submission_path: &Path,
        locale: Option<Language>,
    ) -> Result<NewSubmission, CoreError> {
        self.report_progress(
            "Compressing submission...".to_string(),
            StatusType::Processing,
            0.0,
        );
        let compressed = task_executor::compress_project(submission_path)?;
        let mut file = NamedTempFile::new().map_err(CoreError::TempFile)?;
        file.write_all(&compressed)
            .map_err(|e| CoreError::Tmc(FileIo::FileWrite(file.path().to_path_buf(), e).into()))?;
        self.report_progress(
            "Compressed submission. Posting submission...".to_string(),
            StatusType::Sending,
            0.5,
        );

        let result = self.post_submission(submission_url, file.path(), locale);
        self.report_complete("Submission finished!".to_string());
        result
    }

    pub fn reset(&self, exercise_id: usize, exercise_path: &Path) -> Result<(), CoreError> {
        // clear out the exercise directory
        if exercise_path.exists() {
            let mut tries = 0;
            for entry in WalkDir::new(exercise_path).min_depth(1) {
                let entry = entry?;
                log::debug!("{:?}", entry);
                // windows sometimes fails due to files being in use, retry a few times
                // todo: handle properly
                if entry.path().is_dir() {
                    while let Err(err) = file_util::remove_dir_all(entry.path()) {
                        tries += 1;
                        if tries > 8 {
                            return Err(CoreError::FileIo(err));
                        }
                        thread::sleep(Duration::from_secs(1));
                    }
                } else {
                    while let Err(err) = file_util::remove_file(entry.path()) {
                        tries += 1;
                        if tries > 8 {
                            return Err(CoreError::FileIo(err));
                        }
                        thread::sleep(Duration::from_secs(1));
                    }
                }
            }
        }
        self.download_or_update_exercises(vec![(exercise_id, exercise_path)])
    }

    pub fn download_old_submission(
        &self,
        submission_id: usize,
        target: &Path,
    ) -> Result<(), CoreError> {
        self.download_submission(submission_id, target)
    }

    pub fn get_exercise_submissions_for_current_user(
        &self,
        exercise_id: usize,
    ) -> Result<Vec<Submission>, CoreError> {
        self.exercise_submissions_for_current_user(exercise_id)
    }

    pub fn wait_for_submission(
        &self,
        submission_url: &str,
    ) -> Result<SubmissionFinished, CoreError> {
        let mut previous_status = None;
        loop {
            match self.check_submission(submission_url)? {
                SubmissionProcessingStatus::Finished(f) => {
                    self.report_complete("Submission finished processing!".to_string());
                    return Ok(*f);
                }
                SubmissionProcessingStatus::Processing(p) => {
                    if p.status == SubmissionStatus::Hidden {
                        // hidden status, return constructed status
                        self.report_complete(
                            "Submission status hidden, stopping waiting.".to_string(),
                        );
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
                                SandboxStatus::Created => self.report_progress(
                                    "Created on sandbox".to_string(),
                                    StatusType::WaitingForResults,
                                    0.25,
                                ),
                                SandboxStatus::SendingToSandbox => self.report_progress(
                                    "Sending to sandbox".to_string(),
                                    StatusType::WaitingForResults,
                                    0.5,
                                ),
                                SandboxStatus::ProcessingOnSandbox => self.report_progress(
                                    "Processing on sandbox".to_string(),
                                    StatusType::WaitingForResults,
                                    0.75,
                                ),
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
    /// and finds new or updated exercises.
    /// If an exercise's id is not found in the checksum map, it is considered new.
    /// If an id is found, it is compared to the current one. If they are different,
    /// it is considered updated.
    ///
    /// # Errors
    /// Returns an error if there's some problem reaching the API, or if the API returns an error.
    ///
    /// # Examples
    /// ```rust,no_run
    /// use tmc_langs_core::TmcCore;
    ///
    /// let core = TmcCore::new_in_config("https://tmc.mooc.fi".to_string(), "some_client".to_string(), "some_version".to_string()).unwrap();
    /// // authenticate
    /// let mut checksums = std::collections::HashMap::new();
    /// checksums.insert(1234, "exercisechecksum".to_string());
    /// let update_result = core.get_exercise_updates(600, checksums).unwrap();
    /// ```
    pub fn get_exercise_updates(
        &self,
        course_id: usize,
        checksums: HashMap<usize, String>,
    ) -> Result<UpdateResult, CoreError> {
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
        Ok(UpdateResult {
            created: new_exercises,
            updated: updated_exercises,
        })
    }

    /// Mark the review as read on the server.
    ///
    /// # Errors
    /// Returns an error if there's some problem reaching the API, or if the API returns an error.
    pub fn mark_review_as_read(&self, review_update_url: String) -> Result<(), CoreError> {
        self.mark_review(review_update_url, true)
    }

    /// Fetches all reviews.
    ///
    /// # Errors
    /// Returns an error if there's some problem reaching the API, or if the API returns an error.
    pub fn get_unread_reviews(&self, reviews_url: Url) -> Result<Vec<Review>, CoreError> {
        self.get_json_from_url(reviews_url)
    }

    /// Request code review.
    ///
    /// # Errors
    /// Returns an error if there's some problem reaching the API, or if the API returns an error.
    /// The method compresses the project and writes a temporary archive, which may fail.
    pub fn request_code_review(
        &self,
        submission_url: Url,
        submission_path: &Path,
        message_for_reviewer: String,
        locale: Option<Language>,
    ) -> Result<NewSubmission, CoreError> {
        // compress
        let compressed = task_executor::compress_project(submission_path)?;
        let mut file = NamedTempFile::new().map_err(CoreError::TempFile)?;
        file.write_all(&compressed)
            .map_err(|e| CoreError::Tmc(FileIo::FileWrite(file.path().to_path_buf(), e).into()))?;

        self.post_submission_for_review(submission_url, file.path(), message_for_reviewer, locale)
    }

    /// Downloads the model solution from the given url.
    ///
    /// # Errors
    /// Returns an error if there's some problem reaching the API, or if the API returns an error.
    /// The method extracts the downloaded model solution archive, which may fail.
    pub fn download_model_solution(
        &self,
        solution_download_url: Url,
        target: &Path,
    ) -> Result<(), CoreError> {
        let zip_file = NamedTempFile::new().map_err(CoreError::TempFile)?;
        self.download_from(solution_download_url, zip_file.path())?;
        task_executor::extract_project(zip_file.path(), target, false)?;
        Ok(())
    }

    /// Checks the status of a submission on the server.
    ///
    /// # Errors
    /// Returns an error if the core has not been authenticated,
    /// or if there's some problem reaching the API, or if the API returns an error.
    pub fn check_submission(
        &self,
        submission_url: &str,
    ) -> Result<SubmissionProcessingStatus, CoreError> {
        if self.token.is_none() {
            return Err(CoreError::NotLoggedIn);
        }

        let url = Url::parse(submission_url)
            .map_err(|e| CoreError::UrlParse(submission_url.to_string(), e))?;
        let res: Response<SubmissionProcessingStatus> = self.get_json_from_url(url)?;
        let res = res.into_result()?;
        Ok(res)
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
        let mut core = TmcCore::new_in_config(
            local_server.to_string(),
            "some_client".to_string(),
            "some_ver".to_string(),
        )
        .unwrap();
        core.authenticate("client_name", "email".to_string(), "password".to_string())
            .unwrap();
        (core, local_server)
    }

    #[test]
    fn gets_organizations() {
        let (core, _addr) = init();
        let _m = mock("GET", "/api/v8/org.json")
            .match_header("client", "some_client")
            .match_header("client_version", "some_ver")
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
    fn downloads_or_update_exercises() {
        let (core, _addr) = init();
        let _m = mock("GET", "/api/v8/core/exercises/1234/download")
            .match_header("client", "some_client")
            .match_header("client_version", "some_ver")
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
    fn gets_course_details() {
        let (core, _addr) = init();
        let _m = mock("GET", "/api/v8/core/courses/1234")
            .match_header("client", "some_client")
            .match_header("client_version", "some_ver")
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
    fn lists_courses() {
        let (core, _addr) = init();
        let _m = mock("GET", "/api/v8/core/org/slug/courses")
            .match_header("client", "some_client")
            .match_header("client_version", "some_ver")
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
    fn pastes_with_comment() {
        let (core, url) = init();
        let submission_url = Url::parse(&format!("{}/submission", url)).unwrap();
        let _m = mock("POST", "/submission")
            .match_header("client", "some_client")
            .match_header("client_version", "some_ver")
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
            .paste(
                submission_url,
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
    fn runs_checkstyle() {
        // todo, just calls task executor
    }

    #[test]
    fn runs_tests() {
        // todo, just calls task executor
    }

    #[test]
    fn sends_feedback() {
        let (core, url) = init();
        let feedback_url = Url::parse(&format!("{}/feedback", url)).unwrap();
        let _m = mock("POST", "/feedback")
            .match_header("client", "some_client")
            .match_header("client_version", "some_ver")
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
                feedback_url,
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
        let (core, url) = init();
        let submission_url = Url::parse(&format!("{}/submission", url)).unwrap();
        let _m = mock("POST", "/submission")
            .match_header("client", "some_client")
            .match_header("client_version", "some_ver")
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

        let new_submission = core
            .submit(
                submission_url,
                Path::new("tests/data/exercise"),
                Some(Language::Eng),
            )
            .unwrap();
        assert_eq!(
            new_submission.submission_url,
            "https://tmc.mooc.fi/api/v8/core/submissions/7400888"
        );
    }

    #[test]
    fn gets_exercise_updates() {
        let (core, _addr) = init();
        let _m = mock("GET", "/api/v8/core/courses/1234")
            .match_header("client", "some_client")
            .match_header("client_version", "some_ver")
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
        let update_result = core.get_exercise_updates(1234, checksums).unwrap();

        assert_eq!(update_result.created.len(), 1);
        assert_eq!(update_result.created[0].id, 34);

        assert_eq!(update_result.updated.len(), 1);
        assert_eq!(update_result.updated[0].checksum, "zz");
    }

    //#[test]
    fn _marks_review_as_read() {
        // todo
        let (core, addr) = init();
        let update_url = Url::parse(&addr).unwrap().join("update-url").unwrap();

        let _m = mock("POST", "/update-url.json").create();

        core.mark_review_as_read(update_url.to_string()).unwrap();
    }

    #[test]
    fn gets_unread_reviews() {
        let (core, addr) = init();
        let reviews_url = Url::parse(&format!("{}/reviews", addr)).unwrap();
        let _m = mock("GET", "/reviews")
            .match_header("client", "some_client")
            .match_header("client_version", "some_ver")
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

        let reviews = core.get_unread_reviews(reviews_url).unwrap();
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].submission_id, "5678");
    }

    #[test]
    fn requests_code_review() {
        let (core, url) = init();
        let submission_url = Url::parse(&format!("{}/submission", url)).unwrap();
        let _m = mock("POST", "/submission")
            .match_header("client", "some_client")
            .match_header("client_version", "some_ver")
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
                submission_url,
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
        let (core, addr) = init();
        let solution_url = Url::parse(&format!("{}/solution", addr)).unwrap();
        let _m = mock("GET", "/solution")
            .match_header("client", "some_client")
            .match_header("client_version", "some_ver")
            .with_body_from_file(Path::new("tests/data/81842.zip"))
            .create();

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("temp");
        assert!(!target.exists());
        core.download_model_solution(solution_url, &target).unwrap();
        assert!(target.join("src/main/java/Hiekkalaatikko.java").exists());
    }

    #[test]
    fn checks_submission_processing() {
        let (core, addr) = init();
        let _m = mock("GET", "/submission-url")
            .match_header("client", "some_client")
            .match_header("client_version", "some_ver")
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
    fn checks_submission_finished() {
        let (core, addr) = init();
        let _m = mock("GET", "/submission-url")
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
        let submission_processing_status = core.check_submission(&sub_url).unwrap();
        match submission_processing_status {
            SubmissionProcessingStatus::Finished(f) => {
                assert_eq!(f.all_tests_passed, Some(true));
            }
            SubmissionProcessingStatus::Processing(_) => panic!("wrong status"),
        }
    }

    #[test]
    fn status_serde() {
        let p = StatusUpdate {
            finished: false,
            message: "submitting...".to_string(),
            percent_done: 0.5,
            status_type: StatusType::Sending,
        };
        assert_eq!(
            r#"{"finished":false,"message":"submitting...","percent_done":0.5,"status_type":"sending"}"#,
            serde_json::to_string(&p).unwrap()
        );
        let f = StatusUpdate {
            finished: true,
            message: "done".to_string(),
            percent_done: 1.0,
            status_type: StatusType::Finished,
        };
        assert_eq!(
            r#"{"finished":true,"message":"done","percent_done":1.0,"status_type":"finished"}"#,
            serde_json::to_string(&f).unwrap()
        );
    }

    #[test]
    fn multi_step_progress() {
        use std::sync::{Arc, Mutex};

        let (mut core, _) = init();
        let report = Arc::new(Mutex::default());

        let report_clone = Arc::clone(&report);
        core.set_progress_report(move |rep| {
            log::debug!("got {:#?}", rep);
            let report = Arc::clone(&report_clone);
            *report.lock().unwrap() = Some(rep);
            Ok(())
        });
        core.increment_progress_steps();
        core.increment_progress_steps();

        core.report_progress(
            "msg".to_string(),
            StatusType::DownloadingExercise {
                id: 0,
                path: PathBuf::new(),
            },
            0.2,
        );
        let err = f64::EPSILON;
        assert!((report.lock().unwrap().as_ref().unwrap().percent_done - (0.2 / 3.0)).abs() < err);
        core.report_progress(
            "msg".to_string(),
            StatusType::DownloadingExercise {
                id: 0,
                path: PathBuf::new(),
            },
            0.8,
        );
        assert!((report.lock().unwrap().as_ref().unwrap().percent_done - (0.8 / 3.0)).abs() < err);
        core.report_complete("msg".to_string());
        assert!((report.lock().unwrap().as_ref().unwrap().percent_done - (1.0 / 3.0)).abs() < err);
        core.report_complete("msg".to_string());
        assert!((report.lock().unwrap().as_ref().unwrap().percent_done - (2.0 / 3.0)).abs() < err);
        core.report_progress(
            "msg".to_string(),
            StatusType::DownloadingExercise {
                id: 0,
                path: PathBuf::new(),
            },
            0.5,
        );
        assert!((report.lock().unwrap().as_ref().unwrap().percent_done - (2.5 / 3.0)).abs() < err);
        core.report_complete("msg".to_string());
        assert!((report.lock().unwrap().as_ref().unwrap().percent_done - 1.0).abs() < err);
    }
}
