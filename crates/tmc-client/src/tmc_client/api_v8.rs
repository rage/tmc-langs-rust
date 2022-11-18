#![allow(dead_code)]

//! Models the API of https://tmc.mooc.fi (https://testmycode.github.io/tmc-server/).

use crate::{request::*, response::*, ClientError, TmcClient};
use http::Method;
use oauth2::TokenResponse;
use reqwest::blocking::{
    multipart::{Form, Part},
    RequestBuilder, Response,
};
use serde::de::DeserializeOwned;
use std::{
    collections::HashMap,
    io::{Read, Write},
    time::SystemTime,
};
use tmc_langs_plugins::Language;
use tmc_langs_util::deserialize;
use url::Url;

pub enum PasteData {
    WithoutMessage,
    WithMessage(String),
}

pub enum ReviewData {
    WithoutMessage,
    WithMessage(String),
}

// joins the URL "tail" with the API url root from the client
fn make_url(client: &TmcClient, tail: impl AsRef<str>) -> Result<Url, ClientError> {
    client
        .0
        .root_url
        .join(tail.as_ref())
        .map_err(|e| ClientError::UrlParse(tail.as_ref().to_string(), e))
}

// encodes a string so it can be used for a URL
fn percent_encode(target: &str) -> String {
    percent_encoding::utf8_percent_encode(target, percent_encoding::NON_ALPHANUMERIC).to_string()
}

// creates a request with the required TMC and authentication headers
fn prepare_tmc_request(client: &TmcClient, method: Method, url: Url) -> RequestBuilder {
    log::info!("{} {}", method, url);
    let req = client.0.client.request(method, url).query(&[
        ("client", &client.0.client_name),
        ("client_version", &client.0.client_version),
    ]);
    if let Some(token) = &client.0.token {
        req.bearer_auth(token.access_token().secret())
    } else {
        req
    }
}

// checks a response for failure
fn assert_success(response: Response, url: &Url) -> Result<Response, ClientError> {
    let status = response.status();
    if status.is_success() {
        Ok(response)
    } else if let Ok(err) = response.json::<ErrorResponse>() {
        // failed and got an error json
        let error = match (err.error, err.errors) {
            (Some(err), Some(errs)) => format!("{}, {}", err, errs.join(",")),
            (Some(err), None) => err,
            (None, Some(errs)) => errs.join(","),
            (None, None) => "".to_string(),
        };
        Err(ClientError::HttpError {
            url: url.clone(),
            status,
            error,
            obsolete_client: err.obsolete_client,
        })
    } else {
        // failed and failed to parse error json, return generic HTTP error
        Err(ClientError::HttpError {
            url: url.clone(),
            status,
            error: status.to_string(),
            obsolete_client: false,
        })
    }
}

fn assert_success_json<T: DeserializeOwned>(res: Response, url: &Url) -> Result<T, ClientError> {
    let res = assert_success(res, url)?
        .bytes()
        .map_err(ClientError::HttpReadResponse)?;
    let json = deserialize::json_from_slice(&res)
        .map_err(|e| ClientError::HttpJsonResponse(url.clone(), e))?;
    Ok(json)
}

/// Converts a list of feedback answers to the format expected by the TMC server.
pub fn prepare_feedback_form(feedback: Vec<FeedbackAnswer>) -> HashMap<String, String> {
    let mut form = HashMap::new();
    for (i, answer) in feedback.into_iter().enumerate() {
        form.insert(
            format!("answers[{}][question_id]", i),
            answer.question_id.to_string(),
        );
        form.insert(format!("answers[{}][answer]", i), answer.answer);
    }
    form
}

/// Fetches data from the URL and writes it into the target.
pub fn download(client: &TmcClient, url: Url, mut target: impl Write) -> Result<(), ClientError> {
    let res = prepare_tmc_request(client, Method::GET, url.clone())
        .send()
        .map_err(|e| ClientError::ConnectionError(Method::GET, url.clone(), e))?;

    let mut res = assert_success(res, &url)?;
    let _bytes = res
        .copy_to(&mut target)
        .map_err(ClientError::HttpWriteResponse)?;
    Ok(())
}

/// Fetches JSON from the given URL and deserializes it into T.
pub fn get_json<T: DeserializeOwned>(
    client: &TmcClient,
    url: Url,
    params: &[(&str, String)],
) -> Result<T, ClientError> {
    let res = prepare_tmc_request(client, Method::GET, url.clone())
        .query(params)
        .send()
        .map_err(|e| ClientError::ConnectionError(Method::GET, url.clone(), e))?;

    let json = assert_success_json(res, &url)?;
    Ok(json)
}

/// Posts the given form data to the given URL and deserializes the response to T.
pub fn post_form<T: DeserializeOwned>(
    client: &TmcClient,
    url: Url,
    form: &HashMap<String, String>,
) -> Result<T, ClientError> {
    let res = prepare_tmc_request(client, Method::POST, url.clone())
        .form(form)
        .send()
        .map_err(|e| ClientError::ConnectionError(Method::GET, url.clone(), e))?;

    let json = assert_success_json(res, &url)?;
    Ok(json)
}

/// get /api/v8/application/{client_name}/credentials
/// Fetches oauth2 credentials info.
pub fn get_credentials(client: &TmcClient, client_name: &str) -> Result<Credentials, ClientError> {
    let url = make_url(
        client,
        format!("/api/v8/application/{}/credentials", client_name),
    )?;
    get_json(client, url, &[])
}

/// get /api/v8/core/submission/{submission_id}
/// Checks the submission processing status from the given URL.
pub fn get_submission(
    client: &TmcClient,
    submission_id: u32,
) -> Result<SubmissionProcessingStatus, ClientError> {
    let url = make_url(client, format!("/api/v8/core/submission/{}", submission_id))?;
    get_json(client, url, &[])
}

pub mod user {
    use super::*;

    /// get /api/v8/users/{user_id}
    /// Returns the user's username, email, and administrator status by user id
    pub fn get(client: &TmcClient, user_id: u32) -> Result<User, ClientError> {
        let url = make_url(client, format!("/api/v8/users/{}", user_id))?;
        get_json(client, url, &[])
    }

    /// get /api/v8/users/current
    /// Returns the current user's username, email, and administrator status
    pub fn get_current(client: &TmcClient) -> Result<User, ClientError> {
        let url = make_url(client, "/api/v8/users/current")?;
        get_json(client, url, &[])
    }

    /// post /api/v8/users/basic_info_by_usernames
    /// Requires admin.
    /// Find all users' basic infos with the posted json array of usernames
    pub fn get_basic_info_by_usernames(
        client: &TmcClient,
        usernames: &[String],
    ) -> Result<Vec<User>, ClientError> {
        let url = make_url(client, "/api/v8/users/basic_info_by_usernames")?;
        let mut username_map = HashMap::new();
        username_map.insert("usernames", usernames);
        let res = prepare_tmc_request(client, Method::POST, url.clone())
            .json(&username_map)
            .send()
            .map_err(|e| ClientError::ConnectionError(Method::POST, url.clone(), e))?;

        let json = assert_success_json(res, &url)?;
        Ok(json)
    }

    /// post /api/v8/users/basic_info_by_emails
    /// Requires admin.
    /// Find all users' basic infos with the posted json array of emails
    pub fn get_basic_info_by_emails(
        client: &TmcClient,
        emails: &[String],
    ) -> Result<Vec<User>, ClientError> {
        let url = make_url(client, "/api/v8/users/basic_info_by_emails")?;
        let mut email_map = HashMap::new();
        email_map.insert("emails", emails);
        let res = prepare_tmc_request(client, Method::POST, url.clone())
            .json(&email_map)
            .send()
            .map_err(|e| ClientError::ConnectionError(Method::POST, url.clone(), e))?;

        let json = assert_success_json(res, &url)?;
        Ok(json)
    }
}

pub mod course {
    use super::*;

    /// get /api/v8/courses/{course_id}
    /// Returns the course's information in a json format. Course is searched by id
    pub fn get_by_id(client: &TmcClient, course_id: u32) -> Result<CourseData, ClientError> {
        let url = make_url(client, format!("/api/v8/courses/{}", course_id))?;
        get_json(client, url, &[])
    }

    /// get /api/v8/org/{organization_slug}/courses/{course_name}
    /// Returns the course's information in a json format. Course is searched by organization slug and course name
    pub fn get(
        client: &TmcClient,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<CourseData, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/org/{}/courses/{}",
                percent_encode(organization_slug),
                percent_encode(course_name)
            ),
        )?;
        get_json(client, url, &[])
    }
}

pub mod point {
    use super::*;

    /// get /api/v8/courses/{course_id}/points
    /// Returns the course's points in a json format. Course is searched by id
    pub fn get_course_points_by_id(
        client: &TmcClient,
        course_id: u32,
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        let url = make_url(client, format!("/api/v8/courses/{}/points", course_id))?;
        get_json(client, url, &[])
    }

    /// get /api/v8/courses/{course_id}/exercises/{exercise_name}/points
    /// Returns all the awarded points of an excercise for all users
    pub fn get_exercise_points_by_id(
        client: &TmcClient,
        course_id: u32,
        exercise_name: &str,
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/courses/{}/exercises/{}/points",
                course_id,
                percent_encode(exercise_name)
            ),
        )?;
        get_json(client, url, &[])
    }

    /// get /api/v8/courses/{course_id}/exercises/{exercise_name}/users/{user_id}/points
    /// Returns all the awarded points of an excercise for the specified user
    pub fn get_exercise_points_for_user_by_id(
        client: &TmcClient,
        course_id: u32,
        exercise_name: &str,
        user_id: u32,
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/courses/{}/exercises/{}/users/{}/points",
                course_id,
                percent_encode(exercise_name),
                user_id
            ),
        )?;
        get_json(client, url, &[])
    }

    /// get /api/v8/courses/{course_id}/exercises/{exercise_name}/users/current/points
    /// Returns all the awarded points of an excercise for current user
    pub fn get_exercise_points_for_current_user_by_id(
        client: &TmcClient,
        course_id: u32,
        exercise_name: &str,
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/courses/{}/exercises/{}/users/current/points",
                course_id,
                percent_encode(exercise_name)
            ),
        )?;
        get_json(client, url, &[])
    }

    /// get /api/v8/courses/{course_id}/users/{user_id}/points
    /// Returns the given user's points from the course in a json format. Course is searched by id
    pub fn get_course_points_for_user_by_id(
        client: &TmcClient,
        course_id: u32,
        user_id: u32,
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        let url = make_url(
            client,
            format!("/api/v8/courses/{}/users/{}/points", course_id, user_id),
        )?;
        get_json(client, url, &[])
    }

    /// get /api/v8/courses/{course_id}/users/current/points
    /// Returns the current user's points from the course in a json format. Course is searched by id
    pub fn get_course_points_for_current_user_by_id(
        client: &TmcClient,
        course_id: u32,
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        let url = make_url(
            client,
            format!("/api/v8/courses/{}/users/current/points", course_id),
        )?;
        get_json(client, url, &[])
    }

    /// get /api/v8/org/{organization_slug}/courses/{course_name}/points
    /// Returns the course's points in a json format. Course is searched by name
    pub fn get_course_points(
        client: &TmcClient,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/org/{}/courses/{}/points",
                percent_encode(organization_slug),
                percent_encode(course_name)
            ),
        )?;
        get_json(client, url, &[])
    }

    /* unimplemented, "This feature is only for MOOC-organization's 2019 programming MOOC"
    /// get /api/v8/org/{organization_slug}/courses/{course_name}/eligible_students
    /// Returns all users from the course who have at least 90% of every part's points and are applying for study right, in a json format. Course is searched by name, only 2019 programming mooc course is valid
    pub fn get_course_eligible_students(
        client: &TmcClient,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<(), ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/org/{}/courses/{}/eligible_students",
                percent_encode(organization_slug),
                percent_encode(course_name)
            ),
        )?;
        todo!()
    }
    */

    /// get /api/v8/org/{organization_slug}/courses/{course_name}/exercises/{exercise_name}/points
    /// Returns all the awarded points of an excercise for all users
    pub fn get_exercise_points(
        client: &TmcClient,
        organization_slug: &str,
        course_name: &str,
        exercise_name: &str,
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/org/{}/courses/{}/exercises/{}/points",
                percent_encode(organization_slug),
                percent_encode(course_name),
                percent_encode(exercise_name)
            ),
        )?;
        get_json(client, url, &[])
    }

    /* does not seem to work
    /// get /api/v8/org/{organization_slug}/courses/{course_name}/exercises/{exercise_name}/users/current/points
    /// Returns all the awarded points of an excercise for current user
    pub fn get_exercise_points_for_current_user(
        client: &TmcClient,
        organization_slug: &str,
        course_name: &str,
        exercise_name: &str,
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/org/{}/courses/{}/exercises/{}/users/current/points",
                percent_encode(organization_slug),
                percent_encode(course_name),
                percent_encode(exercise_name)
            ),
        )?;
        get_json(client, url, &[])
    }
    */

    /* does not seem to work
    /// get /api/v8/org/{organization_slug}/courses/{course_name}/exercises/{exercise_name}/users/{user_id}/points
    /// Returns all the awarded points of an excercise for the specified user
    pub fn get_exercise_points_for_user(
        client: &TmcClient,
        organization_slug: &str,
        course_name: &str,
        exercise_name: &str,
        user_id: u32,
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/org/{}/courses/{}/exercises/{}/users/{}/points",
                percent_encode(organization_slug),
                percent_encode(course_name),
                percent_encode(exercise_name),
                user_id
            ),
        )?;
        get_json(client, url, &[])
    }
    */

    /// get /api/v8/org/{organization_slug}/courses/{course_name}/users/{user_id}/points
    /// Returns the given user's points from the course in a json format. Course is searched by name
    pub fn get_course_points_for_user(
        client: &TmcClient,
        organization_slug: &str,
        course_name: &str,
        user_id: u32,
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/org/{}/courses/{}/users/{}/points",
                percent_encode(organization_slug),
                percent_encode(course_name),
                user_id
            ),
        )?;
        get_json(client, url, &[])
    }

    /// get /api/v8/org/{organization_slug}/courses/{course_name}/users/current/points
    /// Returns the current user's points from the course in a json format. Course is searched by name
    pub fn get_course_points_for_current_user(
        client: &TmcClient,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/org/{}/courses/{}/users/current/points",
                percent_encode(organization_slug),
                percent_encode(course_name),
            ),
        )?;
        get_json(client, url, &[])
    }
}

pub mod submission {
    use super::*;

    /// get /api/v8/courses/{course_id}/submissions
    /// Returns the submissions visible to the user in a json format
    pub fn get_course_submissions_by_id(
        client: &TmcClient,
        course_id: u32,
    ) -> Result<Vec<Submission>, ClientError> {
        let url = make_url(client, format!("/api/v8/courses/{}/submissions", course_id))?;
        get_json(client, url, &[])
    }

    /// get /api/v8/courses/{course_id}/submissions/last_hour
    /// Returns submissions to the course in the latest hour
    pub fn get_course_submissions_for_last_hour(
        client: &TmcClient,
        course_id: u32,
    ) -> Result<Vec<u32>, ClientError> {
        let url = make_url(
            client,
            format!("/api/v8/courses/{}/submissions/last_hour", course_id),
        )?;
        get_json(client, url, &[])
    }

    /// get /api/v8/courses/{course_id}/users/{user_id}/submissions
    /// Returns the submissions visible to the user in a json format
    pub fn get_course_submissions_for_user_by_id(
        client: &TmcClient,
        course_id: u32,
        user_id: u32,
    ) -> Result<Vec<Submission>, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/courses/{}/users/{}/submissions",
                course_id, user_id
            ),
        )?;
        get_json(client, url, &[])
    }

    /// get /api/v8/courses/{course_id}/users/current/submissions
    /// Returns the user's own submissions in a json format
    pub fn get_course_submissions_for_current_user_by_id(
        client: &TmcClient,
        course_id: u32,
    ) -> Result<Vec<Submission>, ClientError> {
        let url = make_url(
            client,
            format!("/api/v8/courses/{}/users/current/submissions", course_id,),
        )?;
        get_json(client, url, &[])
    }

    /// get api/v8/exercises/{exercise_id}/users/{user_id}/submissions
    /// Returns the submissions visible to the user in a json format
    pub fn get_exercise_submissions_for_user(
        client: &TmcClient,
        exercise_id: u32,
        user_id: u32,
    ) -> Result<Vec<Submission>, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/exercises/{}/users/{}/submissions",
                exercise_id, user_id
            ),
        )?;
        get_json(client, url, &[])
    }

    /// get api/v8/exercises/{exercise_id}/users/current/submissions
    /// Returns the current user's submissions for the exercise in a json format. The exercise is searched by id.
    pub fn get_exercise_submissions_for_current_user(
        client: &TmcClient,
        exercise_id: u32,
    ) -> Result<Vec<Submission>, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/exercises/{}/users/current/submissions",
                exercise_id
            ),
        )?;
        get_json(client, url, &[])
    }

    /// get /api/v8/org/{organization_slug}/courses/{course_name}/submissions
    /// Returns the submissions visible to the user in a json format
    pub fn get_course_submissions(
        client: &TmcClient,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<Vec<Submission>, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/org/{}/courses/{}/submissions",
                percent_encode(organization_slug),
                percent_encode(course_name)
            ),
        )?;
        get_json(client, url, &[])
    }

    /// get /api/v8/org/{organization_slug}/courses/{course_name}/users/{user_id}/submissions
    /// Returns the submissions visible to the user in a json format
    pub fn get_course_submissions_for_user(
        client: &TmcClient,
        organization_slug: &str,
        course_name: &str,
        user_id: u32,
    ) -> Result<Vec<Submission>, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/org/{}/courses/{}/users/{}/submissions",
                percent_encode(organization_slug),
                percent_encode(course_name),
                user_id
            ),
        )?;
        get_json(client, url, &[])
    }

    /// get /api/v8/org/{organization_slug}/courses/{course_name}/users/current/submissions
    /// Returns the user's own submissions in a json format
    pub fn get_course_submissions_for_current_user(
        client: &TmcClient,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<Vec<Submission>, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/org/{}/courses/{}/users/current/submissions",
                percent_encode(organization_slug),
                percent_encode(course_name),
            ),
        )?;
        get_json(client, url, &[])
    }
}

pub mod exercise {
    use super::*;

    /// get /api/v8/courses/{course_id}/exercises
    /// Returns all exercises of the course as json. Course is searched by id
    pub fn get_course_exercises_by_id(
        client: &TmcClient,
        course_id: u32,
    ) -> Result<Vec<CourseExercise>, ClientError> {
        let url = make_url(client, format!("/api/v8/courses/{}/exercises", course_id))?;
        get_json(client, url, &[])
    }

    /// get api/v8/exercises/{exercise_id}/users/{user_id}/submissions
    /// Returns the submissions visible to the user in a json format
    pub fn get_exercise_submissions_for_user(
        client: &TmcClient,
        exercise_id: u32,
        user_id: u32,
    ) -> Result<Vec<Submission>, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/exercises/{}/users/{}/submissions",
                exercise_id, user_id
            ),
        )?;
        get_json(client, url, &[])
    }

    /// get api/v8/exercises/{exercise_id}/users/current/submissions
    /// Returns the current user's submissions for the exercise in a json format. The exercise is searched by id.
    pub fn get_exercise_submissions_for_current_user(
        client: &TmcClient,
        exercise_id: u32,
    ) -> Result<Vec<Submission>, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/exercises/{}/users/current/submissions",
                exercise_id
            ),
        )?;
        get_json(client, url, &[])
    }

    /// get /api/v8/org/{organization_slug}/courses/{course_name}/exercises
    /// Returns all exercises of the course as json. Course is searched by name
    pub fn get_course_exercises(
        client: &TmcClient,
        organization_slug: &str,
        course_name: &str,
    ) -> Result<Vec<CourseDataExercise>, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/org/{}/courses/{}/exercises",
                percent_encode(organization_slug),
                percent_encode(course_name)
            ),
        )?;
        get_json(client, url, &[])
    }

    /// get /api/v8/org/{organization_slug}/courses/{course_name}/exercises/{exercise_name}/download
    /// Download the exercise as a zip file
    pub fn download_course_exercise(
        client: &TmcClient,
        organization_slug: &str,
        course_name: &str,
        exercise_name: &str,
        target: impl Write,
    ) -> Result<(), ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/org/{}/courses/{}/exercises/{}/download",
                percent_encode(organization_slug),
                percent_encode(course_name),
                percent_encode(exercise_name)
            ),
        )?;
        download(client, url, target)
    }
}

pub mod organization {
    use super::*;

    /// get /api/v8/org.json
    /// Returns a list of all organizations
    pub fn get_organizations(client: &TmcClient) -> Result<Vec<Organization>, ClientError> {
        let url = make_url(client, "/api/v8/org.json")?;
        get_json(client, url, &[])
    }

    /// get /api/v8/org/{organization_slug}.json
    /// Returns a json representation of the organization
    pub fn get_organization(
        client: &TmcClient,
        organization_slug: &str,
    ) -> Result<Organization, ClientError> {
        let url = make_url(
            client,
            format!("/api/v8/org/{}.json", percent_encode(organization_slug)),
        )?;
        get_json(client, url, &[])
    }
}

pub mod core {
    use super::*;

    /// get /api/v8/core/courses/{course_id}
    /// Returns the course details in a json format. Course is searched by id
    pub fn get_course(client: &TmcClient, course_id: u32) -> Result<CourseDetails, ClientError> {
        let url = make_url(client, format!("/api/v8/core/courses/{}", course_id))?;
        get_json(client, url, &[])
    }

    /// get /api/v8/core/courses/{course_id}/reviews
    /// Returns the course's review information for current user's submissions in a json format. Course is searched by id
    pub fn get_course_reviews(
        client: &TmcClient,
        course_id: u32,
    ) -> Result<Vec<Review>, ClientError> {
        let url = make_url(
            client,
            format!("/api/v8/core/courses/{}/reviews", course_id),
        )?;
        get_json(client, url, &[])
    }

    /// put /api/v8/core/courses/{course_id}/reviews/{review_id}
    /// Update existing review.
    /// Review text can be updated by using `review_body`.
    /// Review can be marked as read or unread by setting `mark_as_read`.
    pub fn update_course_review(
        client: &TmcClient,
        course_id: u32,
        review_id: u32,
        review_body: Option<String>,
        mark_as_read: Option<bool>,
    ) -> Result<(), ClientError> {
        let url = make_url(
            client,
            format!("/api/v8/core/courses/{}/reviews/{}", course_id, review_id),
        )?;

        let mut form = HashMap::new();
        if let Some(review_body) = review_body {
            form.insert("review[review_body]", review_body);
        }
        if let Some(mark_as_read) = mark_as_read {
            if mark_as_read {
                form.insert("mark_as_read", "true".to_string());
            } else {
                form.insert("mark_as_unread", "true".to_string());
            }
        }
        let res = prepare_tmc_request(client, Method::PUT, url.clone())
            .form(&form)
            .send()
            .map_err(|e| ClientError::ConnectionError(Method::PUT, url.clone(), e))?;

        assert_success(res, &url)?;
        Ok(())
    }

    /// post /api/v8/core/courses/{course_id}/unlock
    /// Untested.
    /// Unlocks the courses exercises
    pub fn unlock_course(client: &TmcClient, course_id: u32) -> Result<(), ClientError> {
        let url = make_url(client, format!("/api/v8/core/courses/{}/unlock", course_id))?;
        let res = prepare_tmc_request(client, Method::POST, url.clone())
            .send()
            .map_err(|e| ClientError::ConnectionError(Method::POST, url.clone(), e))?;

        assert_success(res, &url)?;
        Ok(())
    }

    /// get /api/v8/core/exercises/{exercise_id}/download
    /// Download the exercise as a zip file
    pub fn download_exercise(
        client: &TmcClient,
        exercise_id: u32,
        target: impl Write,
    ) -> Result<(), ClientError> {
        let url = make_url(
            client,
            format!("/api/v8/core/exercises/{}/download", exercise_id),
        )?;
        download(client, url, target)
    }

    /// get /api/v8/core/exercises/{exercise_id}
    /// Returns information about exercise and its submissions.
    pub fn get_exercise(
        client: &TmcClient,
        exercise_id: u32,
    ) -> Result<ExerciseDetails, ClientError> {
        let url = make_url(client, format!("/api/v8/core/exercises/{}", exercise_id))?;
        get_json(client, url, &[])
    }

    /// get /api/v8/core/exercises/details
    /// Fetch multiple exercise details as query parameters.
    pub fn get_exercise_details(
        client: &TmcClient,
        exercises: &[u32],
    ) -> Result<Vec<ExercisesDetails>, ClientError> {
        let url = make_url(client, "/api/v8/core/exercises/details")?;
        let exercise_ids = (
            "ids",
            exercises
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(","),
        );

        let res: ExercisesDetailsWrapper = get_json(client, url, &[exercise_ids])?;
        Ok(res.exercises)
    }

    /// get /api/v8/core/exercises/{exercise_id}/solution/download
    /// Download the solution for an exercise as a zip file
    pub fn download_exercise_solution(
        client: &TmcClient,
        exercise_id: u32,
        target: impl Write,
    ) -> Result<(), ClientError> {
        let url = make_url(
            client,
            format!("/api/v8/core/exercises/{}/solution/download", exercise_id),
        )?;
        download(client, url, target)
    }

    /// post /api/v8/core/exercises/{exercise_id}/submissions
    /// Create submission from a zip file
    pub fn submit_exercise(
        client: &TmcClient,
        exercise_id: u32,
        submission_zip: impl Read + Send + Sync + 'static,
        submit_paste: Option<PasteData>,
        submit_for_review: Option<ReviewData>,
        locale: Option<Language>,
    ) -> Result<NewSubmission, ClientError> {
        let url = make_url(
            client,
            format!("/api/v8/core/exercises/{}/submissions", exercise_id),
        )?;

        let mut form = Form::new();
        form = form
            .text(
                "client_time",
                SystemTime::UNIX_EPOCH.elapsed()?.as_secs().to_string(),
            )
            .text(
                "client_nanotime",
                SystemTime::UNIX_EPOCH.elapsed()?.as_nanos().to_string(),
            )
            .part(
                "submission[file]",
                Part::reader(submission_zip).file_name("submission.zip"),
            );

        if let Some(submit_paste) = submit_paste {
            form = form.text("paste", "1");
            if let PasteData::WithMessage(message) = submit_paste {
                form = form.text("message_for_paste", message);
            }
        }
        if let Some(submit_for_review) = submit_for_review {
            form = form.text("request_review", "1");
            if let ReviewData::WithMessage(message) = submit_for_review {
                form = form.text("message_for_reviewer", message);
            }
        }

        if let Some(locale) = locale {
            form = form.text("error_msg_locale", locale.to_639_3());
        }

        let res = prepare_tmc_request(client, Method::POST, url.clone())
            .multipart(form)
            .send()
            .map_err(|e| ClientError::ConnectionError(Method::POST, url.clone(), e))?;

        let json = assert_success_json(res, &url)?;
        Ok(json)
    }

    /// get /api/v8/core/org/{organization_slug}/courses
    /// Returns an array containing each course's collection of links
    pub fn get_organization_courses(
        client: &TmcClient,
        organization_slug: &str,
    ) -> Result<Vec<Course>, ClientError> {
        let url = make_url(
            client,
            format!(
                "/api/v8/core/org/{}/courses",
                percent_encode(organization_slug)
            ),
        )?;
        get_json(client, url, &[])
    }

    /// get /api/v8/core/submissions/{submission_id}/download
    /// Download the submission as a zip file
    pub fn download_submission(
        client: &TmcClient,
        submission_id: u32,
        target: impl Write,
    ) -> Result<(), ClientError> {
        let url = make_url(
            client,
            format!("/api/v8/core/submissions/{}/download", submission_id),
        )?;
        download(client, url, target)
    }

    /// post /api/v8/core/submissions/{submission_id}/feedback
    /// Submits a feedback for submission
    pub fn post_submission_feedback(
        client: &TmcClient,
        submission_id: u32,
        feedback: Vec<FeedbackAnswer>,
    ) -> Result<SubmissionFeedbackResponse, ClientError> {
        let url = make_url(
            client,
            format!("/api/v8/core/submissions/{}/feedback", submission_id),
        )?;

        let form = prepare_feedback_form(feedback);
        let res = prepare_tmc_request(client, Method::POST, url.clone())
            .form(&form)
            .send()
            .map_err(|e| ClientError::ConnectionError(Method::POST, url.clone(), e))?;

        let json = assert_success_json(res, &url)?;
        Ok(json)
    }

    /// post /api/v8/core/submissions/{submission_id}/reviews
    /// Submits a review for the submission
    pub fn post_submission_review(
        client: &TmcClient,
        submission_id: u32,
        review: String,
        // review_points: &[String], doesn't work yet
    ) -> Result<(), ClientError> {
        let url = make_url(
            client,
            format!("/api/v8/core/submissions/{}/reviews", submission_id),
        )?;

        let mut form = HashMap::new();
        form.insert("review[review_body]".to_string(), review);

        /*
        for point in review_points {
            form.insert(format!("review[points][{}]", point), "".to_string());
        }
        */

        let res = prepare_tmc_request(client, Method::POST, url.clone())
            .form(&form)
            .send()
            .map_err(|e| ClientError::ConnectionError(Method::POST, url.clone(), e))?;

        assert_success(res, &url)?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::{super::TmcClient, *};
    use mockito::{Matcher, Mock};
    use std::io::{Cursor, Seek, SeekFrom};

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new().with_level(LevelFilter::Debug).init();
    }

    fn make_client() -> TmcClient {
        TmcClient::new(
            mockito::server_url().parse().unwrap(),
            "client".to_string(),
            "version".to_string(),
        )
    }

    fn client_matcher() -> Matcher {
        Matcher::AllOf(vec![
            Matcher::UrlEncoded("client".to_string(), "client".to_string()),
            Matcher::UrlEncoded("client_version".to_string(), "version".to_string()),
        ])
    }

    fn mock_get(path: &str, body: &str) -> Mock {
        mockito::mock("GET", path)
            .match_query(client_matcher())
            .with_body(body)
            .create()
    }

    #[test]
    fn gets_credentials() {
        init();

        let client = make_client();
        let _m = mock_get(
            "/api/v8/application/client/credentials",
            r#"
        {
            "application_id": "id",
            "secret": "s"
        }
        "#,
        );

        let _res = get_credentials(&client, "client").unwrap();
    }

    #[test]
    fn gets_submission_processing_status() {
        init();

        let client = make_client();
        let _m = mockito::mock("GET", "/api/v8/core/submission/0")
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("client".into(), "client".into()),
                Matcher::UrlEncoded("client_version".into(), "version".into()),
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

        let submission_processing_status = get_submission(&client, 0).unwrap();
        match submission_processing_status {
            SubmissionProcessingStatus::Finished(f) => {
                assert_eq!(f.all_tests_passed, Some(true));
            }
            SubmissionProcessingStatus::Processing(_) => panic!("wrong status"),
        }
    }

    #[test]
    fn user_get() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/users/0",
            r#"
{
    "id": 1,
    "username": "student",
    "email": "student@example.com",
    "administrator": false
}
"#,
        );
        let _user = user::get(client, 0).unwrap();
    }

    #[test]
    fn user_get_current() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/users/current",
            r#"
{
    "id": 1,
    "username": "student",
    "email": "student@example.com",
    "administrator": false
}
"#,
        );

        let _res = user::get_current(client).unwrap();
    }

    #[test]
    fn user_get_basic_info_by_usernames() {
        init();

        let client = &make_client();
        let _m = mockito::mock("POST", "/api/v8/users/basic_info_by_usernames")
            .match_query(client_matcher())
            .match_body(Matcher::JsonString(
                r#"{"usernames": ["username"]}"#.to_string(),
            ))
            .with_body(
                r#"
[
  {
    "id": 1,
    "username": "student",
    "email": "student@example.com",
    "administrator": false
  }
]
"#,
            )
            .create();

        let _res = user::get_basic_info_by_usernames(client, &["username".to_string()]).unwrap();
    }

    #[test]
    fn user_get_basic_info_by_emails() {
        init();

        let client = &make_client();
        let _m = mockito::mock("POST", "/api/v8/users/basic_info_by_emails")
            .match_query(client_matcher())
            .match_body(Matcher::JsonString(r#"{"emails": ["email"]}"#.to_string()))
            .with_body(
                r#"
[
  {
    "id": 1,
    "username": "student",
    "email": "student@example.com",
    "administrator": false
  }
]
"#,
            )
            .create();

        let _res = user::get_basic_info_by_emails(client, &["email".to_string()]).unwrap();
    }

    #[test]
    fn course_get_by_id() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/courses/0",
            r#"
{
    "name": "organizationid-coursename",
    "hide_after": "2016-10-10T13:22:19.554+03:00",
    "hidden": false,
    "cache_version": 1,
    "spreadsheet_key": "string",
    "hidden_if_registered_after": "string",
    "refreshed_at": "2016-10-10T13:22:36.871+03:00",
    "locked_exercise_points_visible": true,
    "description": "",
    "paste_visibility": 0,
    "formal_name": "string",
    "certificate_downloadable": false,
    "certificate_unlock_spec": "string",
    "organization_id": 1,
    "disabled_status": "enabled",
    "title": "testcourse",
    "material_url": "",
    "course_template_id": 1,
    "hide_submission_results": false,
    "external_scoreboard_url": "string",
    "organization_slug": "hy"
}
"#,
        );

        let _res = course::get_by_id(client, 0).unwrap();
    }

    #[test]
    fn course_get() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/org/someorg/courses/somecourse",
            r#"
{
    "name": "organizationid-coursename",
    "hide_after": "2016-10-10T13:22:19.554+03:00",
    "hidden": false,
    "cache_version": 1,
    "spreadsheet_key": "string",
    "hidden_if_registered_after": "string",
    "refreshed_at": "2016-10-10T13:22:36.871+03:00",
    "locked_exercise_points_visible": true,
    "description": "",
    "paste_visibility": 0,
    "formal_name": "string",
    "certificate_downloadable": false,
    "certificate_unlock_spec": "string",
    "organization_id": 1,
    "disabled_status": "enabled",
    "title": "testcourse",
    "material_url": "",
    "course_template_id": 1,
    "hide_submission_results": false,
    "external_scoreboard_url": "string",
    "organization_slug": "hy"
}
"#,
        );

        let _res = course::get(client, "someorg", "somecourse").unwrap();
    }

    #[test]
    fn point_get_course_points_by_id() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/courses/0/points",
            r#"
[
  {
    "awarded_point": {
      "id": 1,
      "course_id": 1,
      "user_id": 1,
      "submission_id": 2,
      "name": "point name",
      "created_at": "2016-10-17T11:10:17.295+03:00"
    },
    "exercise_id": 1
  }
]
"#,
        );

        let _res = point::get_course_points_by_id(client, 0).unwrap();
    }

    #[test]
    fn point_get_exercise_points_by_id() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/courses/0/exercises/someexercise/points",
            r#"
[
  {
    "awarded_point": {
      "id": 1,
      "course_id": 1,
      "user_id": 1,
      "submission_id": 2,
      "name": "point name",
      "created_at": "2016-10-17T11:10:17.295+03:00"
    },
    "exercise_id": 1
  }
]
"#,
        );

        let _res = point::get_exercise_points_by_id(client, 0, "someexercise").unwrap();
    }

    #[test]
    fn point_get_exercise_points_for_user_by_id() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/courses/0/exercises/someexercise/users/1/points",
            r#"
[
  {
    "awarded_point": {
      "id": 1,
      "course_id": 1,
      "user_id": 1,
      "submission_id": 2,
      "name": "point name",
      "created_at": "2016-10-17T11:10:17.295+03:00"
    },
    "exercise_id": 1
  }
]
"#,
        );

        let _res = point::get_exercise_points_for_user_by_id(client, 0, "someexercise", 1).unwrap();
    }

    #[test]
    fn point_get_exercise_points_for_current_user_by_id() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/courses/0/exercises/someexercise/users/current/points",
            r#"
[
  {
    "awarded_point": {
      "id": 1,
      "course_id": 1,
      "user_id": 1,
      "submission_id": 2,
      "name": "point name",
      "created_at": "2016-10-17T11:10:17.295+03:00"
    },
    "exercise_id": 1
  }
]
"#,
        );

        let _res =
            point::get_exercise_points_for_current_user_by_id(client, 0, "someexercise").unwrap();
    }

    #[test]
    fn point_get_course_points_for_user_by_id() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/courses/0/users/1/points",
            r#"
[
  {
    "awarded_point": {
      "id": 1,
      "course_id": 1,
      "user_id": 1,
      "submission_id": 2,
      "name": "point name",
      "created_at": "2016-10-17T11:10:17.295+03:00"
    },
    "exercise_id": 1
  }
]
"#,
        );

        let _res = point::get_course_points_for_user_by_id(client, 0, 1).unwrap();
    }

    #[test]
    fn point_get_course_points_for_current_user_by_id() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/courses/0/users/current/points",
            r#"
[
  {
    "awarded_point": {
      "id": 1,
      "course_id": 1,
      "user_id": 1,
      "submission_id": 2,
      "name": "point name",
      "created_at": "2016-10-17T11:10:17.295+03:00"
    },
    "exercise_id": 1
  }
]
"#,
        );

        let _res = point::get_course_points_for_current_user_by_id(client, 0).unwrap();
    }

    #[test]
    fn point_get_course_points() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/org/someorg/courses/somecourse/points",
            r#"
[
  {
    "awarded_point": {
      "id": 1,
      "course_id": 1,
      "user_id": 1,
      "submission_id": 2,
      "name": "point name",
      "created_at": "2016-10-17T11:10:17.295+03:00"
    },
    "exercise_id": 1
  }
]
"#,
        );

        let _res = point::get_course_points(client, "someorg", "somecourse").unwrap();
    }

    #[test]
    fn point_get_exercise_points() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/org/someorg/courses/somecourse/exercises/someexercise/points",
            r#"
[
  {
    "awarded_point": {
      "id": 1,
      "course_id": 1,
      "user_id": 1,
      "submission_id": 2,
      "name": "point name",
      "created_at": "2016-10-17T11:10:17.295+03:00"
    },
    "exercise_id": 1
  }
]
"#,
        );

        let _res =
            point::get_exercise_points(client, "someorg", "somecourse", "someexercise").unwrap();
    }

    #[test]
    fn point_get_course_points_for_user() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/org/someorg/courses/somecourse/users/0/points",
            r#"
[
  {
    "awarded_point": {
      "id": 1,
      "course_id": 1,
      "user_id": 1,
      "submission_id": 2,
      "name": "point name",
      "created_at": "2016-10-17T11:10:17.295+03:00"
    },
    "exercise_id": 1
  }
]
  "#,
        );

        let _res = point::get_course_points_for_user(client, "someorg", "somecourse", 0).unwrap();
    }

    #[test]
    fn point_get_course_points_for_current_user() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/org/someorg/courses/somecourse/users/current/points",
            r#"
[
  {
    "awarded_point": {
      "id": 1,
      "course_id": 1,
      "user_id": 1,
      "submission_id": 2,
      "name": "point name",
      "created_at": "2016-10-17T11:10:17.295+03:00"
    },
    "exercise_id": 1
  }
]
  "#,
        );

        let _res =
            point::get_course_points_for_current_user(client, "someorg", "somecourse").unwrap();
    }

    #[test]
    fn submission_get_course_submissions_by_id() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/courses/0/submissions",
            r#"
[
    {
      "id": 1,
      "user_id": 1,
      "pretest_error": "Missing test output. Did you terminate your program with an exit() command?",
      "created_at": "2016-10-17T11:10:17.295+03:00",
      "exercise_name": "trivial",
      "course_id": 1,
      "processed": true,
      "all_tests_passed": true,
      "points": "string",
      "processing_tried_at": "2016-10-17T11:10:17.295+03:00",
      "processing_began_at": "2016-10-17T11:10:17.295+03:00",
      "processing_completed_at": "2016-10-17T11:10:17.295+03:00",
      "times_sent_to_sandbox": 1,
      "processing_attempts_started_at": "2016-10-17T11:10:17.295+03:00",
      "params_json": "{\"error_msg_locale\":\"en\"}",
      "requires_review": true,
      "requests_review": true,
      "reviewed": true,
      "message_for_reviewer": "",
      "newer_submission_reviewed": true,
      "review_dismissed": true,
      "paste_available": true,
      "message_for_paste": "",
      "paste_key": "string"
    }
]
"#,
        );

        let _res = submission::get_course_submissions_by_id(client, 0).unwrap();
    }

    #[test]
    fn submission_get_course_submissions_for_last_hour() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/courses/0/submissions/last_hour",
            r#"
[
    1
]
"#,
        );

        let _res = submission::get_course_submissions_for_last_hour(client, 0).unwrap();
    }

    #[test]
    fn submission_get_course_submissions_for_user_by_id() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/courses/0/users/1/submissions")
            .match_query(client_matcher())
            .with_body(
                r#"
[
    {
      "id": 1,
      "user_id": 1,
      "pretest_error": "Missing test output. Did you terminate your program with an exit() command?",
      "created_at": "2016-10-17T11:10:17.295+03:00",
      "exercise_name": "trivial",
      "course_id": 1,
      "processed": true,
      "all_tests_passed": true,
      "points": "string",
      "processing_tried_at": "2016-10-17T11:10:17.295+03:00",
      "processing_began_at": "2016-10-17T11:10:17.295+03:00",
      "processing_completed_at": "2016-10-17T11:10:17.295+03:00",
      "times_sent_to_sandbox": 1,
      "processing_attempts_started_at": "2016-10-17T11:10:17.295+03:00",
      "params_json": "{\"error_msg_locale\":\"en\"}",
      "requires_review": true,
      "requests_review": true,
      "reviewed": true,
      "message_for_reviewer": "",
      "newer_submission_reviewed": true,
      "review_dismissed": true,
      "paste_available": true,
      "message_for_paste": "",
      "paste_key": "string"
    }
]
"#,
            )
            .create();

        let _res = submission::get_course_submissions_for_user_by_id(client, 0, 1).unwrap();
    }

    #[test]
    fn submission_get_course_submissions_for_current_user_by_id() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/courses/0/users/current/submissions",
            r#"
[
    {
      "id": 1,
      "user_id": 1,
      "pretest_error": "Missing test output. Did you terminate your program with an exit() command?",
      "created_at": "2016-10-17T11:10:17.295+03:00",
      "exercise_name": "trivial",
      "course_id": 1,
      "processed": true,
      "all_tests_passed": true,
      "points": "string",
      "processing_tried_at": "2016-10-17T11:10:17.295+03:00",
      "processing_began_at": "2016-10-17T11:10:17.295+03:00",
      "processing_completed_at": "2016-10-17T11:10:17.295+03:00",
      "times_sent_to_sandbox": 1,
      "processing_attempts_started_at": "2016-10-17T11:10:17.295+03:00",
      "params_json": "{\"error_msg_locale\":\"en\"}",
      "requires_review": true,
      "requests_review": true,
      "reviewed": true,
      "message_for_reviewer": "",
      "newer_submission_reviewed": true,
      "review_dismissed": true,
      "paste_available": true,
      "message_for_paste": "",
      "paste_key": "string"
    }
]
"#,
        );

        let _res = submission::get_course_submissions_for_current_user_by_id(client, 0).unwrap();
    }

    #[test]
    fn submission_get_exercise_submissions_for_user() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/org/someorg/courses/somecourse/users/0/submissions",
            r#"
[
    {
      "id": 1,
      "user_id": 1,
      "pretest_error": "Missing test output. Did you terminate your program with an exit() command?",
      "created_at": "2016-10-17T11:10:17.295+03:00",
      "exercise_name": "trivial",
      "course_id": 1,
      "processed": true,
      "all_tests_passed": true,
      "points": "string",
      "processing_tried_at": "2016-10-17T11:10:17.295+03:00",
      "processing_began_at": "2016-10-17T11:10:17.295+03:00",
      "processing_completed_at": "2016-10-17T11:10:17.295+03:00",
      "times_sent_to_sandbox": 1,
      "processing_attempts_started_at": "2016-10-17T11:10:17.295+03:00",
      "params_json": "{\"error_msg_locale\":\"en\"}",
      "requires_review": true,
      "requests_review": true,
      "reviewed": true,
      "message_for_reviewer": "",
      "newer_submission_reviewed": true,
      "review_dismissed": true,
      "paste_available": true,
      "message_for_paste": "",
      "paste_key": "string"
    }
]
"#,
        );

        let _res = submission::get_course_submissions_for_user(client, "someorg", "somecourse", 0)
            .unwrap();
    }

    #[test]
    fn submission_get_exercise_submissions_for_current_user() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/org/someorg/courses/somecourse/users/current/submissions",
            r#"
[
    {
      "id": 1,
      "user_id": 1,
      "pretest_error": "Missing test output. Did you terminate your program with an exit() command?",
      "created_at": "2016-10-17T11:10:17.295+03:00",
      "exercise_name": "trivial",
      "course_id": 1,
      "processed": true,
      "all_tests_passed": true,
      "points": "string",
      "processing_tried_at": "2016-10-17T11:10:17.295+03:00",
      "processing_began_at": "2016-10-17T11:10:17.295+03:00",
      "processing_completed_at": "2016-10-17T11:10:17.295+03:00",
      "times_sent_to_sandbox": 1,
      "processing_attempts_started_at": "2016-10-17T11:10:17.295+03:00",
      "params_json": "{\"error_msg_locale\":\"en\"}",
      "requires_review": true,
      "requests_review": true,
      "reviewed": true,
      "message_for_reviewer": "",
      "newer_submission_reviewed": true,
      "review_dismissed": true,
      "paste_available": true,
      "message_for_paste": "",
      "paste_key": "string"
    }
]
"#,
        );

        let _res =
            submission::get_course_submissions_for_current_user(client, "someorg", "somecourse")
                .unwrap();
    }

    #[test]
    fn submission_get_course_submissions() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/org/someorg/courses/somecourse/submissions",
            r#"
[
    {
      "id": 1,
      "user_id": 1,
      "pretest_error": "Missing test output. Did you terminate your program with an exit() command?",
      "created_at": "2016-10-17T11:10:17.295+03:00",
      "exercise_name": "trivial",
      "course_id": 1,
      "processed": true,
      "all_tests_passed": true,
      "points": "string",
      "processing_tried_at": "2016-10-17T11:10:17.295+03:00",
      "processing_began_at": "2016-10-17T11:10:17.295+03:00",
      "processing_completed_at": "2016-10-17T11:10:17.295+03:00",
      "times_sent_to_sandbox": 1,
      "processing_attempts_started_at": "2016-10-17T11:10:17.295+03:00",
      "params_json": "{\"error_msg_locale\":\"en\"}",
      "requires_review": true,
      "requests_review": true,
      "reviewed": true,
      "message_for_reviewer": "",
      "newer_submission_reviewed": true,
      "review_dismissed": true,
      "paste_available": true,
      "message_for_paste": "",
      "paste_key": "string"
    }
]
"#,
        );

        let _res = submission::get_course_submissions(client, "someorg", "somecourse").unwrap();
    }

    #[test]
    fn submission_get_course_submissions_for_user() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/org/someorg/courses/somecourse/users/0/submissions",
            r#"
[
    {
      "id": 1,
      "user_id": 1,
      "pretest_error": "Missing test output. Did you terminate your program with an exit() command?",
      "created_at": "2016-10-17T11:10:17.295+03:00",
      "exercise_name": "trivial",
      "course_id": 1,
      "processed": true,
      "all_tests_passed": true,
      "points": "string",
      "processing_tried_at": "2016-10-17T11:10:17.295+03:00",
      "processing_began_at": "2016-10-17T11:10:17.295+03:00",
      "processing_completed_at": "2016-10-17T11:10:17.295+03:00",
      "times_sent_to_sandbox": 1,
      "processing_attempts_started_at": "2016-10-17T11:10:17.295+03:00",
      "params_json": "{\"error_msg_locale\":\"en\"}",
      "requires_review": true,
      "requests_review": true,
      "reviewed": true,
      "message_for_reviewer": "",
      "newer_submission_reviewed": true,
      "review_dismissed": true,
      "paste_available": true,
      "message_for_paste": "",
      "paste_key": "string"
    }
]
"#,
        );

        let _res = submission::get_course_submissions_for_user(client, "someorg", "somecourse", 0)
            .unwrap();
    }

    #[test]
    fn submission_get_course_submissions_for_current_user() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/org/someorg/courses/somecourse/users/current/submissions",
            r#"
[
    {
      "id": 1,
      "user_id": 1,
      "pretest_error": "Missing test output. Did you terminate your program with an exit() command?",
      "created_at": "2016-10-17T11:10:17.295+03:00",
      "exercise_name": "trivial",
      "course_id": 1,
      "processed": true,
      "all_tests_passed": true,
      "points": "string",
      "processing_tried_at": "2016-10-17T11:10:17.295+03:00",
      "processing_began_at": "2016-10-17T11:10:17.295+03:00",
      "processing_completed_at": "2016-10-17T11:10:17.295+03:00",
      "times_sent_to_sandbox": 1,
      "processing_attempts_started_at": "2016-10-17T11:10:17.295+03:00",
      "params_json": "{\"error_msg_locale\":\"en\"}",
      "requires_review": true,
      "requests_review": true,
      "reviewed": true,
      "message_for_reviewer": "",
      "newer_submission_reviewed": true,
      "review_dismissed": true,
      "paste_available": true,
      "message_for_paste": "",
      "paste_key": "string"
    }
]
"#,
        );

        let _res =
            submission::get_course_submissions_for_current_user(client, "someorg", "somecourse")
                .unwrap();
    }

    #[test]
    fn exercise_get_course_exercises_by_id() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/courses/0/exercises",
            r#"
[
    {
      "id": 1,
      "name": "Exercise name",
      "publish_time": "2016-10-24T14:06:36.730+03:00",
      "solution_visible_after": "2016-10-24T14:06:36.730+03:00",
      "deadline": "2016-10-24T14:06:36.730+03:00",
      "soft_deadline": "2016-10-24T14:06:36.730+03:00",
      "disabled": false,
      "awarded_points": [],
      "available_points": [
        {
          "id": 1,
          "exercise_id": 1,
          "name": "Point name",
          "requires_review": false
        }
      ],
      "unlocked": false
    }
]
"#,
        );

        let _res = exercise::get_course_exercises_by_id(client, 0).unwrap();
    }

    #[test]
    fn exercise_get_exercise_submissions_for_user() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/exercises/0/users/1/submissions",
            r#"
[
    {
      "id": 1,
      "user_id": 1,
      "pretest_error": "Missing test output. Did you terminate your program with an exit() command?",
      "created_at": "2016-10-17T11:10:17.295+03:00",
      "exercise_name": "trivial",
      "course_id": 1,
      "processed": true,
      "all_tests_passed": true,
      "points": "string",
      "processing_tried_at": "2016-10-17T11:10:17.295+03:00",
      "processing_began_at": "2016-10-17T11:10:17.295+03:00",
      "processing_completed_at": "2016-10-17T11:10:17.295+03:00",
      "times_sent_to_sandbox": 1,
      "processing_attempts_started_at": "2016-10-17T11:10:17.295+03:00",
      "params_json": "{\"error_msg_locale\":\"en\"}",
      "requires_review": true,
      "requests_review": true,
      "reviewed": true,
      "message_for_reviewer": "",
      "newer_submission_reviewed": true,
      "review_dismissed": true,
      "paste_available": true,
      "message_for_paste": "",
      "paste_key": "string"
    }
]
"#,
        );

        let _res = exercise::get_exercise_submissions_for_user(client, 0, 1).unwrap();
    }

    #[test]
    fn exercise_get_exercise_submissions_for_current_user() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/exercises/0/users/current/submissions",
            r#"
[
    {
      "id": 1,
      "user_id": 1,
      "pretest_error": "Missing test output. Did you terminate your program with an exit() command?",
      "created_at": "2016-10-17T11:10:17.295+03:00",
      "exercise_name": "trivial",
      "course_id": 1,
      "processed": true,
      "all_tests_passed": true,
      "points": "string",
      "processing_tried_at": "2016-10-17T11:10:17.295+03:00",
      "processing_began_at": "2016-10-17T11:10:17.295+03:00",
      "processing_completed_at": "2016-10-17T11:10:17.295+03:00",
      "times_sent_to_sandbox": 1,
      "processing_attempts_started_at": "2016-10-17T11:10:17.295+03:00",
      "params_json": "{\"error_msg_locale\":\"en\"}",
      "requires_review": true,
      "requests_review": true,
      "reviewed": true,
      "message_for_reviewer": "",
      "newer_submission_reviewed": true,
      "review_dismissed": true,
      "paste_available": true,
      "message_for_paste": "",
      "paste_key": "string"
    }
]
"#,
        );

        let _res = exercise::get_exercise_submissions_for_current_user(client, 0).unwrap();
    }

    #[test]
    fn exercise_get_course_exercises() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/org/someorg/courses/somecourse/exercises",
            r#"
[
    {
      "id": 1,
      "name": "Exercise name",
      "publish_time": "2016-10-24T14:06:36.730+03:00",
      "solution_visible_after": "2016-10-24T14:06:36.730+03:00",
      "deadline": "2016-10-24T14:06:36.730+03:00",
      "soft_deadline": "2016-10-24T14:06:36.730+03:00",
      "disabled": false,
      "awarded_points": [],
      "available_points": [
        {
          "id": 1,
          "exercise_id": 1,
          "name": "Point name",
          "requires_review": false
        }
      ],
      "unlocked": false
    }
]
"#,
        );

        let _res = exercise::get_course_exercises(client, "someorg", "somecourse").unwrap();
    }

    #[test]
    fn exercise_download_course_exercise() {
        init();

        let client = &make_client();
        let _m = mockito::mock(
            "GET",
            "/api/v8/org/someorg/courses/somecourse/exercises/someexercise/download",
        )
        .match_query(client_matcher())
        .with_body(b"1234")
        .create();

        let mut temp = tempfile::tempfile().unwrap();
        exercise::download_course_exercise(
            client,
            "someorg",
            "somecourse",
            "someexercise",
            &mut temp,
        )
        .unwrap();
        let mut buf = vec![];
        temp.seek(SeekFrom::Start(0)).unwrap();
        temp.read_to_end(&mut buf).unwrap();
        assert!(!buf.is_empty());
    }

    #[test]
    fn organization_get_organizations() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/org.json",
            r#"
        [
  {
    "name": "University of Helsinki",
    "information": "Organization for University of Helsinki",
    "slug": "hy",
    "logo_path": "/logos/hy_logo.png",
    "pinned": false
  }
]
"#,
        );

        let _res = organization::get_organizations(client).unwrap();
    }

    #[test]
    fn organization_get_organization() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/org/someorg.json",
            r#"
        {
    "name": "University of Helsinki",
    "information": "Organization for University of Helsinki",
    "slug": "hy",
    "logo_path": "/logos/hy_logo.png",
    "pinned": false
}
"#,
        );

        let _res = organization::get_organization(client, "someorg").unwrap();
    }

    #[test]
    fn core_get_course() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/core/courses/0",
            r#"
{
  "course": {
      "id": 13,
      "name": "organizationid-coursename",
      "title": "coursetitle",
      "description": "description of the course",
      "details_url": "http://tmc.mooc.fi/api/v8/core/courses/13",
      "unlock_url": "https://tmc.mooc.fi/api/v8/core/courses/13/unlock",
      "reviews_url": "https://tmc.mooc.fi/api/v8/core/courses/13/reviews",
      "comet_url": "https://tmc.mooc.fi:8443/comet",
      "spyware_urls": [
        "http://mooc.spyware.testmycode.net/"
      ],
      "unlockables": [
        ""
      ],
      "exercises": [
        {
          "id": 1,
          "name": "Exercise name",
          "locked": false,
          "deadline_description": "2016-02-29 23:59:00 +0200",
          "deadline": "2016-02-29T23:59:00.000+02:00",
          "checksum": "f25e139769b2688e213938456959eeaf",
          "return_url": "https://tmc.mooc.fi/api/v8/core/exercises/1337/submissions",
          "zip_url": "https://tmc.mooc.fi/api/v8/core/exercises/4272/download",
          "returnable": true,
          "requires_review": false,
          "attempted": false,
          "completed": false,
          "reviewed": false,
          "all_review_points_given": true,
          "memory_limit": 1024,
          "runtime_params": [
            "-Xss64M"
          ],
          "valgrind_strategy": "fail",
          "code_review_requests_enabled": false,
          "run_tests_locally_action_enabled": true,
          "exercise_submissions_url": "https://tmc.mooc.fi/api/v8/core/exercises/1337/solution/download",
          "latest_submission_url": "https://tmc.mooc.fi/api/v8/core/exercises/1337",
          "latest_submission_id": 13337,
          "solution_zip_url": "http://tmc.mooc.fi/api/v8/core/submissions/1337/download"
        }
      ]
    }
}
    "#,
        );

        let _res = core::get_course(client, 0).unwrap();
    }

    #[test]
    fn core_get_course_reviews() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/core/courses/0/reviews",
            r#"
        [
    {
      "submission_id": 1,
      "exercise_name": "trivial",
      "id": 1,
      "marked_as_read": false,
      "reviewer_name": "hn",
      "review_body": "",
      "points": [
        "string"
      ],
      "points_not_awarded": [
        "string"
      ],
      "url": "http://localhost:3000/api/core/v8/submissions/1/reviews",
      "update_url": "http://localhost:3000/api/v8/core/courses/1/reviews/1",
      "created_at": "2016-10-10T13:22:19.554+03:00",
      "updated_at": "2016-10-10T13:22:19.554+03:00"
    }
  ]
  "#,
        );

        let _res = core::get_course_reviews(client, 0).unwrap();
    }

    #[test]
    fn core_update_course_review() {
        init();

        let client = &make_client();
        let _m = mockito::mock("PUT", "/api/v8/core/courses/0/reviews/1")
            .match_query(client_matcher())
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("review[review_body]".to_string(), "body".to_string()),
                Matcher::UrlEncoded("mark_as_read".to_string(), "true".to_string()),
            ]))
            .create();

        core::update_course_review(client, 0, 1, Some("body".to_string()), Some(true)).unwrap();
    }

    #[test]
    fn core_unlock_course() {
        init();

        let client = &make_client();
        let _m = mockito::mock("POST", "/api/v8/core/courses/0/unlock")
            .match_query(client_matcher())
            .create();

        core::unlock_course(client, 0).unwrap();
    }

    #[test]
    fn core_download_exercise() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/core/exercises/0/download")
            .match_query(client_matcher())
            .with_body(b"1234")
            .create();

        let mut temp = tempfile::tempfile().unwrap();
        core::download_exercise(client, 0, &mut temp).unwrap();
        let mut buf = vec![];
        temp.seek(SeekFrom::Start(0)).unwrap();
        temp.read_to_end(&mut buf).unwrap();
        assert!(!buf.is_empty());
    }

    #[test]
    fn core_get_exercise() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/core/exercises/0",
            r#"
        {
      "course_name": "course",
      "course_id": 1,
      "code_review_requests_enabled": true,
      "run_tests_locally_action_enabled": true,
      "exercise_name": "exercise",
      "exercise_id": 1,
      "unlocked_at": "2016-12-05T12:00:00.000+03:00",
      "deadline": "2016-12-24T00:00:00.000+03:00",
      "submissions": [
        {
          "exercise_name": "exercise",
          "id": 1,
          "user_id": 1,
          "course_id": 1,
          "created_at": "2016-12-05T12:00:00.000+03:00",
          "all_tests_passed": true,
          "points": "point1",
          "submitted_zip_url": "http://example.com/api/v8/core/submissions/1/download",
          "paste_url": "http://example.com/paste/qqbKk2Z7INqBH8cmaZ7i_A,",
          "processing_time": 25,
          "reviewed": false,
          "requests_review": false
        }
      ]
    }
    "#,
        );

        let _res = core::get_exercise(client, 0).unwrap();
    }

    #[test]
    fn core_get_exercise_details() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/core/exercises/details")
            .match_query(Matcher::AllOf(vec![
                client_matcher(),
                Matcher::UrlEncoded("ids".to_string(), "0,1".to_string()),
            ]))
            .with_body(
                r#"
{
  "exercises": [
    {
      "id": 1,
      "course_name": "course",
      "exercise_name": "exercise",
      "checksum": "f25e139769b2688e213938456959eeaf",
      "hide_submission_results": false
    }
  ]
}
  "#,
            )
            .create();

        let _res = core::get_exercise_details(client, &[0, 1]).unwrap();
    }

    #[test]
    fn core_download_exercise_solution() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/core/exercises/0/solution/download")
            .match_query(client_matcher())
            .with_body(b"1234")
            .create();

        let mut temp = tempfile::tempfile().unwrap();
        core::download_exercise_solution(client, 0, &mut temp).unwrap();
        let mut buf = vec![];
        temp.seek(SeekFrom::Start(0)).unwrap();
        temp.read_to_end(&mut buf).unwrap();
        assert!(!buf.is_empty());
    }

    #[test]
    fn core_submit_exercise() {
        init();

        let client = &make_client();
        let _m = mockito::mock("POST", "/api/v8/core/exercises/0/submissions")
            .match_query(client_matcher())
            .match_body(Matcher::AllOf(vec![
                Matcher::Regex("client_time".to_string()),
                Matcher::Regex("client_nanotime".to_string()),
                Matcher::Regex("submission\\[file\\]".to_string()),
                Matcher::Regex("paste".to_string()),
                Matcher::Regex("message_for_paste".to_string()),
                Matcher::Regex("request_review".to_string()),
                Matcher::Regex("message_for_reviewer".to_string()),
                Matcher::Regex("error_msg_locale".to_string()),
            ]))
            .with_body(
                r#"
            {
      "show_submission_url": "someurl",
      "paste_url": "anotherurl",
      "submission_url": "third"
    }
    "#,
            )
            .create();

        let _res = core::submit_exercise(
            client,
            0,
            Cursor::new(vec![]),
            Some(PasteData::WithMessage("paste".to_string())),
            Some(ReviewData::WithMessage("message".to_string())),
            Some(Language::from_639_1("fi").unwrap()),
        )
        .unwrap();
    }

    #[test]
    fn core_get_organization_courses() {
        init();

        let client = &make_client();
        let _m = mock_get(
            "/api/v8/core/org/someorg/courses",
            r#"
        [
  {
    "id": 13,
    "name": "organizationid-coursename",
    "title": "coursetitle",
    "description": "description of the course",
    "details_url": "https://tmc.mooc.fi/api/v8/core/courses/13",
    "unlock_url": "https://tmc.mooc.fi/api/v8/core/courses/13/unlock",
    "reviews_url": "https://tmc.mooc.fi/api/v8/core/courses/13/reviews",
    "comet_url": "https://tmc.mooc.fi:8443/comet",
    "spyware_urls": [
      "http://mooc.spyware.testmycode.net/"
    ]
  }
]
"#,
        );

        let _res = core::get_organization_courses(client, "someorg").unwrap();
    }

    #[test]
    fn core_download_submission() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/core/submissions/0/download")
            .match_query(client_matcher())
            .with_body(b"1234")
            .create();

        let mut temp = tempfile::tempfile().unwrap();
        core::download_submission(client, 0, &mut temp).unwrap();
        let mut buf = vec![];
        temp.seek(SeekFrom::Start(0)).unwrap();
        temp.read_to_end(&mut buf).unwrap();
        assert!(!buf.is_empty());
    }

    #[test]
    fn core_post_submission_feedback() {
        init();

        let client = &make_client();
        let _m = mockito::mock("POST", "/api/v8/core/submissions/0/feedback")
            .match_query(client_matcher())
            .match_body(Matcher::AllOf(vec![
                Matcher::UrlEncoded("answers[0][question_id]".to_string(), "0".to_string()),
                Matcher::UrlEncoded("answers[0][answer]".to_string(), "ans".to_string()),
            ]))
            .with_body(
                r#"
            {
                "api_version": 0,
                "status": "processing"
            }"#,
            )
            .create();

        let _res = core::post_submission_feedback(
            client,
            0,
            vec![FeedbackAnswer {
                answer: "ans".to_string(),
                question_id: 0,
            }],
        )
        .unwrap();
    }

    #[test]
    fn core_post_submission_review() {
        init();

        let client = &make_client();
        let _m = mockito::mock("POST", "/api/v8/core/submissions/0/reviews")
            .match_query(client_matcher())
            .match_body(Matcher::UrlEncoded(
                "review[review_body]".to_string(),
                "review".to_string(),
            ))
            .create();

        core::post_submission_review(client, 0, "review".to_string()).unwrap();
    }
}
