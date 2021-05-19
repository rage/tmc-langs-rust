#![allow(dead_code)]

use crate::{request::*, response::*, ClientError, TmcClient};
use http::{Method, StatusCode};
use oauth2::TokenResponse;
use reqwest::blocking::multipart::Form;
use reqwest::blocking::{RequestBuilder, Response};
use serde::de::DeserializeOwned;
use std::io::Write;
use std::{collections::HashMap, time::SystemTime};
use tmc_langs_plugins::Language;
use url::Url;

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

// converts a failed response into a ClientError
// assumes the response status code is a failure
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

fn get_json<T: DeserializeOwned>(
    client: &TmcClient,
    url: Url,
    params: &[(&str, String)],
) -> Result<T, ClientError> {
    let res = prepare_tmc_request(client, Method::GET, url.clone())
        .query(params)
        .send()
        .map_err(|e| ClientError::ConnectionError(Method::GET, url.clone(), e))?;

    let res = assert_success(res, &url)?;
    // expecting successful response
    let json = res
        .json()
        .map_err(|e| ClientError::HttpJsonResponse(url, e))?;
    Ok(json)
}

fn download(client: &TmcClient, url: Url, mut target: impl Write) -> Result<(), ClientError> {
    let mut res = prepare_tmc_request(client, Method::GET, url.clone())
        .send()
        .map_err(|e| ClientError::ConnectionError(Method::GET, url.clone(), e))?;

    let res = assert_success(res, &url)?;
    res.copy_to(&mut target)
        .map_err(ClientError::HttpWriteResponse)?;
    Ok(())
}

pub fn get_credentials(
    client: &mut TmcClient,
    client_name: &str,
) -> Result<Credentials, ClientError> {
    let url = make_url(
        client,
        format!("/api/v8/application/{}/credentials", client_name),
    )?;
    get_json(client, url, &[])
}

pub fn get_submission_processing_status(
    client: &TmcClient,
    url: Url,
) -> Result<SubmissionProcessingStatus, ClientError> {
    get_json(client, url, &[])
}

pub mod user {
    use super::*;

    /// get /api/v8/users/{user_id}
    pub fn get(client: &TmcClient, user_id: u32) -> Result<User, ClientError> {
        let url = make_url(client, format!("/api/v8/users/{}", user_id))?;
        get_json(client, url, &[])
    }

    /// get /api/v8/users/current
    pub fn get_current(client: &TmcClient) -> Result<User, ClientError> {
        let url = make_url(client, "/api/v8/users/current")?;
        get_json(client, url, &[])
    }

    /// post /api/v8/users/basic_info_by_usernames
    /// needs admin
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

        let res = assert_success(res, &url)?;
        let res = res
            .json()
            .map_err(|e| ClientError::HttpJsonResponse(url, e))?;
        Ok(res)
    }

    /// post /api/v8/users/basic_info_by_emails
    /// needs admin
    pub fn post_basic_info_by_emails(
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

        let res = assert_success(res, &url)?;
        let res = res
            .json()
            .map_err(|e| ClientError::HttpJsonResponse(url, e))?;
        Ok(res)
    }
}

pub mod course {
    use super::*;

    /// get /api/v8/courses/{course_id}
    pub fn get_by_id(client: &TmcClient, course_id: u32) -> Result<CourseData, ClientError> {
        let url = make_url(client, format!("/api/v8/courses/{}", course_id))?;
        get_json(client, url, &[])
    }

    /// get /api/v8/org/{organization_slug}/courses/{course_name}
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
    pub fn get_course_points_by_id(
        client: &TmcClient,
        course_id: u32,
    ) -> Result<Vec<CourseDataExercisePoint>, ClientError> {
        let url = make_url(client, format!("/api/v8/courses/{}/points", course_id))?;
        get_json(client, url, &[])
    }

    /// get /api/v8/courses/{course_id}/exercises/{exercise_name}/points
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

    // get /api/v8/courses/{course_id}/exercises/{exercise_name}/users/{user_id}/points
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
    pub fn get_course_submissions_by_id(
        client: &TmcClient,
        course_id: u32,
    ) -> Result<Vec<Submission>, ClientError> {
        let url = make_url(client, format!("/api/v8/courses/{}/submissions", course_id))?;
        get_json(client, url, &[])
    }

    /// get /api/v8/courses/{course_id}/submissions/last_hour
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
    pub fn get_course_exercises_by_id(
        client: &TmcClient,
        course_id: u32,
    ) -> Result<Vec<CourseExercise>, ClientError> {
        let url = make_url(client, format!("/api/v8/courses/{}/exercises", course_id))?;
        get_json(client, url, &[])
    }

    /// get api/v8/exercises/{exercise_id}/users/{user_id}/submissions
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
    pub fn get_organizations(client: &TmcClient) -> Result<Vec<Organization>, ClientError> {
        let url = make_url(client, "/api/v8/org.json")?;
        get_json(client, url, &[])
    }

    /// get /api/v8/org/{organization_slug}.json
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
    use std::io::Read;

    use reqwest::blocking::multipart::Part;

    use super::*;

    /// get /api/v8/core/courses/{course_id}
    pub fn get_course(client: &TmcClient, course_id: u32) -> Result<CourseDetails, ClientError> {
        let url = make_url(client, format!("/api/v8/core/courses/{}", course_id))?;
        get_json(client, url, &[])
    }

    /// get /api/v8/core/courses/{course_id}/reviews
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

    ///* unimplemented, does not appear to function
    /// put /api/v8/core/courses/{course_id}/reviews/{review_id}
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

        let res = assert_success(res, &url)?;
        Ok(())
    }

    /// post /api/v8/core/courses/{course_id}/unlock
    /// untested
    pub fn unlock_course(client: &TmcClient, course_id: u32) -> Result<(), ClientError> {
        let url = make_url(client, format!("/api/v8/core/courses/{}/unlock", course_id))?;
        let res = prepare_tmc_request(client, Method::POST, url.clone())
            .send()
            .map_err(|e| ClientError::ConnectionError(Method::POST, url.clone(), e))?;

        let res = assert_success(res, &url)?;
        Ok(())
    }

    /// get /api/v8/core/exercises/{exercise_id}/download
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
    pub fn get_exercise(
        client: &TmcClient,
        exercise_id: u32,
    ) -> Result<ExerciseDetails, ClientError> {
        let url = make_url(client, format!("/api/v8/core/exercises/{}", exercise_id))?;
        get_json(client, url, &[])
    }

    /// get /api/v8/core/exercises/details
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

        // returns map with result in key "exercises"
        let res: HashMap<String, Vec<ExercisesDetails>> = get_json(client, url, &[exercise_ids])?;
        if let Some((_, val)) = res.into_iter().next() {
            // just return whatever value is found first
            return Ok(val);
        }
        Err(ClientError::MissingDetailsValue)
    }

    /// get /api/v8/core/exercises/{exercise_id}/solution/download
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
    pub fn submit_exercise(
        client: &TmcClient,
        exercise_id: u32,
        submission_zip: impl Read + Send + Sync + 'static,
        paste_message: Option<String>,
        message_for_reviewer: Option<String>,
        locale: Option<Language>,
    ) -> Result<NewSubmission, ClientError> {
        let url = make_url(
            client,
            format!("/api/v8/core/exercises/{}/submissions", exercise_id),
        )?;

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
            .part(
                "submission[file]",
                Part::reader(submission_zip).file_name("submission.zip"),
            );

        if let Some(paste_message) = paste_message {
            form = form
                .text("paste", "1")
                .text("message_for_paste", paste_message);
        }

        if let Some(message_for_reviewer) = message_for_reviewer {
            form = form
                .text("request_review", "1")
                .text("message_for_reviewer", message_for_reviewer);
        }

        if let Some(locale) = locale {
            form = form.text("error_msg_locale", locale.to_string());
        }

        let res = prepare_tmc_request(client, Method::POST, url.clone())
            .multipart(form)
            .send()
            .map_err(|e| ClientError::ConnectionError(Method::POST, url.clone(), e))?;

        let res = assert_success(res, &url)?;
        let res = res
            .json()
            .map_err(|e| ClientError::HttpJsonResponse(url, e))?;
        Ok(res)
    }

    /// get /api/v8/core/org/{organization_slug}/courses
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
    pub fn post_submission_feedback(
        client: &TmcClient,
        submission_id: u32,
        feedback: Vec<FeedbackAnswer>,
    ) -> Result<SubmissionFeedbackResponse, ClientError> {
        let url = make_url(
            client,
            format!("/api/v8/core/submissions/{}/feedback", submission_id),
        )?;

        let mut form = HashMap::new();
        for (i, answer) in feedback.into_iter().enumerate() {
            form.insert(
                format!("answers[{}][question_id]", i),
                answer.question_id.to_string(),
            );
            form.insert(format!("answers[{}][answer]", i), answer.answer);
        }

        let res = prepare_tmc_request(client, Method::POST, url.clone())
            .form(&form)
            .send()
            .map_err(|e| ClientError::ConnectionError(Method::POST, url.clone(), e))?;

        let res = assert_success(res, &url)?;
        let res = res
            .json()
            .map_err(|e| ClientError::HttpJsonResponse(url, e))?;
        Ok(res)
    }

    /// post /api/v8/core/submissions/{submission_id}/reviews
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

        let res = assert_success(res, &url)?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::super::TmcClient;
    use super::*;
    use crate::Token;
    use mockito::Matcher;
    use oauth2::{basic::BasicTokenType, AccessToken, EmptyExtraTokenFields};

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new().with_level(LevelFilter::Debug).init();
    }

    fn make_client() -> TmcClient {
        let mut client = TmcClient::new(
            mockito::server_url().parse().unwrap(),
            "client".to_string(),
            "version".to_string(),
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
    fn gets_user() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/users/0")
            .match_query(Matcher::Any)
            .with_body(
                r#"
{
    "id": 1,
    "username": "student",
    "email": "student@example.com",
    "administrator": false
}
"#,
            )
            .create();

        let _user = user::get(client, 0).unwrap();
    }

    #[test]
    fn gets_current_user() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/users/current")
            .match_query(Matcher::Any)
            .with_body(
                r#"
{
    "id": 1,
    "username": "student",
    "email": "student@example.com",
    "administrator": false
}
"#,
            )
            .create();

        let _res = user::get_current(client).unwrap();
    }

    #[test]
    fn gets_course_by_id() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/courses/0")
            .match_query(Matcher::Any)
            .with_body(
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
            )
            .create();

        let _res = course::get_by_id(client, 0).unwrap();
    }

    #[test]
    fn gets_course() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/org/someorg/courses/somecourse")
            .match_query(Matcher::Any)
            .with_body(
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
            )
            .create();

        let _res = course::get(client, "someorg", "somecourse").unwrap();
    }

    #[test]
    fn gets_course_points_by_id() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/courses/0/points")
            .match_query(Matcher::Any)
            .with_body(
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
            )
            .create();

        let _res = point::get_course_points_by_id(client, 0).unwrap();
    }

    #[test]
    fn gets_course_points_for_user_by_id() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/courses/0/users/1/points")
            .match_query(Matcher::Any)
            .with_body(
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
            )
            .create();

        let _res = point::get_course_points_for_user_by_id(client, 0, 1).unwrap();
    }

    #[test]
    fn gets_course_points_for_current_user_by_id() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/courses/0/users/current/points")
            .match_query(Matcher::Any)
            .with_body(
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
            )
            .create();

        let _res = point::get_course_points_for_current_user_by_id(client, 0).unwrap();
    }

    #[test]
    fn gets_course_points() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/org/someorg/courses/somecourse/points")
            .match_query(Matcher::Any)
            .with_body(
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
            )
            .create();

        let _res = point::get_course_points(client, "someorg", "somecourse").unwrap();
    }

    #[test]
    fn gets_exercise_points() {
        init();

        let client = &make_client();
        let _m = mockito::mock(
            "GET",
            "/api/v8/org/someorg/courses/somecourse/exercises/someexercise/points",
        )
        .match_query(Matcher::Any)
        .with_body(
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
        )
        .create();

        let _res =
            point::get_exercise_points(client, "someorg", "somecourse", "someexercise").unwrap();
    }

    #[test]
    fn gets_course_submissions_by_id() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/courses/0/submissions")
            .match_query(Matcher::Any)
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

        let _res = submission::get_course_submissions_by_id(client, 0).unwrap();
    }

    #[test]
    fn gets_course_submissions_for_last_hour() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/courses/0/submissions/last_hour")
            .match_query(Matcher::Any)
            .with_body(
                r#"
[
    1
]
"#,
            )
            .create();

        let _res = submission::get_course_submissions_for_last_hour(client, 0).unwrap();
    }

    #[test]
    fn gets_course_submissions_for_user_by_id() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/courses/0/users/1/submissions")
            .match_query(Matcher::Any)
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
    fn gets_course_submissions_for_current_user_by_id() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/courses/0/users/current/submissions")
            .match_query(Matcher::Any)
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

        let _res = submission::get_course_submissions_for_current_user_by_id(client, 0).unwrap();
    }

    #[test]
    fn gets_exercise_submissions_for_current_user() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/exercises/0/users/current/submissions")
            .match_query(Matcher::Any)
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

        let _res = submission::get_course_submissions_for_current_user_by_id(client, 0).unwrap();
    }

    #[test]
    fn gets_course_submissions() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/org/someorg/courses/somecourse/submissions")
            .match_query(Matcher::Any)
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

        let _res = submission::get_course_submissions(client, "someorg", "somecourse").unwrap();
    }

    #[test]
    fn gets_course_submissions_for_user() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/org/someorg/courses/somecourse/users/0/submissions")
            .match_query(Matcher::Any)
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

        let _res = submission::get_course_submissions_for_user(client, "someorg", "somecourse", 0)
            .unwrap();
    }

    #[test]
    fn gets_course_submissions_for_current_user() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/org/someorg/courses/somecourse/users/current/submissions")
            .match_query(Matcher::Any)
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

        let _res =
            submission::get_course_submissions_for_current_user(client, "someorg", "somecourse")
                .unwrap();
    }

    #[test]
    fn gets_course_exercises_by_id() {
        init();

        let client = &make_client();
        let _m = mockito::mock("GET", "/api/v8/courses/0/exercises")
            .match_query(Matcher::Any)
            .with_body(
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
            )
            .create();

        let _res = exercise::get_course_exercises_by_id(client, 0).unwrap();
    }
}
