//! Node bindings to tmc-langs-rust.

mod de;
mod error;
mod helpers;
mod ser;

use crate::helpers::{convert, convert_err, convert_res};
use base64::Engine;
use neon::prelude::*;
use std::{
    env,
    error::Error,
    io::{Cursor, Read},
    path::PathBuf,
};
use thiserror::Error;
use tmc_langs::{
    Compression, Credentials, DownloadOrUpdateCourseExercisesResult, LangsError, Language,
    PrepareSubmission, TmcConfig, file_util,
    tmc::{
        TestMyCodeClient, TestMyCodeClientError,
        request::FeedbackAnswer,
        response::{NewSubmission, SubmissionFinished},
    },
};

#[derive(Debug, Error)]
enum NodeError {
    #[error(transparent)]
    Langs(#[from] LangsError),
    #[error("Invalid token")]
    InvalidTokenError(#[source] LangsError),
}

fn make_testmycode_client(
    client_name: impl AsRef<str>,
    client_version: impl AsRef<str>,
) -> Result<(TestMyCodeClient, Option<Credentials>), LangsError> {
    let root_url = env::var("TMC_LANGS_TMC_ROOT_URL")
        .unwrap_or_else(|_| "https://tmc.mooc.fi/".to_string())
        .parse()
        .expect("Invalid TMC root url");
    tmc_langs::init_testmycode_client_with_credentials(
        root_url,
        client_name.as_ref(),
        client_version.as_ref(),
    )
}

fn with_client<T, F: FnOnce(&mut TestMyCodeClient) -> Result<T, LangsError>>(
    client_name: impl AsRef<str>,
    client_version: impl AsRef<str>,
    f: F,
) -> Result<T, NodeError> {
    let (mut client, credentials) = make_testmycode_client(client_name, client_version)?;
    match f(&mut client) {
        Ok(res) => Ok(res),
        Err(err) => {
            let mut source = Some(&err as &dyn Error);
            while let Some(s) = source {
                if let Some(TestMyCodeClientError::HttpError { status, .. }) =
                    s.downcast_ref::<TestMyCodeClientError>()
                {
                    if status.as_u16() == 401 {
                        if let Some(credentials) = credentials {
                            credentials.remove()?;
                        }
                        return Err(NodeError::InvalidTokenError(err));
                    }
                }
                source = s.source();
            }
            Err(NodeError::Langs(err))
        }
    }
}

pub fn init_logging(mut cx: FunctionContext) -> JsResult<JsNull> {
    env_logger::init();
    Ok(cx.null())
}

fn checkstyle(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, exercise_path: PathBuf, locale: String);
    lock!(cx, exercise_path);

    let locale = Language::from_639_3(&locale).expect("invalid locale");
    let res = tmc_langs::checkstyle(&exercise_path, locale);
    convert_res(&mut cx, res)
}

fn clean(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, exercise_path: PathBuf);
    lock!(cx, exercise_path);

    let res = tmc_langs::clean(&exercise_path);
    convert_res(&mut cx, res)
}

fn compress_project(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        exercise_path: PathBuf,
        output_path: PathBuf,
        compression: Compression,
        deterministic: bool,
        naive: bool
    );
    lock!(cx, exercise_path);

    let res = tmc_langs::compress_project_to(
        &exercise_path,
        &output_path,
        compression,
        deterministic,
        naive,
    );
    convert_res(&mut cx, res)
}

fn extract_project(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        archive_path: PathBuf,
        output_path: PathBuf,
        compression: Compression
    );

    let mut archive_lock = file_util::Lock::file(archive_path, file_util::LockOptions::Read)
        .map_err(|e| convert_err(&mut cx, e))?;
    let mut archive_guard = archive_lock.lock().map_err(|e| convert_err(&mut cx, e))?;
    let mut data = vec![];
    archive_guard
        .get_file_mut()
        .read_to_end(&mut data)
        .expect("failed to read data");

    let res =
        tmc_langs::extract_project(Cursor::new(data), &output_path, compression, false, false);
    convert_res(&mut cx, res)
}

fn fast_available_points(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, exercise_path: PathBuf);
    lock!(cx, exercise_path);

    let res = tmc_langs::get_available_points(&exercise_path);
    convert_res(&mut cx, res)
}

fn find_exercises(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, exercise_path: PathBuf);
    lock!(cx, exercise_path);

    let res = tmc_langs::find_exercise_directories(&exercise_path);
    convert_res(&mut cx, res)
}

fn get_exercise_packaging_configuration(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, exercise_path: PathBuf);
    lock!(cx, exercise_path);

    let res = tmc_langs::get_exercise_packaging_configuration(&exercise_path);
    convert_res(&mut cx, res)
}

fn list_local_course_exercises(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, client_name: String, course_slug: String);

    let res = tmc_langs::list_local_course_exercises(&client_name, &course_slug);
    convert_res(&mut cx, res)
}

fn prepare_solution(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, exercise_path: PathBuf, output_path: PathBuf);
    lock!(cx, exercise_path);

    let res = tmc_langs::prepare_solution(&exercise_path, &output_path);
    convert_res(&mut cx, res)
}

fn prepare_stub(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, exercise_path: PathBuf, output_path: PathBuf);
    lock!(cx, exercise_path);

    let res = tmc_langs::prepare_stub(&exercise_path, &output_path);
    convert_res(&mut cx, res)
}

fn prepare_submission(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        output_format: String,
        clone_path: PathBuf,
        output_path: PathBuf,
        stub_archive_path: Option<PathBuf>,
        stub_compression: Compression,
        submission_path: PathBuf,
        submission_compression: Compression,
        extract_submission_naively: bool,
        tmc_param: Vec<(String, Vec<String>)>,
        no_archive_prefix: bool
    );

    let mut tmc_params = tmc_langs::TmcParams::new();
    for (key, mut values) in tmc_param {
        if values.len() == 1 {
            let value = values.pop().unwrap();
            tmc_params
                .insert_string(key, value)
                .expect("invalid key-value pair");
        } else {
            tmc_params
                .insert_array(key, values)
                .expect("invalid key-value pair");
        }
    }
    let compression = match output_format.as_str() {
        "Tar" => Compression::Tar,
        "Zip" => Compression::Zip,
        "TarZstd" => Compression::TarZstd,
        _ => panic!("unrecognized output format"),
    };

    let res = tmc_langs::prepare_submission(
        PrepareSubmission {
            archive: &submission_path,
            compression: submission_compression,
            extract_naively: extract_submission_naively,
        },
        &output_path,
        no_archive_prefix,
        tmc_params,
        &clone_path,
        stub_archive_path.as_deref().map(|p| (p, stub_compression)),
        compression,
    );
    convert_res(&mut cx, res)
}

fn refresh_course(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        cache_path: PathBuf,
        cache_root: PathBuf,
        course_name: String,
        git_branch: String,
        source_url: String
    );

    let res =
        tmc_langs::refresh_course(course_name, cache_path, source_url, git_branch, cache_root);
    convert_res(&mut cx, res)
}

fn run_tests(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, exercise_path: PathBuf);
    lock!(cx, exercise_path);

    let res = tmc_langs::run_tests(&exercise_path);
    convert_res(&mut cx, res)
}

fn scan_exercise(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, exercise_path: PathBuf);
    lock!(cx, exercise_path);

    let exercise_name = exercise_path
        .file_name()
        .expect("no file name in path")
        .to_str()
        .expect("file name wasn't valid UTF-8");

    let res = tmc_langs::scan_exercise(&exercise_path, exercise_name.to_string());
    convert_res(&mut cx, res)
}

fn check_exercise_updates(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, client_name: String, client_version: String);

    let projects_dir =
        tmc_langs::get_projects_dir(&client_name).map_err(|e| convert_err(&mut cx, e))?;
    let res = with_client(client_name, client_version, |client| {
        tmc_langs::check_exercise_updates(client, &projects_dir)
    });
    convert_res(&mut cx, res)
}

fn download_model_solution(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        exercise_id: u32,
        target: PathBuf
    );

    let res = with_client(client_name, client_version, |client| {
        Ok(client
            .download_model_solution(exercise_id, &target)
            .map_err(Box::new)?)
    });
    convert_res(&mut cx, res)
}

fn download_old_submission(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        submission_id: u32,
        save_old_state: bool,
        exercise_id: u32,
        output_path: PathBuf
    );

    let res = with_client(client_name, client_version, |client| {
        tmc_langs::download_old_submission(
            client,
            exercise_id,
            &output_path,
            submission_id,
            save_old_state,
        )
    });
    convert_res(&mut cx, res)
}

fn download_or_update_course_exercises(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        download_template: bool,
        exercise_id: Vec<u32>
    );

    let projects_dir =
        tmc_langs::get_projects_dir(&client_name).map_err(|e| convert_err(&mut cx, e))?;
    let res = with_client(client_name, client_version, |client| {
        tmc_langs::download_or_update_course_exercises(
            client,
            &projects_dir,
            &exercise_id,
            download_template,
        )
    })
    .map_err(|e| convert_err(&mut cx, e))?;

    let res = match res {
        tmc_langs::DownloadResult::Success {
            downloaded,
            skipped,
        } => DownloadOrUpdateCourseExercisesResult {
            downloaded,
            skipped,
            failed: None,
        },
        tmc_langs::DownloadResult::Failure {
            downloaded,
            skipped,
            failed,
        } => DownloadOrUpdateCourseExercisesResult {
            downloaded,
            skipped,
            failed: Some(failed),
        },
    };
    convert(&mut cx, &res)
}

fn get_course_data(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        course_id: u32
    );

    let res = with_client(client_name, client_version, |client| {
        tmc_langs::get_course_data(client, course_id)
    });
    convert_res(&mut cx, res)
}

fn get_course_details(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        course_id: u32
    );

    let res = with_client(client_name, client_version, |client| {
        Ok(client.get_course_details(course_id).map_err(Box::new)?)
    });
    convert_res(&mut cx, res)
}

fn get_course_exercises(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        course_id: u32
    );

    let res = with_client(client_name, client_version, |client| {
        Ok(client.get_course_exercises(course_id).map_err(Box::new)?)
    });
    convert_res(&mut cx, res)
}

fn get_course_settings(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        course_id: u32
    );

    let res = with_client(client_name, client_version, |client| {
        Ok(client.get_course(course_id).map_err(Box::new)?)
    });
    convert_res(&mut cx, res)
}

fn get_courses(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        organization: String
    );

    let res = with_client(client_name, client_version, |client| {
        Ok(client.list_courses(&organization).map_err(Box::new)?)
    });
    convert_res(&mut cx, res)
}

fn get_exercise_details(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        exercise_id: u32
    );

    let res = with_client(client_name, client_version, |client| {
        Ok(client.get_exercise_details(exercise_id).map_err(Box::new)?)
    });
    convert_res(&mut cx, res)
}

fn get_exercise_submissions(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        exercise_id: u32
    );

    let res = with_client(client_name, client_version, |client| {
        Ok(client
            .get_exercise_submissions_for_current_user(exercise_id)
            .map_err(Box::new)?)
    });
    convert_res(&mut cx, res)
}

fn get_exercise_updates(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        course_id: u32,
        exercise: Vec<(u32, String)>
    );

    let map = exercise.into_iter().collect();
    let res = with_client(client_name, client_version, |client| {
        Ok(client
            .get_exercise_updates(course_id, map)
            .map_err(Box::new)?)
    });
    convert_res(&mut cx, res)
}

fn get_organization(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        organization: String
    );

    let res = with_client(client_name, client_version, |client| {
        Ok(client.get_organization(&organization).map_err(Box::new)?)
    });
    convert_res(&mut cx, res)
}

fn get_organizations(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, client_name: String, client_version: String);

    let res = with_client(client_name, client_version, |client| {
        Ok(client.get_organizations().map_err(Box::new)?)
    });
    convert_res(&mut cx, res)
}

fn get_unread_reviews(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        course_id: u32
    );

    let res = with_client(client_name, client_version, |client| {
        Ok(client.get_unread_reviews(course_id).map_err(Box::new)?)
    });
    convert_res(&mut cx, res)
}

fn logged_in(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, client_name: String, client_version: String);

    let (_, credentials) =
        make_testmycode_client(client_name, client_version).map_err(|e| convert_err(&mut cx, e))?;
    let res = credentials.is_some();
    convert(&mut cx, &res)
}

fn login(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        base64: bool,
        email: String,
        password: String
    );

    let decoded = if base64 {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(password)
            .expect("Failed to decore password with base64");
        String::from_utf8(bytes).expect("Failed to decode password with base64")
    } else {
        password
    };
    let token = with_client(&client_name, client_version, |client| {
        tmc_langs::login_with_password(client, &client_name, email, decoded)
    })
    .map_err(|e| convert_err(&mut cx, e))?;

    Credentials::save(&client_name, token).expect("Failed to save credentials");
    Ok(cx.null().upcast())
}

fn login_with_token(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        access_token: String
    );

    let token = with_client(&client_name, client_version, |_| {
        Ok(tmc_langs::login_with_token(access_token))
    })
    .map_err(|e| convert_err(&mut cx, e))?;

    Credentials::save(&client_name, token).expect("Failed to save credentials");
    Ok(cx.null().upcast())
}

fn logout(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, client_name: String, client_version: String);

    let (_, credentials) =
        make_testmycode_client(client_name, client_version).map_err(|e| convert_err(&mut cx, e))?;
    if let Some(credentials) = credentials {
        credentials.remove().expect("Failed to remove credentials");
    }
    Ok(cx.null().upcast())
}

fn mark_review_as_read(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        course_id: u32,
        review_id: u32
    );

    let res = with_client(client_name, client_version, |client| {
        Ok(client
            .mark_review_as_read(course_id, review_id)
            .map_err(Box::new)?)
    });
    convert_res(&mut cx, res)
}

fn paste(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        exercise_id: u32,
        locale: Option<String>,
        paste_message: Option<String>,
        submission_path: PathBuf
    );
    lock!(cx, submission_path);

    let locale = locale.map(|l| Language::from_639_3(&l).expect("Invalid locale"));
    let res = with_client(client_name, client_version, |client| {
        Ok(client
            .paste(exercise_id, &submission_path, paste_message, locale)
            .map_err(Box::new)?)
    });
    convert_res(&mut cx, res)
}

fn request_code_review(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        exercise_id: u32,
        locale: String,
        message_for_reviewer: Option<String>,
        submission_path: PathBuf
    );
    lock!(cx, submission_path);

    let locale = Language::from_639_3(&locale).expect("Invalid locale");
    let res = with_client(client_name, client_version, |client| {
        Ok(client
            .request_code_review(
                exercise_id,
                &submission_path,
                message_for_reviewer,
                Some(locale),
            )
            .map_err(Box::new)?)
    });
    convert_res(&mut cx, res)
}

fn reset_exercise(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        save_old_state: bool,
        exercise_id: u32,
        exercise_path: PathBuf
    );
    lock!(cx, exercise_path);

    let res = with_client(client_name, client_version, |client| {
        if save_old_state {
            client
                .submit(exercise_id, &exercise_path, None)
                .map_err(Box::new)?;
        }
        tmc_langs::reset(client, exercise_id, &exercise_path)
    });
    convert_res(&mut cx, res)
}

fn send_feedback(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        submission_id: u32,
        feedback: Vec<(u32, String)>
    );

    let feedback = feedback
        .into_iter()
        .map(|(question_id, answer)| FeedbackAnswer {
            question_id,
            answer,
        })
        .collect();
    let res = with_client(client_name, client_version, |client| {
        Ok(client
            .send_feedback(submission_id, feedback)
            .map_err(Box::new)?)
    });
    convert_res(&mut cx, res)
}

fn submit(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        dont_block: bool,
        locale: Option<String>,
        submission_path: PathBuf,
        exercise_id: u32
    );

    enum Temp {
        NewSubmission(NewSubmission),
        Finished(Box<SubmissionFinished>),
    }

    let locale = locale.map(|l| Language::from_639_3(&l).expect("Invalid locale"));
    let temp = with_client(client_name, client_version, |client| {
        let new_submission = client
            .submit(exercise_id, &submission_path, locale)
            .map_err(Box::new)?;
        if dont_block {
            Ok(Temp::NewSubmission(new_submission))
        } else {
            let submission_url = new_submission
                .submission_url
                .parse()
                .expect("Failed to parse submission URL");
            let finished = client
                .wait_for_submission_at(submission_url)
                .map_err(Box::new)?;
            Ok(Temp::Finished(Box::new(finished)))
        }
    })
    .map_err(|e| convert_err(&mut cx, e))?;
    match temp {
        Temp::NewSubmission(new) => convert(&mut cx, &new),
        Temp::Finished(finished) => convert(&mut cx, &finished),
    }
}

fn update_exercises(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, client_name: String, client_version: String);

    let projects_dir =
        tmc_langs::get_projects_dir(&client_name).map_err(|e| convert_err(&mut cx, e))?;
    let res = with_client(client_name, client_version, |client| {
        tmc_langs::update_exercises(client, &projects_dir)
    });
    convert_res(&mut cx, res)
}

fn wait_for_submission(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        client_version: String,
        submission_id: u32
    );

    let res = with_client(client_name, client_version, |client| {
        Ok(client
            .wait_for_submission(submission_id)
            .map_err(Box::new)?)
    });
    convert_res(&mut cx, res)
}

fn get_setting(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, client_name: String, setting: String);

    let res = tmc_langs::get_setting(&client_name, &setting);
    convert_res(&mut cx, res)
}

fn list_settings(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, client_name: String);

    let res = tmc_langs::get_settings(&client_name);
    convert_res(&mut cx, res)
}

fn migrate_exercise(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        exercise_path: PathBuf,
        course_slug: String,
        exercise_id: u32,
        exercise_slug: String,
        exercise_checksum: String
    );

    let tmc_config = TmcConfig::load(&client_name).map_err(|e| convert_err(&mut cx, e))?;
    let res = tmc_langs::migrate_exercise(
        tmc_config,
        &course_slug,
        &exercise_slug,
        exercise_id,
        &exercise_checksum,
        &exercise_path,
    );
    convert_res(&mut cx, res)
}

fn move_projects_dir(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, client_name: String, dir: PathBuf);

    let tmc_config = TmcConfig::load(&client_name).map_err(|e| convert_err(&mut cx, e))?;
    let res = tmc_langs::move_projects_dir(tmc_config, dir);
    convert_res(&mut cx, res)
}

fn reset_settings(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, client_name: String);

    let res = tmc_langs::reset_settings(&client_name);
    convert_res(&mut cx, res)
}

fn set_setting(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(
        cx,
        client_name: String,
        key: String,
        json: serde_json::Value
    );

    let res = tmc_langs::set_setting(&client_name, &key, json);
    convert_res(&mut cx, res)
}

fn unset_setting(mut cx: FunctionContext) -> JsResult<JsValue> {
    parse_args!(cx, client_name: String, setting: String);

    let res = tmc_langs::unset_setting(&client_name, &setting);
    convert_res(&mut cx, res)
}

#[neon::main]
fn main(mut cx: ModuleContext) -> NeonResult<()> {
    cx.export_function("initLogging", init_logging)?;

    cx.export_function("checkstyle", checkstyle)?;
    cx.export_function("clean", clean)?;
    cx.export_function("compressProject", compress_project)?;
    cx.export_function("extractProject", extract_project)?;
    cx.export_function("fastAvailablePoints", fast_available_points)?;
    cx.export_function("findExercises", find_exercises)?;
    cx.export_function(
        "getExercisePackagingConfiguration",
        get_exercise_packaging_configuration,
    )?;
    cx.export_function("listLocalCourseExercises", list_local_course_exercises)?;
    cx.export_function("prepareSolution", prepare_solution)?;
    cx.export_function("prepareStub", prepare_stub)?;
    cx.export_function("prepareSubmission", prepare_submission)?;
    cx.export_function("refreshCourse", refresh_course)?;
    cx.export_function("runTests", run_tests)?;
    cx.export_function("scanExercise", scan_exercise)?;
    cx.export_function("checkExerciseUpdates", check_exercise_updates)?;
    cx.export_function("downloadModelSolution", download_model_solution)?;
    cx.export_function("downloadOldSubmission", download_old_submission)?;
    cx.export_function(
        "downloadOrUpdateCourseExercises",
        download_or_update_course_exercises,
    )?;
    cx.export_function("getCourseData", get_course_data)?;
    cx.export_function("getCourseDetails", get_course_details)?;
    cx.export_function("getCourseExercises", get_course_exercises)?;
    cx.export_function("getCourseSettings", get_course_settings)?;
    cx.export_function("getCourses", get_courses)?;
    cx.export_function("getExerciseDetails", get_exercise_details)?;
    cx.export_function("getExerciseSubmissions", get_exercise_submissions)?;
    cx.export_function("getExerciseUpdates", get_exercise_updates)?;
    cx.export_function("getOrganization", get_organization)?;
    cx.export_function("getOrganizations", get_organizations)?;
    cx.export_function("getUnreadReviews", get_unread_reviews)?;
    cx.export_function("loggedIn", logged_in)?;
    cx.export_function("login", login)?;
    cx.export_function("loginWithToken", login_with_token)?;
    cx.export_function("logout", logout)?;
    cx.export_function("markReviewAsRead", mark_review_as_read)?;
    cx.export_function("paste", paste)?;
    cx.export_function("requestCodeReview", request_code_review)?;
    cx.export_function("resetExercise", reset_exercise)?;
    cx.export_function("sendFeedback", send_feedback)?;
    cx.export_function("submit", submit)?;
    cx.export_function("updateExercises", update_exercises)?;
    cx.export_function("waitForSubmission", wait_for_submission)?;
    cx.export_function("getSetting", get_setting)?;
    cx.export_function("listSettings", list_settings)?;
    cx.export_function("migrateExercise", migrate_exercise)?;
    cx.export_function("moveProjectsDir", move_projects_dir)?;
    cx.export_function("resetSettings", reset_settings)?;
    cx.export_function("setSetting", set_setting)?;
    cx.export_function("unsetSetting", unset_setting)?;
    Ok(())
}

#[cfg(test)]
mod test {
    use std::process::Command;
    use tmc_server_mock::mockito::Server;

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new()
            .with_level(LevelFilter::Debug)
            .with_module_level("mockito", LevelFilter::Info)
            .init();
    }

    #[test]
    #[cfg(not(windows))]
    fn jest() {
        init();
        let mut server = Server::new();
        tmc_server_mock::mock_all(&mut server);

        let s = Command::new("npm")
            .args(["run", "jest"])
            .env(
                "TMC_LANGS_MOCK_SERVER_ADDR",
                format!("http://{}", server.host_with_port()),
            )
            .output()
            .expect("running jest failed");
        println!("stdout: {}", String::from_utf8_lossy(&s.stdout));
        println!("stderr: {}", String::from_utf8_lossy(&s.stderr));
        if !s.status.success() {
            panic!("jest test failed")
        }
    }
}
