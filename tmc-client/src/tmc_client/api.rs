//! Contains and additional impl for TmcClient for calling the TMC Server API.

use crate::error::ClientError;
use crate::response::ErrorResponse;
use crate::{
    Course, CourseData, CourseDataExercise, CourseDataExercisePoint, CourseDetails, CourseExercise,
    ExerciseDetails, ExercisesDetails, FeedbackAnswer, NewSubmission, Organization, Review,
    Submission, SubmissionFeedbackResponse, TmcClient, User,
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
use tmc_langs_plugins::Language;
use tmc_langs_util::{file_util, FileError};
use url::Url;

/// Provides a wrapper for reqwest Response's json that deserializes into Response<T> and converts it into a result
trait ClientExt {
    fn json_res<T: DeserializeOwned>(self) -> Result<T, ClientError>;
}

impl ClientExt for ReqwestResponse {
    fn json_res<T: DeserializeOwned>(self) -> Result<T, ClientError> {
        let url = self.url().clone();
        let status = self.status();
        if status.is_success() {
            // expecting successful response
            Ok(self
                .json()
                .map_err(|e| ClientError::HttpJsonResponse(url.clone(), e))?)
        } else if let Ok(err) = self.json::<ErrorResponse>() {
            // failed and got an error json
            let error = match (err.error, err.errors) {
                (Some(err), Some(errs)) => format!("{}, {}", err, errs.join(",")),
                (Some(err), None) => err,
                (None, Some(errs)) => errs.join(","),
                _ => "".to_string(),
            };
            Err(ClientError::HttpError {
                url,
                status,
                error,
                obsolete_client: err.obsolete_client,
            })
        } else {
            // failed and failed to parse error json, return generic HTTP error
            Err(ClientError::HttpError {
                url,
                status,
                error: status.to_string(),
                obsolete_client: false,
            })
        }
    }
}

/// Provides a convenience function for adding a token and client headers
trait GetExt {
    fn tmc_headers(self, client: &TmcClient) -> RequestBuilder;
}

impl GetExt for RequestBuilder {
    fn tmc_headers(self, client: &TmcClient) -> RequestBuilder {
        let request = self.query(&[
            ("client", &client.0.client_name),
            ("client_version", &client.0.client_version),
        ]);
        if let Some(token) = client.0.token.as_ref() {
            request.bearer_auth(token.access_token().secret())
        } else {
            request
        }
    }
}

#[allow(dead_code)]
impl TmcClient {
    // convenience function
    fn get_json<T: DeserializeOwned>(&self, url_tail: &str) -> Result<T, ClientError> {
        let url = self
            .0
            .api_url
            .join(url_tail)
            .map_err(|e| ClientError::UrlParse(url_tail.to_string(), e))?;
        self.get_json_from_url(url, &[])
    }
    // convenience function
    fn get_json_with_params<T: DeserializeOwned>(
        &self,
        url_tail: &str,
        params: &[(String, String)],
    ) -> Result<T, ClientError> {
        let url = self
            .0
            .api_url
            .join(url_tail)
            .map_err(|e| ClientError::UrlParse(url_tail.to_string(), e))?;
        self.get_json_from_url(url, params)
    }
    // convenience function
    pub fn get_json_from_url<T: DeserializeOwned>(
        &self,
        url: Url,
        params: &[(String, String)],
    ) -> Result<T, ClientError> {
        log::debug!("get {}", url);
        self.0
            .client
            .get(url.clone())
            .tmc_headers(self)
            .query(params)
            .send()
            .map_err(|e| ClientError::ConnectionError(Method::GET, url, e))?
            .json_res()
    }

    fn download(&self, url_tail: &str, target: &Path) -> Result<(), ClientError> {
        let url = self
            .0
            .api_url
            .join(url_tail)
            .map_err(|e| ClientError::UrlParse(url_tail.to_string(), e))?;

        // download zip
        let mut target_file = file_util::create_file(target)?;
        log::debug!("downloading {}", url);
        let mut response = self
            .0
            .client
            .get(url.clone())
            .tmc_headers(self)
            .send()
            .map_err(|e| ClientError::ConnectionError(Method::GET, url.clone(), e))?;

        // check for HTTP error
        // todo use same code here and in json_res
        if !response.status().is_success() {
            let status = response.status();
            if let Ok(err) = response.json::<ErrorResponse>() {
                // failed and got an error json
                let error = match (err.error, err.errors) {
                    (Some(err), Some(errs)) => format!("{}, {}", err, errs.join(",")),
                    (Some(err), None) => err,
                    (None, Some(errs)) => errs.join(","),
                    _ => "".to_string(),
                };
                Err(ClientError::HttpError {
                    url,
                    status,
                    error,
                    obsolete_client: err.obsolete_client,
                })
            } else {
                // failed and failed to parse error json, return generic HTTP error
                Err(ClientError::HttpError {
                    url,
                    status,
                    error: status.to_string(),
                    obsolete_client: false,
                })
            }
        } else {
            response
                .copy_to(&mut target_file)
                .map_err(|e| ClientError::HttpWriteResponse(target.to_path_buf(), e))?;
            Ok(())
        }
    }

    pub(crate) fn download_from(&self, url: Url, target: &Path) -> Result<(), ClientError> {
        // download zip
        let mut target_file = file_util::create_file(target)?;
        log::debug!("downloading {}", url);
        self.0
            .client
            .get(url.clone())
            .tmc_headers(self)
            .send()
            .map_err(|e| ClientError::ConnectionError(Method::GET, url, e))?
            .copy_to(&mut target_file)
            .map_err(|e| ClientError::HttpWriteResponse(target.to_path_buf(), e))?;
        Ok(())
    }

    pub(super) fn user(&self, user_id: usize) -> Result<User, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let url_tail = format!("users/{}", user_id);
        self.get_json(&url_tail)
    }

    pub(super) fn user_current(&self) -> Result<User, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let url_tail = "users/current";
        self.get_json(url_tail)
    }

    pub(super) fn basic_info_by_usernames(&self) -> Result<Vec<User>, ClientError> {
        todo!("needs admin")
    }

    pub(super) fn basic_info_by_emails(&self) -> Result<Vec<User>, ClientError> {
        todo!("needs admin")
    }

    pub(super) fn course(&self, course_id: usize) -> Result<CourseData, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let url_tail = format!("courses/{}", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn course_by_name(
        &self,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<CourseData, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let url_tail = format!(
            "org/{}/courses/{}",
            percent_encode(organization_slug),
            percent_encode(course_name)
        );
        self.get_json(&url_tail)
    }

    pub(super) fn course_points(
        &self,
        course_id: usize,
    ) -> Result<CourseDataExercisePoint, ClientError> {
        let url_tail = format!("courses/{}/points", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn exercise_points(
        &self,
        course_id: usize,
        exercise_name: &str,
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
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
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
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
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
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
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        let url_tail = format!("courses/{}/users/{}/points", course_id, user_id);
        self.get_json(&url_tail)
    }

    pub(super) fn course_points_for_current_user(
        &self,
        course_id: usize,
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        let url_tail = format!("courses/{}/users/current/points", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn course_points_by_name(
        &self,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
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
    ) -> Result<(), ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
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
    ) -> Result<(), ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
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
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
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
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
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
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
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
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
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
    ) -> Result<Vec<Submission>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let url_tail = format!("courses/{}/submissions", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn course_submissions_in_last_hour(
        &self,
        course_id: usize,
    ) -> Result<Vec<usize>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let url_tail = format!("courses/{}/submissions/last_hour", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn course_submissions_for_user(
        &self,
        course_id: usize,
        user_id: usize,
    ) -> Result<Vec<Submission>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let url_tail = format!("courses/{}/users/{}/submissions", course_id, user_id);
        self.get_json(&url_tail)
    }

    pub(super) fn course_submissions_for_current_user(
        &self,
        course_id: usize,
    ) -> Result<Vec<Submission>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let url_tail = format!("courses/{}/users/current/submissions", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn exercise_submissions_for_user(
        &self,
        exercise_id: usize,
        user_id: usize,
    ) -> Result<Vec<Submission>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let url_tail = format!("exercises/{}/users/{}/submissions", exercise_id, user_id);
        self.get_json(&url_tail)
    }

    pub(super) fn exercise_submissions_for_current_user(
        &self,
        exercise_id: usize,
    ) -> Result<Vec<Submission>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let url_tail = format!("exercises/{}/users/current/submissions", exercise_id);
        self.get_json(&url_tail)
    }

    pub(super) fn exercise_submissions_by_name(
        &self,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<Vec<Submission>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
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
    ) -> Result<Vec<Submission>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
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
    ) -> Result<Vec<Submission>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let url_tail = format!(
            "org/{}/courses/{}/users/current/submissions",
            percent_encode(organization_slug),
            percent_encode(course_name),
        );
        self.get_json(&url_tail)
    }

    pub(super) fn exercises(&self, course_id: usize) -> Result<Vec<CourseExercise>, ClientError> {
        let url_tail = format!("courses/{}/exercises", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn exercises_by_name(
        &self,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<Vec<CourseDataExercise>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
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
    ) -> Result<(), ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let url_tail = format!(
            "org/{}/courses/{}/exercises/{}/download",
            percent_encode(organization_slug),
            percent_encode(course_name),
            percent_encode(exercise_name)
        );
        self.download(&url_tail, target)
    }

    pub(super) fn organizations(&self) -> Result<Vec<Organization>, ClientError> {
        let url_tail = "org.json";
        self.get_json(url_tail)
    }

    pub(super) fn organization(
        &self,
        organization_slug: &str,
    ) -> Result<Organization, ClientError> {
        let url_tail = format!("org/{}.json", organization_slug);
        self.get_json(&url_tail)
    }

    pub(super) fn core_course(&self, course_id: usize) -> Result<CourseDetails, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let url_tail = format!("core/courses/{}", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn reviews(&self, course_id: usize) -> Result<Vec<Review>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let url_tail = format!("core/courses/{}/reviews", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn review(
        &self,
        course_id: usize,
        review_id: usize,
    ) -> Result<Vec<Review>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let _url_tail = format!("core/courses/{}/reviews/{}", course_id, review_id);
        // self.get_json(&url_tail)
        todo!("does not appear to function")
    }

    pub(super) fn unlock(&self, course_id: usize) -> Result<(), ClientError> {
        let _url_tail = format!("core/courses/{}", course_id);
        todo!("needs admin?");
    }

    pub fn download_exercise(&self, exercise_id: usize, target: &Path) -> Result<(), ClientError> {
        let url_tail = format!("core/exercises/{}/download", exercise_id);
        self.download(&url_tail, target)
    }

    pub(super) fn core_exercise(&self, exercise_id: usize) -> Result<ExerciseDetails, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let url_tail = format!("core/exercises/{}", exercise_id);
        self.get_json(&url_tail)
    }

    pub(super) fn core_exercise_details(
        &self,
        exercise_ids: &[usize],
    ) -> Result<Vec<ExercisesDetails>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let url_tail = "core/exercises/details";
        let exercise_ids = (
            "ids".to_string(),
            exercise_ids
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(","),
        );

        // returns map with result in key "exercises"
        let res: HashMap<String, Vec<ExercisesDetails>> =
            self.get_json_with_params(&url_tail, &[exercise_ids])?;
        if let Some((_, val)) = res.into_iter().next() {
            // just return whatever value is found first
            return Ok(val);
        }
        Err(ClientError::MissingDetailsValue)
    }

    pub(super) fn download_solution(
        &self,
        exercise_id: usize,
        target: &Path,
    ) -> Result<(), ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let url_tail = format!("core/exercises/{}/solution/download", exercise_id);
        self.download(&url_tail, target)
    }

    pub(super) fn post_submission(
        &self,
        submission_url: Url,
        submission: &Path,
        locale: Option<Language>,
    ) -> Result<NewSubmission, ClientError> {
        self.post_submission_with_params(submission_url, submission, None, locale)
    }

    pub(super) fn post_submission_to_paste(
        &self,
        submission_url: Url,
        submission: &Path,
        paste_message: Option<String>,
        locale: Option<Language>,
    ) -> Result<NewSubmission, ClientError> {
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
    ) -> Result<NewSubmission, ClientError> {
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
    ) -> Result<NewSubmission, ClientError> {
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
            .map_err(|e| {
                ClientError::FileError(FileError::FileOpen(submission.to_path_buf(), e))
            })?;

        if let Some(params) = params {
            for (key, val) in params {
                form = form.text(key, val);
            }
        }

        log::debug!("posting submission to {}", submission_url);
        let res: NewSubmission = self
            .0
            .client
            .post(submission_url.clone())
            .multipart(form)
            .tmc_headers(self)
            .send()
            .map_err(|e| ClientError::ConnectionError(Method::POST, submission_url, e))?
            .json_res()?;
        log::debug!("received {:?}", res);
        Ok(res)
    }

    pub(super) fn organization_courses(
        &self,
        organization_slug: &str,
    ) -> Result<Vec<Course>, ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let url_tail = format!("core/org/{}/courses", organization_slug);
        self.get_json(&url_tail)
    }

    pub(super) fn download_submission(
        &self,
        submission_id: usize,
        target: &Path,
    ) -> Result<(), ClientError> {
        if self.0.token.is_none() {
            return Err(ClientError::NotLoggedIn);
        }
        let url_tail = format!("core/submissions/{}/download", submission_id);
        self.download(&url_tail, target)
    }

    pub(super) fn post_feedback(
        &self,
        feedback_url: Url,
        feedback: Vec<FeedbackAnswer>,
    ) -> Result<SubmissionFeedbackResponse, ClientError> {
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

        self.0
            .client
            .post(feedback_url.clone())
            .multipart(form)
            .tmc_headers(self)
            .send()
            .map_err(|e| ClientError::ConnectionError(Method::POST, feedback_url, e))?
            .json_res()
    }

    pub(super) fn post_review(
        &self,
        submission_id: usize,
        review_body: &str,
        review_points: &str,
    ) -> Result<(), ClientError> {
        // needs auth?
        let url_tail = format!("core/submissions/{}/reviews", submission_id);
        let url = self
            .0
            .api_url
            .join(&url_tail)
            .map_err(|e| ClientError::UrlParse(url_tail, e))?;

        log::debug!("posting {}", url);
        let res: Value = self
            .0
            .client
            .post(url.clone())
            .query(&[("review[review_body]", review_body)])
            .query(&[("review[points]", review_points)])
            .tmc_headers(self)
            .send()
            .map_err(|e| ClientError::ConnectionError(Method::POST, url, e))?
            .json_res()?;
        log::trace!("received {:?}", res);
        Ok(())
    }

    pub(super) fn mark_review(
        &self,
        review_update_url: String,
        read: bool,
    ) -> Result<(), ClientError> {
        // needs auth?
        let url = format!("{}.json", review_update_url);
        let url = Url::parse(&url).map_err(|e| ClientError::UrlParse(url, e))?;

        let mut form = Form::new().text("_method", "put");
        if read {
            form = form.text("mark_as_read", "1");
        } else {
            form = form.text("mark_as_unread", "1");
        }

        self.0
            .client
            .post(url.clone())
            .multipart(form)
            .send()
            .map_err(|e| ClientError::ConnectionError(Method::POST, url, e))?
            .json_res()
    }
}

fn percent_encode(target: &str) -> String {
    percent_encoding::utf8_percent_encode(target, percent_encoding::NON_ALPHANUMERIC).to_string()
}
