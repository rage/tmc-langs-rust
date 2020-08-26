//! Contains and additional impl for TmcCore for calling the TMC Server API.

use crate::error::CoreError;
use crate::response::Response;
use crate::{
    Course, CourseData, CourseDataExercise, CourseDataExercisePoint, CourseDetails, CourseExercise,
    ExerciseDetails, FeedbackAnswer, NewSubmission, Organization, Review, Submission,
    SubmissionFeedbackResponse, TmcCore, User,
};

use oauth2::TokenResponse;
use reqwest::{
    blocking::{multipart::Form, RequestBuilder, Response as ReqwestResponse},
    Method,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;
use tmc_langs_util::{file_util, FileIo, Language};
use url::Url;

/// Provides a wrapper for reqwest Response's json that deserializes into Response<T> and converts it into a result
trait CoreExt {
    fn json_res<T: DeserializeOwned>(self) -> Result<T, CoreError>;
    fn check_error(self, url: Url) -> Result<Self, CoreError>
    where
        Self: Sized;
}

impl CoreExt for ReqwestResponse {
    #[cfg(not(test))]
    fn json_res<T: DeserializeOwned>(self) -> Result<T, CoreError> {
        let res: Response<T> = self.json().map_err(CoreError::HttpJsonResponse)?;
        res.into_result()
    }

    // logs received JSON for easier debugging in tests
    #[cfg(test)]
    fn json_res<T: DeserializeOwned>(self) -> Result<T, CoreError> {
        let res: Value = self.json().map_err(CoreError::HttpJsonResponse)?;
        log::debug!("JSON {}", res);
        let res: Response<T> = serde_json::from_value(res).unwrap();
        res.into_result()
    }

    fn check_error(self, url: Url) -> Result<Self, CoreError> {
        let status = self.status();
        if status.is_success() {
            Ok(self)
        } else {
            let text = self.text().unwrap_or_default();
            // todo: clean the parsing
            let parsed = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|ok| {
                    ok.as_object().and_then(|obj|
                        // parses either the error string or errors string array
                        if let Some(error) = obj.get("error").and_then(|e| e.as_str()) {
                            Some(error.to_string())
                        } else if let Some(errors) = obj.get("errors").and_then(|e| e.as_array()) {
                            let errors = errors
                                .iter()
                                .filter_map(|e| e.as_str())
                                .collect::<Vec<_>>()
                                .join(". ");
                            Some(errors)
                        } else {
                            None
                        })
                })
                .unwrap_or(text);
            Err(CoreError::HttpError(url, status, parsed))
        }
    }
}

/// Provides a convenience function for adding a token and client headers
trait GetExt {
    fn core_headers(self, core: &TmcCore) -> RequestBuilder;
}

impl GetExt for RequestBuilder {
    fn core_headers(self, core: &TmcCore) -> RequestBuilder {
        let request = self
            .header("client", &core.client_name)
            .header("client_version", &core.client_version);
        if let Some(token) = core.token.as_ref() {
            request.bearer_auth(token.access_token().secret())
        } else {
            request
        }
    }
}

#[allow(dead_code)]
impl TmcCore {
    // convenience function
    fn get_json<T: DeserializeOwned>(&self, url_tail: &str) -> Result<T, CoreError> {
        let url = self
            .api_url
            .join(url_tail)
            .map_err(|e| CoreError::UrlParse(url_tail.to_string(), e))?;
        self.get_json_from_url(url)
    }
    // convenience function
    pub fn get_json_from_url<T: DeserializeOwned>(&self, url: Url) -> Result<T, CoreError> {
        log::debug!("get {}", url);
        self.client
            .get(url.clone())
            .core_headers(self)
            .send()
            .map_err(|e| CoreError::ConnectionError(Method::GET, url.clone(), e))?
            .check_error(url)?
            .json_res()
    }

    fn download(&self, url_tail: &str, target: &Path) -> Result<(), CoreError> {
        let url = self
            .api_url
            .join(url_tail)
            .map_err(|e| CoreError::UrlParse(url_tail.to_string(), e))?;

        // download zip
        let mut target_file = file_util::create_file(target)?;
        log::debug!("downloading {}", url);
        self.client
            .get(url.clone())
            .core_headers(self)
            .send()
            .map_err(|e| CoreError::ConnectionError(Method::GET, url.clone(), e))?
            .check_error(url)?
            .copy_to(&mut target_file)
            .map_err(|e| CoreError::HttpWriteResponse(target.to_path_buf(), e))?;
        Ok(())
    }

    pub(crate) fn download_from(&self, url: Url, target: &Path) -> Result<(), CoreError> {
        // download zip
        let mut target_file = file_util::create_file(target)?;
        log::debug!("downloading {}", url);
        self.client
            .get(url.clone())
            .core_headers(self)
            .send()
            .map_err(|e| CoreError::ConnectionError(Method::GET, url.clone(), e))?
            .check_error(url)?
            .copy_to(&mut target_file)
            .map_err(|e| CoreError::HttpWriteResponse(target.to_path_buf(), e))?;
        Ok(())
    }

    pub(super) fn user(&self, user_id: usize) -> Result<User, CoreError> {
        let url_tail = format!("users/{}", user_id);
        self.get_json(&url_tail)
    }

    pub(super) fn user_current(&self) -> Result<User, CoreError> {
        let url_tail = "users/current";
        self.get_json(url_tail)
    }

    pub(super) fn basic_info_by_usernames(&self) -> Result<Vec<User>, CoreError> {
        todo!("needs admin")
    }

    pub(super) fn basic_info_by_emails(&self) -> Result<Vec<User>, CoreError> {
        todo!("needs admin")
    }

    pub(super) fn course(&self, course_id: usize) -> Result<CourseData, CoreError> {
        let url_tail = format!("courses/{}", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn course_by_name(
        &self,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<CourseData, CoreError> {
        let url_tail = format!(
            "org/{}/courses/{}",
            percent_encode(organization_slug),
            percent_encode(course_name)
        );
        self.get_json(&url_tail)
    }

    pub(super) fn course_points(&self, course_id: usize) -> Result<(), CoreError> {
        let _url_tail = format!("courses/{}/points", course_id);
        todo!("times out")
    }

    pub(super) fn exercise_points(
        &self,
        course_id: usize,
        exercise_name: &str,
    ) -> Result<Vec<CourseDataExercisePoint>, CoreError> {
        let url_tail = format!(
            "courses/{}/exercises/{}/points",
            course_id,
            percent_encode(exercise_name)
        );
        self.get_json(&url_tail)
    }

    pub(super) fn exercise_points_for_user(
        &self,
        course_id: usize,
        exercise_name: &str,
        user_id: usize,
    ) -> Result<Vec<CourseDataExercisePoint>, CoreError> {
        let url_tail = format!(
            "courses/{}/exercises/{}/users/{}/points",
            course_id,
            percent_encode(exercise_name),
            user_id
        );
        self.get_json(&url_tail)
    }

    pub(super) fn exercise_points_for_current_user(
        &self,
        course_id: usize,
        exercise_name: &str,
    ) -> Result<Vec<CourseDataExercisePoint>, CoreError> {
        let url_tail = format!(
            "courses/{}/exercises/{}/users/current/points",
            course_id,
            percent_encode(exercise_name)
        );
        self.get_json(&url_tail)
    }

    pub(super) fn course_points_for_user(
        &self,
        course_id: usize,
        user_id: usize,
    ) -> Result<Vec<CourseDataExercisePoint>, CoreError> {
        let url_tail = format!("courses/{}/users/{}/points", course_id, user_id);
        self.get_json(&url_tail)
    }

    pub(super) fn course_points_for_current_user(
        &self,
        course_id: usize,
    ) -> Result<Vec<CourseDataExercisePoint>, CoreError> {
        let url_tail = format!("courses/{}/users/current/points", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn course_points_by_name(
        &self,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<Vec<CourseDataExercisePoint>, CoreError> {
        let url_tail = format!(
            "org/{}/courses/{}/points",
            percent_encode(organization_slug),
            percent_encode(course_name)
        );
        self.get_json(&url_tail)
    }

    pub(super) fn eligible_students(
        &self,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<(), CoreError> {
        let _url_tail = format!(
            "org/{}/courses/{}/eligible_students",
            percent_encode(organization_slug),
            percent_encode(course_name)
        );
        todo!("This feature is only for MOOC-organization's 2019 programming MOOC");
    }

    pub(super) fn exercise_points_by_name(
        &self,
        organization_slug: &str,
        course_name: &str,
        exercise_name: &str,
    ) -> Result<(), CoreError> {
        let url_tail = format!(
            "org/{}/courses/{}/exercises/{}/points",
            percent_encode(organization_slug),
            percent_encode(course_name),
            percent_encode(exercise_name)
        );
        self.get_json(&url_tail)
    }

    pub(super) fn exercise_points_by_name_for_current_user(
        &self,
        organization_slug: &str,
        course_name: &str,
        exercise_name: &str,
    ) -> Result<Vec<CourseDataExercisePoint>, CoreError> {
        let url_tail = format!(
            "org/{}/courses/{}/exercises/{}/users/current/points",
            percent_encode(organization_slug),
            percent_encode(course_name),
            percent_encode(exercise_name)
        );
        self.get_json(&url_tail)
    }

    pub(super) fn exercise_points_by_name_for_user(
        &self,
        organization_slug: &str,
        course_name: &str,
        exercise_name: &str,
        user_id: usize,
    ) -> Result<Vec<CourseDataExercisePoint>, CoreError> {
        let url_tail = format!(
            "org/{}/courses/{}/exercises/{}/users/{}/points",
            percent_encode(organization_slug),
            percent_encode(course_name),
            percent_encode(exercise_name),
            user_id
        );
        self.get_json(&url_tail)
    }

    pub(super) fn course_points_by_name_for_user(
        &self,
        organization_slug: &str,
        course_name: &str,
        user_id: usize,
    ) -> Result<Vec<CourseDataExercisePoint>, CoreError> {
        let url_tail = format!(
            "org/{}/courses/{}/users/{}/points",
            percent_encode(organization_slug),
            percent_encode(course_name),
            user_id
        );
        self.get_json(&url_tail)
    }

    pub(super) fn course_points_by_name_for_current_user(
        &self,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<Vec<CourseDataExercisePoint>, CoreError> {
        let url_tail = format!(
            "org/{}/courses/{}/users/current/points",
            percent_encode(organization_slug),
            percent_encode(course_name),
        );
        self.get_json(&url_tail)
    }

    pub(super) fn course_submissions(
        &self,
        course_id: usize,
    ) -> Result<Vec<Submission>, CoreError> {
        let url_tail = format!("courses/{}/submissions", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn course_submissions_in_last_hour(
        &self,
        course_id: usize,
    ) -> Result<Vec<Submission>, CoreError> {
        let url_tail = format!("courses/{}/submissions/last_hour", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn course_submissions_for_user(
        &self,
        course_id: usize,
        user_id: usize,
    ) -> Result<Vec<Submission>, CoreError> {
        let url_tail = format!("courses/{}/users/{}/submissions", course_id, user_id);
        self.get_json(&url_tail)
    }

    pub(super) fn course_submissions_for_current_user(
        &self,
        course_id: usize,
    ) -> Result<Vec<Submission>, CoreError> {
        let url_tail = format!("courses/{}/users/current/submissions", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn exercise_submissions_for_user(
        &self,
        exercise_id: usize,
        user_id: usize,
    ) -> Result<Vec<Submission>, CoreError> {
        let url_tail = format!("exercises/{}/users/{}/submissions", exercise_id, user_id);
        self.get_json(&url_tail)
    }

    pub(super) fn exercise_submissions_for_current_user(
        &self,
        exercise_id: usize,
    ) -> Result<Vec<Submission>, CoreError> {
        let url_tail = format!("exercises/{}/users/current/submissions", exercise_id);
        self.get_json(&url_tail)
    }

    pub(super) fn exercise_submissions_by_name(
        &self,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<Vec<Submission>, CoreError> {
        let url_tail = format!(
            "org/{}/courses/{}/submissions",
            percent_encode(organization_slug),
            percent_encode(course_name)
        );
        self.get_json(&url_tail)
    }

    pub(super) fn exercise_submissions_by_name_for_user(
        &self,
        organization_slug: &str,
        course_name: &str,
        user_id: usize,
    ) -> Result<Vec<Submission>, CoreError> {
        let url_tail = format!(
            "org/{}/courses/{}/users/{}/submissions",
            percent_encode(organization_slug),
            percent_encode(course_name),
            user_id
        );
        self.get_json(&url_tail)
    }

    pub(super) fn exercise_submissions_by_name_for_currrent_user(
        &self,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<Vec<Submission>, CoreError> {
        let url_tail = format!(
            "org/{}/courses/{}/users/current/submissions",
            percent_encode(organization_slug),
            percent_encode(course_name),
        );
        self.get_json(&url_tail)
    }

    pub(super) fn exercises(&self, course_id: usize) -> Result<Vec<CourseExercise>, CoreError> {
        let url_tail = format!("courses/{}/exercises", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn exercises_by_name(
        &self,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<Vec<CourseDataExercise>, CoreError> {
        let url_tail = format!(
            "org/{}/courses/{}/exercises",
            percent_encode(organization_slug),
            percent_encode(course_name)
        );
        self.get_json(&url_tail)
    }

    pub(super) fn download_exercise_by_name(
        &self,
        organization_slug: &str,
        course_name: &str,
        exercise_name: &str,
        target: &Path,
    ) -> Result<(), CoreError> {
        let url_tail = format!(
            "org/{}/courses/{}/exercises/{}/download",
            percent_encode(organization_slug),
            percent_encode(course_name),
            percent_encode(exercise_name)
        );
        self.download(&url_tail, target)
    }

    pub(super) fn organizations(&self) -> Result<Vec<Organization>, CoreError> {
        let url_tail = "org.json";
        self.get_json(url_tail)
    }

    pub(super) fn organization(&self, organization_slug: &str) -> Result<Organization, CoreError> {
        let url_tail = format!("org/{}.json", organization_slug);
        self.get_json(&url_tail)
    }

    pub(super) fn core_course(&self, course_id: usize) -> Result<CourseDetails, CoreError> {
        let url_tail = format!("core/courses/{}", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn reviews(&self, course_id: usize) -> Result<Vec<Review>, CoreError> {
        let url_tail = format!("core/courses/{}/reviews", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn review(
        &self,
        course_id: usize,
        review_id: usize,
    ) -> Result<Vec<Review>, CoreError> {
        let url_tail = format!("core/courses/{}/reviews/{}", course_id, review_id);
        self.get_json(&url_tail)
    }

    pub(super) fn unlock(&self, course_id: usize) -> Result<(), CoreError> {
        let _url_tail = format!("core/courses/{}", course_id);
        todo!("needs admin?");
    }

    pub(super) fn download_exercise(
        &self,
        exercise_id: usize,
        target: &Path,
    ) -> Result<(), CoreError> {
        let url_tail = format!("core/exercises/{}/download", exercise_id);
        self.download(&url_tail, target)
    }

    pub(super) fn core_exercise(&self, exercise_id: usize) -> Result<ExerciseDetails, CoreError> {
        let url_tail = format!("core/exercises/{}", exercise_id);
        self.get_json(&url_tail)
    }

    pub(super) fn download_solution(
        &self,
        exercise_id: usize,
        target: &Path,
    ) -> Result<(), CoreError> {
        let url_tail = format!("core/exercises/{}/solution/download", exercise_id);
        self.download(&url_tail, target)
    }

    pub(super) fn post_submission(
        &self,
        submission_url: Url,
        submission: &Path,
        locale: Option<Language>,
    ) -> Result<NewSubmission, CoreError> {
        self.post_submission_with_params(submission_url, submission, None, locale)
    }

    pub(super) fn post_submission_to_paste(
        &self,
        submission_url: Url,
        submission: &Path,
        paste_message: Option<String>,
        locale: Option<Language>,
    ) -> Result<NewSubmission, CoreError> {
        let mut params = HashMap::new();
        params.insert("paste".to_string(), "1".to_string());
        params.insert(
            "message_for_paste".to_string(),
            paste_message.unwrap_or_default(), // TODO: can this field be ignored?
        );
        self.post_submission_with_params(submission_url, submission, Some(params), locale)
    }

    pub(super) fn post_submission_for_review(
        &self,
        submission_url: Url,
        submission: &Path,
        message_for_reviewer: String,
        locale: Option<Language>,
    ) -> Result<NewSubmission, CoreError> {
        let mut params = HashMap::new();
        params.insert("request_review".to_string(), "1".to_string());
        params.insert("message_for_reviewer".to_string(), message_for_reviewer);
        self.post_submission_with_params(submission_url, submission, Some(params), locale)
    }

    fn post_submission_with_params(
        &self,
        submission_url: Url,
        submission: &Path,
        params: Option<HashMap<String, String>>,
        locale: Option<Language>,
    ) -> Result<NewSubmission, CoreError> {
        if self.token.is_none() {
            return Err(CoreError::AuthRequired);
        }

        /*
        let url = self
            .api_url
            .join(&format!("core/exercises/{}/submissions", exercise_id))
            .unwrap();
        */

        // send
        let mut form = Form::new();
        if let Some(locale) = locale {
            form = form.text("error_msg_locale", locale.to_string()) // TODO: verify server accepts 639-3
        }
        form = form
            .text(
                "client_time",
                SystemTime::UNIX_EPOCH.elapsed()?.as_secs().to_string(),
            )
            .text(
                "client_nanotime",
                SystemTime::UNIX_EPOCH.elapsed()?.as_nanos().to_string(),
            )
            .file("submission[file]", submission)
            .map_err(|e| CoreError::FileIo(FileIo::FileOpen(submission.to_path_buf(), e)))?;

        if let Some(params) = params {
            for (key, val) in params {
                form = form.text(key, val);
            }
        }

        log::debug!("posting submission to {}", submission_url);
        let res: NewSubmission = self
            .client
            .post(submission_url.clone())
            .multipart(form)
            .core_headers(self)
            .send()
            .map_err(|e| CoreError::ConnectionError(Method::POST, submission_url.clone(), e))?
            .check_error(submission_url)?
            .json_res()?;
        log::debug!("received {:?}", res);
        Ok(res)
    }

    pub(super) fn organization_courses(
        &self,
        organization_slug: &str,
    ) -> Result<Vec<Course>, CoreError> {
        let url_tail = format!("core/org/{}/courses", organization_slug);
        self.get_json(&url_tail)
    }

    pub(super) fn download_submission(
        &self,
        submission_id: usize,
        target: &Path,
    ) -> Result<(), CoreError> {
        let url_tail = format!("core/submissions/{}/download", submission_id);
        self.download(&url_tail, target)
    }

    pub(super) fn post_feedback(
        &self,
        feedback_url: Url,
        feedback: Vec<FeedbackAnswer>,
    ) -> Result<SubmissionFeedbackResponse, CoreError> {
        // let url_tail = format!("core/submissions/{}/feedback", submission_id);
        // let url = self.api_url.join(&url_tail)?;

        log::debug!("posting feedback to {}", feedback_url);
        let mut form = Form::new();
        for (i, answer) in feedback.into_iter().enumerate() {
            form = form.text(
                format!("answers[{}][question_id]", i),
                answer.question_id.to_string(),
            );
            form = form.text(format!("answers[{}][answer]", i), answer.answer);
        }

        self.client
            .post(feedback_url.clone())
            .multipart(form)
            .core_headers(self)
            .send()
            .map_err(|e| CoreError::ConnectionError(Method::POST, feedback_url.clone(), e))?
            .check_error(feedback_url)?
            .json_res()
    }

    pub(super) fn post_review(
        &self,
        submission_id: usize,
        review_body: &str,
        review_points: &str,
    ) -> Result<(), CoreError> {
        let url_tail = format!("core/submissions/{}/reviews", submission_id);
        let url = self
            .api_url
            .join(&url_tail)
            .map_err(|e| CoreError::UrlParse(url_tail, e))?;

        log::debug!("posting {}", url);
        let res: Value = self
            .client
            .post(url.clone())
            .query(&[("review[review_body]", review_body)])
            .query(&[("review[points]", review_points)])
            .core_headers(self)
            .send()
            .map_err(|e| CoreError::ConnectionError(Method::POST, url.clone(), e))?
            .check_error(url)?
            .json_res()?;
        log::trace!("received {:?}", res);
        Ok(())
    }

    pub(super) fn mark_review(
        &self,
        review_update_url: String,
        read: bool,
    ) -> Result<(), CoreError> {
        let url = format!("{}.json", review_update_url);
        let url = Url::parse(&url).map_err(|e| CoreError::UrlParse(url, e))?;

        let mut form = Form::new().text("_method", "put");
        if read {
            form = form.text("mark_as_read", "1");
        } else {
            form = form.text("mark_as_unread", "1");
        }

        self.client
            .post(url.clone())
            .multipart(form)
            .send()
            .map_err(|e| CoreError::ConnectionError(Method::POST, url.clone(), e))?
            .check_error(url)?
            .json_res()
    }
}

fn percent_encode(target: &str) -> String {
    percent_encoding::utf8_percent_encode(target, percent_encoding::NON_ALPHANUMERIC).to_string()
}
