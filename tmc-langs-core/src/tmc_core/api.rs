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
use std::io::Write;
use std::path::Path;
use tempfile::NamedTempFile;
use tmc_langs_util::task_executor;

/// Provides a wrapper for reqwest Response's json that deserializes into Response<T> and converts it into a result
trait JsonExt {
    fn json_res<T: DeserializeOwned>(self) -> Result<T>;
}

impl JsonExt for ReqwestResponse {
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
            .get(url)
            .authenticate(&self.token)
            .send()?
            .json_res()
    }

    fn download(&self, url_tail: &str, target: &Path) -> Result<()> {
        let url = self.api_url.join(&url_tail)?;

        // download zip
        let mut target_file = File::create(target)?;
        log::debug!("downloading {}", url);
        let mut res = self.client.get(url).authenticate(&self.token).send()?;
        // TODO: improve error handling
        if !res.status().is_success() {
            if let Ok(value) = res.json::<Value>() {
                log::error!("HTTP Error: {}", value);
            } else {
                log::error!("HTTP Error");
            }
            return Err(CoreError::HttpStatus);
        }
        res.copy_to(&mut target_file)?; // write response to target file
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
        let url_tail = format!("core/courses/{}", course_id);
        todo!()
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
        exercise_id: usize,
        submission: &Path,
    ) -> Result<NewSubmission> {
        self.post_submission_with_params(exercise_id, submission, None)
    }

    pub(super) fn post_submission_to_paste(
        &self,
        exercise_id: usize,
        submission: &Path,
        paste_message: String,
    ) -> Result<NewSubmission> {
        let mut params = HashMap::new();
        params.insert("paste".to_string(), "1".to_string());
        params.insert("message_for_paste".to_string(), paste_message);
        self.post_submission_with_params(exercise_id, submission, Some(params))
    }

    pub(super) fn post_submission_for_review(
        &self,
        exercise_id: usize,
        submission: &Path,
        message_for_reviewer: String,
    ) -> Result<NewSubmission> {
        let mut params = HashMap::new();
        params.insert("request_review".to_string(), "1".to_string());
        params.insert("message_for_reviewer".to_string(), message_for_reviewer);
        self.post_submission_with_params(exercise_id, submission, Some(params))
    }

    // TODO: test with params
    pub(super) fn post_submission_with_params(
        &self,
        exercise_id: usize,
        submission: &Path,
        params: Option<HashMap<String, String>>,
    ) -> Result<NewSubmission> {
        if self.token.is_none() {
            return Err(CoreError::AuthRequired);
        }

        // compress
        let compressed = task_executor::compress_project(submission)?;
        let mut file = NamedTempFile::new()?;
        file.write_all(&compressed)?;

        let url = self
            .api_url
            .join(&format!("core/exercises/{}/submissions", exercise_id))
            .unwrap();

        // send
        let mut form = Form::new().file("submission[file]", file.path())?;
        if let Some(params) = params {
            for (key, val) in params {
                form = form.text(key, val);
            }
        }

        log::debug!("posting {}", url);
        let res: NewSubmission = self
            .client
            .post(url)
            .multipart(form)
            .authenticate(&self.token)
            .send()?
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
        submission_id: usize,
        feedback: Vec<FeedbackAnswer>,
    ) -> Result<SubmissionFeedbackResponse> {
        let url_tail = format!("core/submissions/{}/feedback", submission_id);
        let url = self.api_url.join(&url_tail)?;

        log::debug!("posting {}", url);
        let mut req = self.client.post(url);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token.access_token().secret());
        }
        for (i, answer) in feedback.into_iter().enumerate() {
            req = req
                .query(&[(format!("answers[{}][question_id]", i), answer.question_id)])
                .query(&[(format!("answers[{}][answer]", i), answer.answer)]);
        }
        let res: SubmissionFeedbackResponse = req.send()?.json_res()?;
        log::trace!("received {:?}", res);
        Ok(res)
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
            .post(url)
            .query(&[("review[review_body]", review_body)])
            .query(&[("review[points]", review_points)])
            .authenticate(&self.token)
            .send()?
            .json_res()?;
        log::trace!("received {:?}", res);
        todo!()
    }
}

fn percent_encode(target: &str) -> String {
    percent_encoding::utf8_percent_encode(target, percent_encoding::NON_ALPHANUMERIC).to_string()
}
#[cfg(test)]
mod test {
    use super::*;

    const ROOT_URL: &'static str = "https://tmc.mooc.fi";

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
    fn user() {
        let core = authenticated_core();
        let _user = core.user(3232).unwrap();
    }

    #[test]
    #[ignore]
    fn user_current() {
        let core = authenticated_core();
        let _user = core.user_current().unwrap();
        panic!()
    }

    #[test]
    #[ignore]
    fn course() {
        let core = authenticated_core();
        let _course = core.course(600).unwrap();
        panic!()
    }

    #[test]
    #[ignore]
    fn course_by_name() {
        let core = authenticated_core();
        let _course = core.course_by_name("mooc", "java-programming-i").unwrap();
    }

    #[test]
    #[ignore]
    fn exercise_points() {
        let core = authenticated_core();
        let _points = core
            .exercise_points(600, "part01-Part01_02.AdaLovelace")
            .unwrap();
    }

    #[test]
    #[ignore]
    fn exercise_points_for_user() {
        let core = authenticated_core();
        let _points = core
            .exercise_points_for_user(600, "part01-Part01_02.AdaLovelace", 3232)
            .unwrap();
    }

    #[test]
    #[ignore]
    fn exercise_points_for_current_user() {
        let core = authenticated_core();
        let _points = core
            .exercise_points_for_current_user(600, "part01-Part01_02.AdaLovelace")
            .unwrap();
    }

    #[test]
    #[ignore]
    fn course_points_for_user() {
        let core = authenticated_core();
        let _points = core.course_points_for_user(600, 3232).unwrap();
    }

    #[test]
    #[ignore]
    fn course_points_for_current_user() {
        let core = authenticated_core();
        let _points = core.course_points_for_current_user(600).unwrap();
    }

    #[test]
    #[ignore]
    fn course_points_by_name() {
        let core = authenticated_core();
        let _points = core
            .course_points_by_name("mooc", "java-programming-i")
            .unwrap();
        todo!("timeout")
    }

    #[test]
    #[ignore]
    fn eligible_students() {
        let core = authenticated_core();
        let _points = core
            .eligible_students("mooc", "java-programming-i")
            .unwrap();
        todo!("This feature is only for MOOC-organization's 2019 programming MOOC")
    }

    #[test]
    #[ignore]
    fn exercise_points_by_name() {
        let core = authenticated_core();
        let _points = core
            .exercise_points_by_name("mooc", "java-programming-i", "part01-Part01_02.AdaLovelace")
            .unwrap();
        todo!("times out")
    }

    #[test]
    #[ignore]
    fn exercise_points_by_name_for_current_user() {
        let core = authenticated_core();
        let _points = core
            .exercise_points_by_name_for_current_user(
                "mooc",
                "java-programming-i",
                "part01-Part01_02.AdaLovelace",
            )
            .unwrap();
    }

    #[test]
    #[ignore]
    fn exercise_points_by_name_for_user() {
        let core = authenticated_core();
        let _points = core
            .exercise_points_by_name_for_user(
                "mooc",
                "java-programming-i",
                "part01-Part01_02.AdaLovelace",
                3232,
            )
            .unwrap();
    }

    #[test]
    #[ignore]
    fn course_points_by_name_for_user() {
        let core = authenticated_core();
        let _points = core
            .course_points_by_name_for_user("mooc", "java-programming-i", 3232)
            .unwrap();
    }

    #[test]
    #[ignore]
    fn course_points_by_name_for_current_user() {
        let core = authenticated_core();
        let _points = core
            .course_points_by_name_for_current_user("mooc", "java-programming-i")
            .unwrap();
    }

    #[test]
    #[ignore]
    fn course_submissions() {
        let core = authenticated_core();
        let _submissions = core.course_submissions(600).unwrap();
        todo!("timeout")
    }

    #[test]
    #[ignore]
    fn course_submissions_in_last_hour() {
        let core = authenticated_core();
        let _submissions = core.course_submissions_in_last_hour(600).unwrap();
        todo!("access denied")
    }

    #[test]
    #[ignore]
    fn course_submissions_for_user() {
        let core = authenticated_core();
        let _submissions = core.course_submissions_for_user(600, 3232).unwrap();
    }

    #[test]
    #[ignore]
    fn course_submissions_for_current_user() {
        let core = authenticated_core();
        let _submissions = core.course_submissions_for_current_user(600).unwrap();
    }

    #[test]
    #[ignore]
    fn exercise_submissions_for_user() {
        let core = authenticated_core();
        let _submissions = core.exercise_submissions_for_user(83114, 3232).unwrap();
    }

    #[test]
    #[ignore]
    fn exercise_submissions_for_current_user() {
        let core = authenticated_core();
        let _submissions = core.exercise_submissions_for_current_user(83114).unwrap();
    }

    #[test]
    #[ignore]
    fn exercise_submissions_by_name() {
        let core = authenticated_core();
        let _submissions = core
            .exercise_submissions_by_name("mooc", "java-programming-i")
            .unwrap();
        todo!("times out")
    }

    #[test]
    #[ignore]
    fn exercise_submissions_by_name_for_user() {
        let core = authenticated_core();
        let _submissions = core
            .exercise_submissions_by_name_for_user("mooc", "java-programming-i", 3232)
            .unwrap();
    }

    #[test]
    #[ignore]
    fn exercise_submissions_by_name_for_currrent_user() {
        let core = authenticated_core();
        let _submissions = core
            .exercise_submissions_by_name_for_currrent_user("mooc", "java-programming-i")
            .unwrap();
    }

    #[test]
    #[ignore]
    fn exercises() {
        let core = authenticated_core();
        let _exercises = core.exercises(600).unwrap();
    }

    #[test]
    #[ignore]
    fn exercises_by_name() {
        let core = authenticated_core();
        let _exercises = core
            .exercises_by_name("mooc", "java-programming-i")
            .unwrap();
    }

    #[test]
    #[ignore]
    fn download_exercise_by_name() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("temp");
        assert!(!path.exists());

        let core = authenticated_core();
        core.download_exercise_by_name(
            "mooc",
            "java-programming-i",
            "part01-Part01_02.AdaLovelace",
            &path,
        )
        .unwrap();
        assert!(path.exists());
    }

    #[test]
    #[ignore]
    fn organizations() {
        let core = authenticated_core();
        let _organizations = core.organizations().unwrap();
    }

    #[test]
    #[ignore]
    fn organization() {
        let core = authenticated_core();
        let _organization = core.organization("mooc").unwrap();
    }

    #[test]
    #[ignore]
    fn core_course() {
        let core = authenticated_core();
        let _course = core.core_course(600).unwrap();
    }

    #[test]
    #[ignore]
    fn reviews() {
        let core = authenticated_core();
        let _reviews = core.reviews(600).unwrap();
        todo!("not verified")
    }

    #[test]
    #[ignore]
    fn review() {
        let core = authenticated_core();
        let _reviews = core.review(600, 0).unwrap();
        todo!("not verified")
    }

    #[test]
    #[ignore]
    fn unlock() {
        let core = authenticated_core();
        core.unlock(600).unwrap();
    }

    #[test]
    #[ignore]
    fn download_exercise() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("temp");
        assert!(!path.exists());

        let core = authenticated_core();
        core.download_exercise(83114, &path).unwrap();
        assert!(path.exists());
    }

    #[test]
    #[ignore]
    fn core_exercise() {
        let core = authenticated_core();
        let _exercise = core.core_exercise(83114).unwrap();
    }

    #[test]
    #[ignore]
    fn download_solution() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("temp");
        assert!(!path.exists());

        let core = authenticated_core();
        core.download_solution(83114, &path).unwrap();
        assert!(path.exists());
        todo!("access denied")
    }

    #[test]
    #[ignore]
    fn post_submission() {
        let path = Path::new("tests/data/exercise");
        let core = authenticated_core();
        let _submission = core.post_submission(83114, &path).unwrap();
    }

    #[test]
    #[ignore]
    fn organization_courses() {
        let core = authenticated_core();
        let _courses = core.organization_courses("mooc").unwrap();
    }

    #[test]
    #[ignore]
    fn download_submission() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("temp");
        assert!(!path.exists());

        let core = authenticated_core();
        core.download_submission(7271229, &path).unwrap();
        assert!(path.exists());
    }

    #[test]
    #[ignore]
    fn post_feedback() {
        let core = authenticated_core();
        let feedback = vec![FeedbackAnswer {
            question_id: 389,
            answer: "3".to_string(),
        }];
        core.post_feedback(7271229, feedback).unwrap();
    }

    #[test]
    #[ignore]
    fn post_review() {
        let core = authenticated_core();
        core.post_review(7271229, "review", "points?").unwrap();
        todo!("You are not authorized to access this page.")
    }
}
