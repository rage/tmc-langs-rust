use crate::error::{CoreError, Result};
use crate::response::Response;
use crate::tmc_core::Token;
use crate::{
    Course, CourseDetails, CourseExercise, ExerciseDetails, FeedbackAnswer, NewSubmission,
    NuCourse, NuCourseExercise, NuExercisePoint, Organization, Review, Submission,
    SubmissionFeedbackResponse, TmcCore, User,
};

use oauth2::{prelude::SecretNewType, TokenResponse};
use reqwest::blocking::{multipart::Form, RequestBuilder, Response as ReqwestResponse};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use tempfile::NamedTempFile;
use tmc_langs_util::task_executor;
use url::Url;

/// Provides a wrapper for reqwest Response's json that deserializes into Response<T> and converts it into a result
trait CoreExt {
    fn json_res<T: DeserializeOwned>(self) -> Result<T>;
    fn check_error(self, url: Url) -> Result<Self>
    where
        Self: Sized;
}

impl CoreExt for ReqwestResponse {
    #[cfg(not(test))]
    fn json_res<T: DeserializeOwned>(self) -> Result<T> {
        let res: Response<T> = self.json()?;
        res.into_result()
    }

    // logs received JSON for easier debugging in tests
    #[cfg(test)]
    fn json_res<T: DeserializeOwned>(self) -> Result<T> {
        let res: Value = self.json()?;
        log::debug!("JSON {}", res);
        let res: Response<T> = serde_json::from_value(res).unwrap();
        res.into_result()
    }

    fn check_error(self, url: Url) -> Result<Self> {
        let status = self.status();
        if status.is_success() {
            Ok(self)
        } else {
            log::error!("HTTP Error: {}", self.text()?);
            Err(CoreError::HttpStatus(url, status))
        }
    }
}

/// Provides a convenience function for adding a token
trait GetExt {
    fn authenticate(self, token: &Option<Token>) -> RequestBuilder;
}

impl GetExt for RequestBuilder {
    fn authenticate(self, token: &Option<Token>) -> RequestBuilder {
        if let Some(token) = token {
            self.bearer_auth(token.access_token().secret())
        } else {
            self
        }
    }
}

impl TmcCore {
    // convenience function
    fn get_json<T: DeserializeOwned>(&self, url_tail: &str) -> Result<T> {
        let url = self.api_url.join(url_tail)?;
        log::debug!("get {}", url);
        self.client
            .get(url.clone())
            .authenticate(&self.token)
            .send()?
            .check_error(url)?
            .json_res()
    }

    fn download(&self, url_tail: &str, target: &Path) -> Result<()> {
        let url = self.api_url.join(&url_tail)?;

        // download zip
        let mut target_file =
            File::create(target).map_err(|e| CoreError::FileCreate(target.to_path_buf(), e))?;
        log::debug!("downloading {}", url);
        self.client
            .get(url.clone())
            .authenticate(&self.token)
            .send()?
            .check_error(url)?
            .copy_to(&mut target_file)?;
        Ok(())
    }

    pub fn download_from(&self, url: Url, target: &Path) -> Result<()> {
        // download zip
        let mut target_file =
            File::create(target).map_err(|e| CoreError::FileCreate(target.to_path_buf(), e))?;
        log::debug!("downloading {}", url);
        self.client
            .get(url.clone())
            .authenticate(&self.token)
            .send()?
            .check_error(url)?
            .copy_to(&mut target_file)?;
        Ok(())
    }

    pub(super) fn user(&self, user_id: usize) -> Result<User> {
        let url_tail = format!("users/{}", user_id);
        self.get_json(&url_tail)
    }

    pub(super) fn user_current(&self) -> Result<User> {
        let url_tail = "users/current";
        self.get_json(url_tail)
    }

    pub(super) fn basic_info_by_usernames(&self) -> Result<Vec<User>> {
        todo!("needs admin")
    }

    pub(super) fn basic_info_by_emails(&self) -> Result<Vec<User>> {
        todo!("needs admin")
    }

    pub(super) fn course(&self, course_id: usize) -> Result<NuCourse> {
        let url_tail = format!("courses/{}", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn course_by_name(
        &self,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<NuCourse> {
        let url_tail = format!(
            "org/{}/courses/{}",
            percent_encode(organization_slug),
            percent_encode(course_name)
        );
        self.get_json(&url_tail)
    }

    pub(super) fn course_points(&self, course_id: usize) -> Result<()> {
        let url_tail = format!("courses/{}/points", course_id);
        todo!("times out")
    }

    pub(super) fn exercise_points(
        &self,
        course_id: usize,
        exercise_name: &str,
    ) -> Result<Vec<NuExercisePoint>> {
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
    ) -> Result<Vec<NuExercisePoint>> {
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
    ) -> Result<Vec<NuExercisePoint>> {
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
    ) -> Result<Vec<NuExercisePoint>> {
        let url_tail = format!("courses/{}/users/{}/points", course_id, user_id);
        self.get_json(&url_tail)
    }

    pub(super) fn course_points_for_current_user(
        &self,
        course_id: usize,
    ) -> Result<Vec<NuExercisePoint>> {
        let url_tail = format!("courses/{}/users/current/points", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn course_points_by_name(
        &self,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<Vec<NuExercisePoint>> {
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
    ) -> Result<()> {
        todo!("This feature is only for MOOC-organization's 2019 programming MOOC");
        let url_tail = format!(
            "org/{}/courses/{}/eligible_students",
            percent_encode(organization_slug),
            percent_encode(course_name)
        );
        self.get_json(&url_tail)
    }

    pub(super) fn exercise_points_by_name(
        &self,
        organization_slug: &str,
        course_name: &str,
        exercise_name: &str,
    ) -> Result<()> {
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
    ) -> Result<Vec<NuExercisePoint>> {
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
    ) -> Result<Vec<NuExercisePoint>> {
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
    ) -> Result<Vec<NuExercisePoint>> {
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
    ) -> Result<Vec<NuExercisePoint>> {
        let url_tail = format!(
            "org/{}/courses/{}/users/current/points",
            percent_encode(organization_slug),
            percent_encode(course_name),
        );
        self.get_json(&url_tail)
    }

    pub(super) fn course_submissions(&self, course_id: usize) -> Result<Vec<Submission>> {
        let url_tail = format!("courses/{}/submissions", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn course_submissions_in_last_hour(
        &self,
        course_id: usize,
    ) -> Result<Vec<Submission>> {
        let url_tail = format!("courses/{}/submissions/last_hour", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn course_submissions_for_user(
        &self,
        course_id: usize,
        user_id: usize,
    ) -> Result<Vec<Submission>> {
        let url_tail = format!("courses/{}/users/{}/submissions", course_id, user_id);
        self.get_json(&url_tail)
    }

    pub(super) fn course_submissions_for_current_user(
        &self,
        course_id: usize,
    ) -> Result<Vec<Submission>> {
        let url_tail = format!("courses/{}/users/current/submissions", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn exercise_submissions_for_user(
        &self,
        exercise_id: usize,
        user_id: usize,
    ) -> Result<Vec<Submission>> {
        let url_tail = format!("exercises/{}/users/{}/submissions", exercise_id, user_id);
        self.get_json(&url_tail)
    }

    pub(super) fn exercise_submissions_for_current_user(
        &self,
        exercise_id: usize,
    ) -> Result<Vec<Submission>> {
        let url_tail = format!("exercises/{}/users/current/submissions", exercise_id);
        self.get_json(&url_tail)
    }

    pub(super) fn exercise_submissions_by_name(
        &self,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<Vec<Submission>> {
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
    ) -> Result<Vec<Submission>> {
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
    ) -> Result<Vec<Submission>> {
        let url_tail = format!(
            "org/{}/courses/{}/users/current/submissions",
            percent_encode(organization_slug),
            percent_encode(course_name),
        );
        self.get_json(&url_tail)
    }

    pub(super) fn exercises(&self, course_id: usize) -> Result<Vec<CourseExercise>> {
        let url_tail = format!("courses/{}/exercises", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn exercises_by_name(
        &self,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<Vec<NuCourseExercise>> {
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
    ) -> Result<()> {
        let url_tail = format!(
            "org/{}/courses/{}/exercises/{}/download",
            percent_encode(organization_slug),
            percent_encode(course_name),
            percent_encode(exercise_name)
        );
        self.download(&url_tail, target)
    }

    pub(super) fn organizations(&self) -> Result<Vec<Organization>> {
        let url_tail = "org.json";
        self.get_json(url_tail)
    }

    pub(super) fn organization(&self, organization_slug: &str) -> Result<Organization> {
        let url_tail = format!("org/{}.json", organization_slug);
        self.get_json(&url_tail)
    }

    pub(super) fn core_course(&self, course_id: usize) -> Result<CourseDetails> {
        let url_tail = format!("core/courses/{}", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn reviews(&self, course_id: usize) -> Result<Vec<Review>> {
        let url_tail = format!("core/courses/{}/reviews", course_id);
        self.get_json(&url_tail)
    }

    pub(super) fn review(&self, course_id: usize, review_id: usize) -> Result<Vec<Review>> {
        let url_tail = format!("core/courses/{}/reviews/{}", course_id, review_id);
        self.get_json(&url_tail)
    }

    pub(super) fn unlock(&self, course_id: usize) -> Result<()> {
        todo!("needs admin?");
        let url_tail = format!("core/courses/{}", course_id);
    }

    pub(super) fn download_exercise(&self, exercise_id: usize, target: &Path) -> Result<()> {
        let url_tail = format!("core/exercises/{}/download", exercise_id);
        self.download(&url_tail, target)
    }

    pub(super) fn core_exercise(&self, exercise_id: usize) -> Result<ExerciseDetails> {
        let url_tail = format!("core/exercises/{}", exercise_id);
        self.get_json(&url_tail)
    }

    pub(super) fn download_solution(&self, exercise_id: usize, target: &Path) -> Result<()> {
        let url_tail = format!("core/exercises/{}/solution/download", exercise_id);
        self.download(&url_tail, target)
    }

    pub(super) fn post_submission(
        &self,
        submission_url: Url,
        submission: &Path,
    ) -> Result<NewSubmission> {
        self.post_submission_with_params(submission_url, submission, None)
    }

    pub(super) fn post_submission_to_paste(
        &self,
        submission_url: Url,
        submission: &Path,
        paste_message: String,
    ) -> Result<NewSubmission> {
        let mut params = HashMap::new();
        params.insert("paste".to_string(), "1".to_string());
        params.insert("message_for_paste".to_string(), paste_message);
        self.post_submission_with_params(submission_url, submission, Some(params))
    }

    pub(super) fn post_submission_for_review(
        &self,
        submission_url: Url,
        submission: &Path,
        message_for_reviewer: String,
    ) -> Result<NewSubmission> {
        let mut params = HashMap::new();
        params.insert("request_review".to_string(), "1".to_string());
        params.insert("message_for_reviewer".to_string(), message_for_reviewer);
        self.post_submission_with_params(submission_url, submission, Some(params))
    }

    fn post_submission_with_params(
        &self,
        submission_url: Url,
        submission: &Path,
        params: Option<HashMap<String, String>>,
    ) -> Result<NewSubmission> {
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
        let mut form = Form::new()
            .file("submission[file]", submission)
            .map_err(|e| CoreError::FileOpen(submission.to_path_buf(), e))?;

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
            .authenticate(&self.token)
            .send()?
            .check_error(submission_url)?
            .json_res()?;
        log::debug!("received {:?}", res);
        Ok(res)
    }

    pub(super) fn organization_courses(&self, organization_slug: &str) -> Result<Vec<Course>> {
        let url_tail = format!("core/org/{}/courses", organization_slug);
        self.get_json(&url_tail)
    }

    pub(super) fn download_submission(&self, submission_id: usize, target: &Path) -> Result<()> {
        let url_tail = format!("core/submissions/{}/download", submission_id);
        self.download(&url_tail, target)
    }

    pub(super) fn post_feedback(
        &self,
        feedback_url: Url,
        feedback: Vec<FeedbackAnswer>,
    ) -> Result<SubmissionFeedbackResponse> {
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
            .authenticate(&self.token)
            .send()?
            .check_error(feedback_url)?
            .json_res()
    }

    pub(super) fn post_review(
        &self,
        submission_id: usize,
        review_body: &str,
        review_points: &str,
    ) -> Result<()> {
        let url_tail = format!("core/submissions/{}/reviews", submission_id);
        let url = self.api_url.join(&url_tail)?;

        log::debug!("posting {}", url);
        let res: Value = self
            .client
            .post(url.clone())
            .query(&[("review[review_body]", review_body)])
            .query(&[("review[points]", review_points)])
            .authenticate(&self.token)
            .send()?
            .check_error(url)?
            .json_res()?;
        log::trace!("received {:?}", res);
        Ok(())
    }

    pub(super) fn mark_review(&self, review_update_url: String, read: bool) -> Result<()> {
        let url = Url::parse(&format!("{}.json", review_update_url))?;

        let mut form = Form::new().text("_method", "put");
        if read {
            form = form.text("mark_as_read", "1");
        } else {
            form = form.text("mark_as_unread", "1");
        }

        self.client
            .post(url.clone())
            .multipart(form)
            .send()?
            .check_error(url)?
            .json_res()
    }
}

fn percent_encode(target: &str) -> String {
    percent_encoding::utf8_percent_encode(target, percent_encoding::NON_ALPHANUMERIC).to_string()
}
