//! In-process dispatch tests for the `mooc` subcommand tree.
//!
//! The CLI reads its config from process-global env vars
//! (`TMC_LANGS_MOOC_ROOT_URL`, `TMC_LANGS_CONFIG_DIR`,
//! `TMC_LANGS_DEFAULT_PROJECTS_DIR`), so every test that sets them runs under a
//! shared lock.

use clap::Parser;
use std::sync::{Mutex, MutexGuard};
use tmc_langs_cli::{
    app::Cli,
    output::{CliOutput, DataKind, OutputData, OutputResult},
};
use uuid::Uuid;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn output_data(output: CliOutput) -> OutputData {
    match output {
        CliOutput::OutputData(box_output) => *box_output,
        other => panic!("unexpected CLI output: {other:?}"),
    }
}

fn data_of(output: CliOutput) -> DataKind {
    match output_data(output) {
        OutputData {
            result: OutputResult::ExecutedCommand,
            data: Some(data),
            ..
        } => data,
        other => panic!("unexpected CLI output data: {other:?}"),
    }
}

/// Runs a `mooc --client-name test <args>` command in-process against `server`,
/// with the given config/projects directories. Env vars are process-global, so
/// the shared `ENV_LOCK` is held across the whole run.
fn run_mooc_in(
    server: &mockito::Server,
    args: &[&str],
    config_dir: &std::path::Path,
    projects_dir: &std::path::Path,
) -> Result<CliOutput, String> {
    run_mooc_in_with_poll(server, args, config_dir, projects_dir, 5, 5000)
}

/// Like [`run_mooc_in`] but with an explicit grading poll interval and timeout
/// (both in milliseconds), so the blocking submit / wait-for-grading loop polls
/// fast instead of sleeping the real ~2 s interval / ~180 s bound.
fn run_mooc_in_with_poll(
    server: &mockito::Server,
    args: &[&str],
    config_dir: &std::path::Path,
    projects_dir: &std::path::Path,
    poll_interval_ms: u64,
    poll_timeout_ms: u64,
) -> Result<CliOutput, String> {
    let _guard = env_lock();
    // SAFETY: all env access in these tests is serialized by ENV_LOCK.
    unsafe {
        std::env::set_var("TMC_LANGS_MOOC_ROOT_URL", server.url());
        std::env::set_var("TMC_LANGS_CONFIG_DIR", config_dir);
        std::env::set_var("TMC_LANGS_DEFAULT_PROJECTS_DIR", projects_dir);
        std::env::set_var(
            "TMC_LANGS_MOOC_POLL_INTERVAL_MS",
            poll_interval_ms.to_string(),
        );
        std::env::set_var(
            "TMC_LANGS_MOOC_POLL_TIMEOUT_MS",
            poll_timeout_ms.to_string(),
        );
    }
    let mut full = vec!["tmc-langs-cli", "mooc", "--client-name", "test"];
    full.extend_from_slice(args);
    let cli = Cli::parse_from(full);
    tmc_langs_cli::run(cli).map_err(|e| format!("{e:?}"))
}

/// Runs a mooc command with fresh (empty) config/projects directories.
fn run_mooc(server: &mockito::Server, args: &[&str]) -> Result<CliOutput, String> {
    let config_dir = tempfile::tempdir().unwrap();
    let projects_dir = tempfile::tempdir().unwrap();
    run_mooc_in(server, args, config_dir.path(), projects_dir.path())
}

/// Runs a mooc command expecting it to fail, returning the resulting
/// [`tmc_langs_cli::CliError`] so tests can assert on the error `Kind` the CLI
/// would print to stdout.
fn run_mooc_in_expect_error(
    server: &mockito::Server,
    args: &[&str],
    config_dir: &std::path::Path,
    projects_dir: &std::path::Path,
) -> tmc_langs_cli::CliError {
    let _guard = env_lock();
    // SAFETY: all env access in these tests is serialized by ENV_LOCK.
    unsafe {
        std::env::set_var("TMC_LANGS_MOOC_ROOT_URL", server.url());
        std::env::set_var("TMC_LANGS_CONFIG_DIR", config_dir);
        std::env::set_var("TMC_LANGS_DEFAULT_PROJECTS_DIR", projects_dir);
    }
    let mut full = vec!["tmc-langs-cli", "mooc", "--client-name", "test"];
    full.extend_from_slice(args);
    let cli = Cli::parse_from(full);
    tmc_langs_cli::run(cli).expect_err("expected the command to fail")
}

/// Writes a `credentials_mooc.json` for `--client-name test` into `config_dir`,
/// as a successful mooc login would, so a rejected-token path has a file to
/// delete. The wrapper mirrors the stored `{token, obtained_at}` shape; the
/// token carries no refresh token, so a 401 falls straight through to deletion
/// instead of attempting a refresh.
fn write_test_credentials(config_dir: &std::path::Path) -> std::path::PathBuf {
    let dir = config_dir.join("tmc-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("credentials_mooc.json");
    std::fs::write(
        &path,
        serde_json::json!({
            "token": {
                "access_token": "rejected-token",
                "token_type": "bearer",
                "scope": "public"
            },
            "obtained_at": "2026-07-22T00:00:00Z"
        })
        .to_string(),
    )
    .unwrap();
    path
}

/// Writes an *expired* `credentials_mooc.json` that still carries a refresh
/// token, so loading it triggers the proactive refresh path.
fn write_expired_refreshable_credentials(config_dir: &std::path::Path) -> std::path::PathBuf {
    let dir = config_dir.join("tmc-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("credentials_mooc.json");
    std::fs::write(
        &path,
        serde_json::json!({
            "token": {
                "access_token": "expired-access",
                "refresh_token": "stored-refresh",
                "token_type": "bearer",
                "expires_in": 3600,
                "scope": "exercise-services"
            },
            // Obtained well over an hour ago with a one-hour lifetime -> expired.
            "obtained_at": "2000-01-01T00:00:00Z"
        })
        .to_string(),
    )
    .unwrap();
    path
}

/// Writes a *valid* (not expired) `credentials_mooc.json`, so the first request
/// uses the stored token (no proactive refresh) and a 401 on it triggers the
/// on-401 refresh-then-retry path.
fn write_valid_refreshable_credentials(config_dir: &std::path::Path) -> std::path::PathBuf {
    let dir = config_dir.join("tmc-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("credentials_mooc.json");
    std::fs::write(
        &path,
        serde_json::json!({
            "token": {
                "access_token": "valid-access",
                "refresh_token": "stored-refresh",
                "token_type": "bearer",
                // ~1000 year lifetime: unexpired regardless of when the test runs.
                "expires_in": 32_503_680_000_u64,
                "scope": "exercise-services"
            },
            "obtained_at": "2020-01-01T00:00:00Z"
        })
        .to_string(),
    )
    .unwrap();
    path
}

/// Like [`run_mooc_in_expect_error`] but also trusts localhost for bearer tokens,
/// so the mock sees the `Authorization` header and can tell the pre-refresh
/// request apart from the post-refresh retry. Unsets the trust knob before
/// returning so it doesn't leak into other tests sharing the process.
fn run_mooc_trusting_localhost_expect_error(
    server: &mockito::Server,
    args: &[&str],
    config_dir: &std::path::Path,
    projects_dir: &std::path::Path,
) -> tmc_langs_cli::CliError {
    let _guard = env_lock();
    // SAFETY: all env access in these tests is serialized by ENV_LOCK.
    unsafe {
        std::env::set_var("TMC_LANGS_MOOC_ROOT_URL", server.url());
        std::env::set_var("TMC_LANGS_CONFIG_DIR", config_dir);
        std::env::set_var("TMC_LANGS_DEFAULT_PROJECTS_DIR", projects_dir);
        std::env::set_var("TMC_LANGS_MOOC_TRUST_LOCALHOST", "1");
    }
    let mut full = vec!["tmc-langs-cli", "mooc", "--client-name", "test"];
    full.extend_from_slice(args);
    let cli = Cli::parse_from(full);
    let result = tmc_langs_cli::run(cli);
    // SAFETY: still holding ENV_LOCK.
    unsafe {
        std::env::remove_var("TMC_LANGS_MOOC_TRUST_LOCALHOST");
    }
    result.expect_err("expected the command to fail")
}

#[test]
fn mooc_non_auth_retry_after_successful_refresh_keeps_credentials() {
    // Refresh succeeds, but the retry of the original request hits a transient
    // 503. That must NOT be treated as invalid-token: keep the refreshed
    // credentials and surface the 503 as-is. Regression for the retry-error gate.
    let mut server = mockito::Server::new();
    // Pre-refresh request: the stored access token is rejected.
    let _rejected = server
        .mock("GET", "/api/v0/exercise-services/client/courses")
        .match_header("authorization", "Bearer valid-access")
        .with_status(401)
        .with_body(r#"{"message":"invalid token"}"#)
        .expect_at_least(1)
        .create();
    // The refresh itself succeeds, rotating the access token.
    let _refresh = server
        .mock("POST", "/api/v0/main-frontend/oauth/token")
        .with_body(
            serde_json::json!({
                "access_token": "refreshed-access",
                "refresh_token": "rotated-refresh",
                "token_type": "bearer",
                "expires_in": 3600
            })
            .to_string(),
        )
        .expect_at_least(1)
        .create();
    // Post-refresh retry carries the rotated token and hits a transient 5xx.
    let _retry = server
        .mock("GET", "/api/v0/exercise-services/client/courses")
        .match_header("authorization", "Bearer refreshed-access")
        .with_status(503)
        .with_body("service unavailable")
        .expect_at_least(1)
        .create();

    let config_dir = tempfile::tempdir().unwrap();
    let projects_dir = tempfile::tempdir().unwrap();
    let credentials_path = write_valid_refreshable_credentials(config_dir.path());
    assert!(credentials_path.exists());

    let error = run_mooc_trusting_localhost_expect_error(
        &server,
        &["courses"],
        config_dir.path(),
        projects_dir.path(),
    );

    assert!(
        credentials_path.exists(),
        "a non-auth failure on the post-refresh retry must NOT delete the credentials"
    );
    // Propagated as-is, not masked as an invalid-token/auth error.
    assert_error_kind(error, "connection-error");
}

#[test]
fn mooc_transient_refresh_failure_keeps_credentials_and_reports_connection_error() {
    // A 5xx on the proactive refresh is transient: the CLI must keep the
    // credentials (so a retry can succeed) and report `connection-error`, NOT
    // delete them and force a re-login.
    let mut server = mockito::Server::new();
    let _refresh = server
        .mock("POST", "/api/v0/main-frontend/oauth/token")
        .with_status(503)
        .with_body("service unavailable")
        .expect_at_least(1)
        .create();

    let config_dir = tempfile::tempdir().unwrap();
    let projects_dir = tempfile::tempdir().unwrap();
    let credentials_path = write_expired_refreshable_credentials(config_dir.path());
    assert!(credentials_path.exists());

    let error = run_mooc_in_expect_error(
        &server,
        &["courses"],
        config_dir.path(),
        projects_dir.path(),
    );

    assert!(
        credentials_path.exists(),
        "a transient refresh failure must NOT delete the credentials file"
    );
    assert_error_kind(error, "connection-error");
}

#[test]
fn mooc_401_deletes_credentials_and_reports_invalid_token() {
    // A rejected mooc token must delete credentials.json and surface the
    // `invalid-token` kind, mirroring the tmc path. Regression: the 401 handler
    // only downcast `TestMyCodeClientError`, missing `MoocClientError::NotAuthenticated`.
    let mut server = mockito::Server::new();
    let _unauthorized = server
        .mock("GET", "/api/v0/exercise-services/client/courses")
        .with_status(401)
        .with_body(r#"{"message":"invalid token"}"#)
        .expect_at_least(1)
        .create();

    let config_dir = tempfile::tempdir().unwrap();
    let projects_dir = tempfile::tempdir().unwrap();
    let credentials_path = write_test_credentials(config_dir.path());
    assert!(credentials_path.exists());

    let error = run_mooc_in_expect_error(
        &server,
        &["courses"],
        config_dir.path(),
        projects_dir.path(),
    );

    assert!(
        !credentials_path.exists(),
        "credentials.json should have been deleted after a 401"
    );

    match *error.output {
        CliOutput::OutputData(data) => match data.data {
            Some(DataKind::Error { kind, .. }) => {
                assert_eq!(
                    serde_json::to_value(&kind).unwrap(),
                    serde_json::json!("invalid-token")
                );
            }
            other => panic!("expected an Error data kind, got {other:?}"),
        },
        other => panic!("expected OutputData, got {other:?}"),
    }
}

fn assert_error_kind(error: tmc_langs_cli::CliError, expected: &str) {
    match *error.output {
        CliOutput::OutputData(data) => match data.data {
            Some(DataKind::Error { kind, .. }) => {
                assert_eq!(
                    serde_json::to_value(&kind).unwrap(),
                    serde_json::json!(expected)
                );
            }
            other => panic!("expected an Error data kind, got {other:?}"),
        },
        other => panic!("expected OutputData, got {other:?}"),
    }
}

#[test]
fn mooc_403_maps_to_forbidden_kind() {
    // `solve_error_kind` downcast the bare `MoocClientError`, but mooc errors
    // travel the anyhow chain as `Box<MoocClientError>`, so every mooc HTTP error
    // collapsed to `generic`. Pins the 403 -> `forbidden` mapping.
    let mut server = mockito::Server::new();
    let _forbidden = server
        .mock("GET", "/api/v0/exercise-services/client/courses")
        .with_status(403)
        .with_body(r#"{"message":"forbidden"}"#)
        .expect_at_least(1)
        .create();

    let config_dir = tempfile::tempdir().unwrap();
    let projects_dir = tempfile::tempdir().unwrap();
    write_test_credentials(config_dir.path());

    let error = run_mooc_in_expect_error(
        &server,
        &["courses"],
        config_dir.path(),
        projects_dir.path(),
    );
    assert_error_kind(error, "forbidden");
}

#[test]
fn mooc_426_maps_to_obsolete_client_kind() {
    // 426 Upgrade Required sets `obsolete_client`; the CLI must surface it as the
    // `obsolete-client` kind.
    let mut server = mockito::Server::new();
    let _upgrade = server
        .mock("GET", "/api/v0/exercise-services/client/courses")
        .with_status(426)
        .with_body(r#"{"message":"obsolete client"}"#)
        .expect_at_least(1)
        .create();

    let config_dir = tempfile::tempdir().unwrap();
    let projects_dir = tempfile::tempdir().unwrap();
    write_test_credentials(config_dir.path());

    let error = run_mooc_in_expect_error(
        &server,
        &["courses"],
        config_dir.path(),
        projects_dir.path(),
    );
    assert_error_kind(error, "obsolete-client");
}

#[test]
fn mooc_422_not_enrolled_maps_to_not_enrolled_kind() {
    // A 422 with `message_key: "not_enrolled"` must surface as the `not-enrolled`
    // kind (not the opaque `generic`), so the extension can phrase a clear message.
    let mut server = mockito::Server::new();
    let exercise_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
    let _not_enrolled = server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/exercises/{exercise_id}").as_str(),
        )
        .with_status(422)
        .with_body(
            serde_json::json!({
                "errors": [],
                "message": "not enrolled to this course",
                "message_key": "not_enrolled",
                "metadata": null,
                "type": "validation_error",
            })
            .to_string(),
        )
        .expect_at_least(1)
        .create();

    let config_dir = tempfile::tempdir().unwrap();
    let projects_dir = tempfile::tempdir().unwrap();
    write_test_credentials(config_dir.path());

    let error = run_mooc_in_expect_error(
        &server,
        &["exercise", "--exercise-id", exercise_id],
        config_dir.path(),
        projects_dir.path(),
    );
    assert_error_kind(error, "not-enrolled");
}

#[test]
fn course_updates_subcommand_is_removed() {
    // Regression: `CourseUpdates` was redundant with `check-exercise-updates` and
    // only ever `todo!()`-panicked, so it was removed. clap must now reject it.
    let result = Cli::try_parse_from([
        "tmc-langs-cli",
        "mooc",
        "--client-name",
        "test",
        "course-updates",
    ]);
    assert!(
        result.is_err(),
        "`mooc course-updates` should no longer be a valid subcommand"
    );

    // the surviving update-check subcommand still parses
    Cli::try_parse_from([
        "tmc-langs-cli",
        "mooc",
        "--client-name",
        "test",
        "check-exercise-updates",
    ])
    .expect("check-exercise-updates should still parse");
}

#[test]
fn dispatches_courses() {
    let mut server = mockito::Server::new();
    server
        .mock("GET", "/api/v0/exercise-services/client/courses")
        .with_body(
            serde_json::json!([{
                "id": Uuid::new_v4(),
                "slug": "mockslug",
                "name": "mockname",
                "description": "mockdesc",
                "organization_name": "mockorg",
            }])
            .to_string(),
        )
        .create();
    let output = run_mooc(&server, &["courses"]).unwrap();
    match data_of(output) {
        DataKind::MoocCourses(courses) => {
            assert_eq!(courses.len(), 1);
            assert_eq!(courses[0].name, "mockname");
        }
        other => panic!("expected MoocCourses, got {other:?}"),
    }
}

#[test]
fn dispatches_course() {
    let mut server = mockito::Server::new();
    let course_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
    server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/courses/{course_id}").as_str(),
        )
        .with_body(
            serde_json::json!({
                "id": course_id,
                "slug": "mockslug",
                "name": "mockname",
                "description": null,
                "organization_name": "mockorg",
            })
            .to_string(),
        )
        .create();
    let output = run_mooc(&server, &["course", "--course-id", course_id]).unwrap();
    match data_of(output) {
        DataKind::MoocCourse(course) => assert_eq!(course.name, "mockname"),
        other => panic!("expected MoocCourse, got {other:?}"),
    }
}

#[test]
fn dispatches_course_exercises() {
    let mut server = mockito::Server::new();
    let course_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
    server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/courses/{course_id}/exercises").as_str(),
        )
        .with_body(
            serde_json::json!([{
                "slide_id": Uuid::new_v4(),
                "exercise_id": Uuid::new_v4(),
                "course_id": Uuid::new_v4(),
                "exercise_name": "mockname",
                "exercise_order_number": 0,
                "tasks": [{
                    "task_id": Uuid::new_v4(),
                    "order_number": 0,
                    "assignment": [],
                    "public_spec": {
                        "type": "editor",
                        "archive_name": "stub.tar.zst",
                        "stub_download_url": "http://example.com/stub.tar.zst",
                        "student_file_paths": ["src/main.py"],
                        "checksum": "abcd1234"
                    },
                    "model_solution_spec": null,
                    "exercise_service_slug": "tmc"
                }],
            }])
            .to_string(),
        )
        .create();
    let output = run_mooc(&server, &["course-exercises", "--course-id", course_id]).unwrap();
    match data_of(output) {
        DataKind::MoocExerciseSlides(slides) => {
            assert_eq!(slides.len(), 1);
            assert_eq!(slides[0].exercise_name, "mockname");
            assert_eq!(slides[0].tasks.len(), 1);
        }
        other => panic!("expected MoocExerciseSlides, got {other:?}"),
    }
}

#[test]
fn dispatches_course_progress() {
    let mut server = mockito::Server::new();
    let course_id = "5f9e0a1c-3b2d-4e6f-8a9b-0c1d2e3f4a5b";
    server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/courses/{course_id}/progress").as_str(),
        )
        .with_body(
            serde_json::json!({
                "course_id": course_id,
                "exercises": [
                    {
                        "exercise_id": "a1b2c3d4-0000-4000-8000-000000000001",
                        "score_given": 1.0,
                        "score_maximum": 1,
                        "completed": true,
                        "attempted": true
                    },
                    {
                        "exercise_id": "a1b2c3d4-0000-4000-8000-000000000002",
                        "score_given": 0.5,
                        "score_maximum": 2,
                        "completed": false,
                        "attempted": true
                    },
                    {
                        "exercise_id": "a1b2c3d4-0000-4000-8000-000000000003",
                        "score_given": 0.0,
                        "score_maximum": 3,
                        "completed": false,
                        "attempted": false
                    }
                ]
            })
            .to_string(),
        )
        .create();
    let output = run_mooc(&server, &["course-progress", "--course-id", course_id]).unwrap();
    match data_of(output) {
        DataKind::MoocCourseProgress(progress) => {
            assert_eq!(progress.course_id, Uuid::parse_str(course_id).unwrap());
            assert_eq!(progress.exercises.len(), 3);
            assert_eq!(progress.exercises[0].score_given, 1.0);
            assert!(progress.exercises[0].completed);
            assert_eq!(progress.exercises[1].score_given, 0.5);
            assert!(!progress.exercises[2].attempted);
        }
        other => panic!("expected MoocCourseProgress, got {other:?}"),
    }
}

#[test]
fn dispatches_exercise() {
    let mut server = mockito::Server::new();
    let exercise_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
    server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/exercises/{exercise_id}").as_str(),
        )
        .with_body(
            serde_json::json!({
                "slide_id": Uuid::new_v4(),
                "exercise_id": exercise_id,
                "course_id": Uuid::new_v4(),
                "exercise_name": "mockname",
                "exercise_order_number": 0,
                "tasks": [],
            })
            .to_string(),
        )
        .create();
    let output = run_mooc(&server, &["exercise", "--exercise-id", exercise_id]).unwrap();
    match data_of(output) {
        DataKind::MoocExerciseSlide(slide) => assert_eq!(slide.exercise_name, "mockname"),
        other => panic!("expected MoocExerciseSlide, got {other:?}"),
    }
}

#[test]
fn dispatches_download_exercise() {
    let mut server = mockito::Server::new();
    let exercise_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
    let stub_download_url = format!("{}/files/stub.tar.zst", server.url());
    server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/exercises/{exercise_id}").as_str(),
        )
        .with_body(
            serde_json::json!({
                "slide_id": Uuid::new_v4(),
                "exercise_id": exercise_id,
                "course_id": Uuid::new_v4(),
                "exercise_name": "mockname",
                "exercise_order_number": 0,
                "tasks": [{
                    "task_id": Uuid::new_v4(),
                    "order_number": 0,
                    "assignment": [],
                    "public_spec": {
                        "type": "editor",
                        "archive_name": "stub.tar.zst",
                        "stub_download_url": stub_download_url,
                        "student_file_paths": ["src/main.py"],
                        "checksum": "abcd1234"
                    },
                    "model_solution_spec": null,
                    "exercise_service_slug": "tmc"
                }],
            })
            .to_string(),
        )
        .create();
    server
        .mock("GET", "/files/stub.tar.zst")
        .with_body(make_tar_zst(&[("src/main.py", b"print('downloaded')")]))
        .create();

    let target = tempfile::tempdir().unwrap();
    let output = run_mooc(
        &server,
        &[
            "download-exercise",
            "--exercise-id",
            exercise_id,
            "--target",
            target.path().to_str().unwrap(),
        ],
    )
    .unwrap();
    // download-exercise carries no data payload, just a finished status
    let data = output_data(output);
    assert!(matches!(data.result, OutputResult::ExecutedCommand));
    let extracted = std::fs::read_to_string(target.path().join("src/main.py")).unwrap();
    assert_eq!(extracted, "print('downloaded')");
}

#[test]
fn dispatches_check_exercise_updates_empty() {
    // Empty projects dir: no local exercises, so no server request; returns an
    // empty `MoocUpdatedExercises`.
    let server = mockito::Server::new();
    let output = run_mooc(&server, &["check-exercise-updates"]).unwrap();
    match data_of(output) {
        DataKind::MoocUpdatedExercises(ids) => assert!(ids.is_empty()),
        other => panic!("expected MoocUpdatedExercises, got {other:?}"),
    }
}

#[test]
fn dispatches_download_or_update_course_exercises() {
    // The extension's bulk mooc download: downloaded + skipped + failed
    // (browser-only + unknown) in one dispatch.
    let mut server = mockito::Server::new();
    let course_id = Uuid::new_v4();
    let ex_skip = Uuid::new_v4();
    let task_skip = Uuid::new_v4();
    let ex_new = Uuid::new_v4();
    let task_new = Uuid::new_v4();
    let ex_browser = Uuid::new_v4();
    let ex_missing = Uuid::new_v4();

    let config_dir = tempfile::tempdir().unwrap();
    let projects_root = tempfile::tempdir().unwrap();
    // The CLI derives the projects dir as <root>/<client-name> ("test" here).
    let projects_dir = projects_root.path().join("test");
    // pre-seed the "skip" exercise with a matching checksum
    let course_config = projects_dir.join("mooc/course/course_config.toml");
    std::fs::create_dir_all(course_config.parent().unwrap()).unwrap();
    std::fs::write(
        &course_config,
        format!(
            r#"
course_id = "{course_id}"
instance_id = "{course_id}"
course = "Course"
directory = "course"

[exercises."{ex_skip}"]
name = "Skip Me"
task_id = "{task_skip}"
checksum = "same checksum"
directory = "skip-me"
"#
        ),
    )
    .unwrap();
    std::fs::create_dir_all(projects_dir.join("mooc/course/skip-me")).unwrap();

    let stub_url = format!("{}/files/new.tar.zst", server.url());
    server
        .mock("GET", "/api/v0/exercise-services/client/courses")
        .with_body(
            serde_json::json!([{
                "id": course_id, "slug": "course", "name": "Course",
                "description": null, "organization_name": "org",
            }])
            .to_string(),
        )
        .create();
    server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/courses/{course_id}/exercises").as_str(),
        )
        .with_body(
            serde_json::json!([
                {
                    "slide_id": Uuid::new_v4(), "exercise_id": ex_skip,
                    "course_id": Uuid::new_v4(),
                    "exercise_name": "Skip Me", "exercise_order_number": 0,
                    "tasks": [{
                        "task_id": task_skip, "order_number": 0, "assignment": [],
                        "public_spec": {
                            "type": "editor", "archive_name": "s.tar.zst",
                            "stub_download_url": stub_url,
                            "student_file_paths": ["src/main.py"], "checksum": "same checksum"
                        },
                        "model_solution_spec": null, "exercise_service_slug": "tmc"
                    }],
                },
                {
                    "slide_id": Uuid::new_v4(), "exercise_id": ex_new,
                    "course_id": Uuid::new_v4(),
                    "exercise_name": "New Exercise", "exercise_order_number": 1,
                    "tasks": [{
                        "task_id": task_new, "order_number": 0, "assignment": [],
                        "public_spec": {
                            "type": "editor", "archive_name": "n.tar.zst",
                            "stub_download_url": stub_url,
                            "student_file_paths": ["src/main.py"], "checksum": "new checksum"
                        },
                        "model_solution_spec": null, "exercise_service_slug": "tmc"
                    }],
                },
                {
                    "slide_id": Uuid::new_v4(), "exercise_id": ex_browser,
                    "course_id": Uuid::new_v4(),
                    "exercise_name": "Browser Exercise", "exercise_order_number": 2,
                    "tasks": [{
                        "task_id": Uuid::new_v4(), "order_number": 0, "assignment": [],
                        "public_spec": {
                            "type": "browser", "archive_name": "b.tar.zst",
                            "stub_download_url": stub_url,
                            "student_file_paths": [], "checksum": "bsum",
                            "browser_test": { "runtime": "python", "script": "" }
                        },
                        "model_solution_spec": null, "exercise_service_slug": "tmc"
                    }],
                }
            ])
            .to_string(),
        )
        .create();
    server
        .mock("GET", "/files/new.tar.zst")
        .with_body(make_tar_zst(&[("src/main.py", b"print('new')")]))
        .create();

    let output = run_mooc_in(
        &server,
        &[
            "download-or-update-course-exercises",
            "--exercise-id",
            &ex_skip.to_string(),
            &ex_new.to_string(),
            &ex_browser.to_string(),
            &ex_missing.to_string(),
        ],
        config_dir.path(),
        projects_root.path(),
    )
    .unwrap();

    match data_of(output) {
        DataKind::MoocExerciseDownload(result) => {
            assert_eq!(result.downloaded.len(), 1, "one downloaded");
            // results are keyed by the requested exercise id, not the editor task id
            assert_eq!(result.downloaded[0].exercise_id, ex_new);
            assert_eq!(result.skipped.len(), 1, "one skipped");
            assert_eq!(result.skipped[0].exercise_id, ex_skip);
            assert_eq!(
                result.failed.expect("failures").len(),
                2,
                "browser + missing"
            );
        }
        other => panic!("expected MoocExerciseDownload, got {other:?}"),
    }
}

#[test]
fn dispatches_update_exercises() {
    // `update-exercises` takes no exercise ids: it scans the locally tracked
    // mooc exercises' course configs and refreshes any whose server checksum
    // changed, downloading the new stub archive in the process.
    let mut server = mockito::Server::new();
    let course_id = Uuid::new_v4();
    let ex_stale = Uuid::new_v4();
    let task_stale = Uuid::new_v4();

    let config_dir = tempfile::tempdir().unwrap();
    let projects_root = tempfile::tempdir().unwrap();
    // The CLI derives the projects dir as <root>/<client-name> ("test" here).
    let projects_dir = projects_root.path().join("test");
    // pre-seed the tracked exercise with a stale checksum
    let course_config = projects_dir.join("mooc/course/course_config.toml");
    std::fs::create_dir_all(course_config.parent().unwrap()).unwrap();
    std::fs::write(
        &course_config,
        format!(
            r#"
course_id = "{course_id}"
instance_id = "{course_id}"
course = "Course"
directory = "course"

[exercises."{ex_stale}"]
name = "Stale Exercise"
task_id = "{task_stale}"
checksum = "old checksum"
directory = "stale-exercise"
"#
        ),
    )
    .unwrap();
    std::fs::create_dir_all(projects_dir.join("mooc/course/stale-exercise")).unwrap();

    let stub_url = format!("{}/files/updated.tar.zst", server.url());
    server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/courses/{course_id}/exercises").as_str(),
        )
        .with_body(
            serde_json::json!([
                {
                    "slide_id": Uuid::new_v4(), "exercise_id": ex_stale,
                    "course_id": Uuid::new_v4(),
                    "exercise_name": "Stale Exercise", "exercise_order_number": 0,
                    "tasks": [{
                        "task_id": task_stale, "order_number": 0, "assignment": [],
                        "public_spec": {
                            "type": "editor", "archive_name": "u.tar.zst",
                            "stub_download_url": stub_url,
                            "student_file_paths": ["src/main.py"], "checksum": "new checksum"
                        },
                        "model_solution_spec": null, "exercise_service_slug": "tmc"
                    }],
                }
            ])
            .to_string(),
        )
        .create();
    server
        .mock("GET", "/files/updated.tar.zst")
        .with_body(make_tar_zst(&[("src/main.py", b"print('updated')")]))
        .create();

    let output = run_mooc_in(
        &server,
        &["update-exercises"],
        config_dir.path(),
        projects_root.path(),
    )
    .unwrap();

    match data_of(output) {
        DataKind::MoocExerciseDownload(result) => {
            assert_eq!(result.downloaded.len(), 1, "one downloaded");
            assert_eq!(result.downloaded[0].exercise_id, ex_stale);
            assert!(result.skipped.is_empty());
            assert!(result.failed.is_none());
        }
        other => panic!("expected MoocExerciseDownload, got {other:?}"),
    }

    let extracted =
        std::fs::read_to_string(projects_dir.join("mooc/course/stale-exercise/src/main.py"))
            .unwrap();
    assert_eq!(extracted, "print('updated')");
}

#[test]
fn download_or_update_course_exercises_rejects_download_template() {
    // mooc has no template-vs-submission concept (the stub archive is the only
    // downloadable), so the bulk command rejects `--download-template`.
    let result = Cli::try_parse_from([
        "tmc-langs-cli",
        "mooc",
        "--client-name",
        "test",
        "download-or-update-course-exercises",
        "--download-template",
        "--exercise-id",
        "df5ee6c1-57d1-43b6-b39e-5d72119edb5f",
    ]);
    assert!(
        result.is_err(),
        "mooc download-or-update-course-exercises must reject --download-template"
    );
}

#[test]
fn download_or_update_course_exercises_with_course_id_skips_scan() {
    // `--course-id` fetches only that course's slides; the all-courses scan
    // (`GET courses`) is NOT mocked, so a regression to it 501s.
    let mut server = mockito::Server::new();
    let course_id = Uuid::new_v4();
    let ex_new = Uuid::new_v4();
    let task_new = Uuid::new_v4();

    let config_dir = tempfile::tempdir().unwrap();
    let projects_root = tempfile::tempdir().unwrap();

    let stub_url = format!("{}/files/new.tar.zst", server.url());
    server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/courses/{course_id}").as_str(),
        )
        .with_body(
            serde_json::json!({
                "id": course_id, "slug": "course", "name": "Course",
                "description": null, "organization_name": "org",
            })
            .to_string(),
        )
        .expect_at_least(1)
        .create();
    server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/courses/{course_id}/exercises").as_str(),
        )
        .with_body(
            serde_json::json!([{
                "slide_id": Uuid::new_v4(), "exercise_id": ex_new, "course_id": course_id,
                "exercise_name": "New Exercise", "exercise_order_number": 0,
                "tasks": [{
                    "task_id": task_new, "order_number": 0, "assignment": [],
                    "public_spec": {
                        "type": "editor", "archive_name": "n.tar.zst",
                        "stub_download_url": stub_url,
                        "student_file_paths": ["src/main.py"], "checksum": "new checksum"
                    },
                    "model_solution_spec": null, "exercise_service_slug": "tmc"
                }],
            }])
            .to_string(),
        )
        .expect_at_least(1)
        .create();
    server
        .mock("GET", "/files/new.tar.zst")
        .with_body(make_tar_zst(&[("src/main.py", b"print('new')")]))
        .create();

    let output = run_mooc_in(
        &server,
        &[
            "download-or-update-course-exercises",
            "--course-id",
            &course_id.to_string(),
            "--exercise-id",
            &ex_new.to_string(),
        ],
        config_dir.path(),
        projects_root.path(),
    )
    .unwrap();

    match data_of(output) {
        DataKind::MoocExerciseDownload(result) => {
            assert_eq!(result.downloaded.len(), 1);
            assert_eq!(result.downloaded[0].exercise_id, ex_new);
        }
        other => panic!("expected MoocExerciseDownload, got {other:?}"),
    }
}

/// Mounts a `POST /api/v0/main-frontend/oauth/token` refresh endpoint that
/// returns a rotated token pair. Mirrors `mock_refresh` in
/// `mooc_credentials.rs`'s own tests.
fn mock_oauth_refresh(
    server: &mut mockito::Server,
    new_access: &str,
    new_refresh: &str,
) -> mockito::Mock {
    server
        .mock("POST", "/api/v0/main-frontend/oauth/token")
        .with_body(
            serde_json::json!({
                "access_token": new_access,
                "refresh_token": new_refresh,
                "token_type": "bearer",
                "expires_in": 3600,
            })
            .to_string(),
        )
        .create()
}

#[test]
fn download_or_update_course_exercises_transient_refresh_failure_recovers() {
    // A 401 on one item's archive download whose refresh attempt itself fails
    // transiently (a network blip talking to the token endpoint, not a
    // rejection of the refresh token) must not abort the batch or the item:
    // the refresh is retried once more by `call_with_refresh`, succeeds, and
    // the failed item's download is retried in place. All five items must
    // end up downloaded and the credentials must survive untouched.
    let mut server = mockito::Server::new();
    let course_id = Uuid::new_v4();
    let exercise_ids: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();
    let task_ids: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();
    let stub_urls: Vec<String> = (0..5)
        .map(|i| format!("{}/files/ex{i}.tar.zst", server.url()))
        .collect();

    let slides: Vec<serde_json::Value> = (0..5)
        .map(|i| {
            serde_json::json!({
                "slide_id": Uuid::new_v4(), "exercise_id": exercise_ids[i], "course_id": course_id,
                "exercise_name": format!("Exercise {i}"), "exercise_order_number": i,
                "tasks": [{
                    "task_id": task_ids[i], "order_number": 0, "assignment": [],
                    "public_spec": {
                        "type": "editor", "archive_name": format!("ex{i}.tar.zst"),
                        "stub_download_url": stub_urls[i],
                        "student_file_paths": ["src/main.py"], "checksum": format!("checksum-{i}")
                    },
                    "model_solution_spec": null, "exercise_service_slug": "tmc"
                }],
            })
        })
        .collect();
    server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/courses/{course_id}").as_str(),
        )
        .with_body(
            serde_json::json!({
                "id": course_id, "slug": "course", "name": "Course",
                "description": null, "organization_name": "org",
            })
            .to_string(),
        )
        .expect_at_least(1)
        .create();
    server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/courses/{course_id}/exercises").as_str(),
        )
        .with_body(serde_json::Value::Array(slides).to_string())
        .create();

    for (i, url) in stub_urls.iter().enumerate() {
        if i == 2 {
            continue; // item 3 (index 2) gets the special 401-then-success sequencing below
        }
        let path = url.strip_prefix(&server.url()).unwrap();
        server
            .mock("GET", path)
            .with_body(make_tar_zst(&[(
                "src/main.py",
                format!("print('{i}')").as_bytes(),
            )]))
            .create();
    }
    let item3_path = stub_urls[2]
        .strip_prefix(&server.url())
        .unwrap()
        .to_string();
    // Item 3's first download attempt is rejected...
    server
        .mock("GET", item3_path.as_str())
        .with_status(401)
        .with_body(r#"{"message":"invalid token"}"#)
        .expect(1)
        .create();
    // ...and the retried download (after the refresh below) succeeds.
    server
        .mock("GET", item3_path.as_str())
        .with_body(make_tar_zst(&[("src/main.py", b"print('2')")]))
        .create();

    // The refresh triggered by item 3's 401 fails transiently once...
    server
        .mock("POST", "/api/v0/main-frontend/oauth/token")
        .with_status(503)
        .with_body("service unavailable")
        .expect(1)
        .create();
    // ...then succeeds on the retry `call_with_refresh` makes in place.
    let refresh_ok = mock_oauth_refresh(&mut server, "refreshed-access", "refreshed-refresh");

    let config_dir = tempfile::tempdir().unwrap();
    let projects_root = tempfile::tempdir().unwrap();
    let credentials_path = write_valid_refreshable_credentials(config_dir.path());

    let mut args = vec![
        "download-or-update-course-exercises".to_string(),
        "--course-id".to_string(),
        course_id.to_string(),
        "--exercise-id".to_string(),
    ];
    args.extend(exercise_ids.iter().map(Uuid::to_string));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    let output = run_mooc_in(&server, &args, config_dir.path(), projects_root.path()).unwrap();

    match data_of(output) {
        DataKind::MoocExerciseDownload(result) => {
            assert_eq!(
                result.downloaded.len(),
                5,
                "all five exercises should end up downloaded: {result:?}"
            );
            assert!(result.failed.is_none(), "no failures expected: {result:?}");
            assert!(result.not_attempted.is_empty());
            assert!(!result.stopped_for_auth);
        }
        other => panic!("expected MoocExerciseDownload, got {other:?}"),
    }
    refresh_ok.assert();
    assert!(
        credentials_path.exists(),
        "a transient refresh hiccup must not delete valid credentials"
    );
}

#[test]
fn download_or_update_course_exercises_permanent_auth_failure_stops_batch() {
    // A permanent refresh rejection (the refresh token itself is rejected) on
    // item 3 of 5 must stop the batch there: items 1-2 stay downloaded, item 3
    // is reported failed, items 4-5 are reported `not_attempted` (never
    // tried), `stopped_for_auth` is set, and the credentials are deleted.
    let mut server = mockito::Server::new();
    let course_id = Uuid::new_v4();
    let exercise_ids: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();
    let task_ids: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();
    let stub_urls: Vec<String> = (0..5)
        .map(|i| format!("{}/files/ex{i}.tar.zst", server.url()))
        .collect();

    let slides: Vec<serde_json::Value> = (0..5)
        .map(|i| {
            serde_json::json!({
                "slide_id": Uuid::new_v4(), "exercise_id": exercise_ids[i], "course_id": course_id,
                "exercise_name": format!("Exercise {i}"), "exercise_order_number": i,
                "tasks": [{
                    "task_id": task_ids[i], "order_number": 0, "assignment": [],
                    "public_spec": {
                        "type": "editor", "archive_name": format!("ex{i}.tar.zst"),
                        "stub_download_url": stub_urls[i],
                        "student_file_paths": ["src/main.py"], "checksum": format!("checksum-{i}")
                    },
                    "model_solution_spec": null, "exercise_service_slug": "tmc"
                }],
            })
        })
        .collect();
    server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/courses/{course_id}").as_str(),
        )
        .with_body(
            serde_json::json!({
                "id": course_id, "slug": "course", "name": "Course",
                "description": null, "organization_name": "org",
            })
            .to_string(),
        )
        .expect_at_least(1)
        .create();
    server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/courses/{course_id}/exercises").as_str(),
        )
        .with_body(serde_json::Value::Array(slides).to_string())
        .create();

    // Items 1-2 (index 0-1) download normally. Items 4-5 (index 3-4) are
    // deliberately left unmocked: the batch must stop before ever reaching
    // them, so a request there would mean a regression.
    for (i, url) in stub_urls.iter().take(2).enumerate() {
        let path = url.strip_prefix(&server.url()).unwrap();
        server
            .mock("GET", path)
            .with_body(make_tar_zst(&[(
                "src/main.py",
                format!("print('{i}')").as_bytes(),
            )]))
            .create();
    }
    // Item 3 (index 2) is rejected...
    let item3_path = stub_urls[2]
        .strip_prefix(&server.url())
        .unwrap()
        .to_string();
    server
        .mock("GET", item3_path.as_str())
        .with_status(401)
        .with_body(r#"{"message":"invalid token"}"#)
        .expect(1)
        .create();
    // ...and the refresh triggered by that rejection is permanently denied.
    let refresh_rejected = server
        .mock("POST", "/api/v0/main-frontend/oauth/token")
        .with_status(400)
        .with_body(r#"{"error":"invalid_grant"}"#)
        .expect(1)
        .create();

    let config_dir = tempfile::tempdir().unwrap();
    let projects_root = tempfile::tempdir().unwrap();
    let credentials_path = write_valid_refreshable_credentials(config_dir.path());

    let mut args = vec![
        "download-or-update-course-exercises".to_string(),
        "--course-id".to_string(),
        course_id.to_string(),
        "--exercise-id".to_string(),
    ];
    args.extend(exercise_ids.iter().map(Uuid::to_string));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    let output = run_mooc_in(&server, &args, config_dir.path(), projects_root.path()).unwrap();

    match data_of(output) {
        DataKind::MoocExerciseDownload(result) => {
            assert_eq!(result.downloaded.len(), 2, "items 1-2: {result:?}");
            let downloaded_ids: std::collections::HashSet<_> =
                result.downloaded.iter().map(|d| d.exercise_id).collect();
            assert_eq!(
                downloaded_ids,
                [exercise_ids[0], exercise_ids[1]].into_iter().collect()
            );

            let failed = result
                .failed
                .as_ref()
                .expect("item 3 must be reported failed");
            assert_eq!(failed.len(), 1);
            assert_eq!(failed[0].0.exercise_id, exercise_ids[2]);

            assert_eq!(result.not_attempted.len(), 2, "items 4-5: {result:?}");
            let not_attempted_ids: std::collections::HashSet<_> =
                result.not_attempted.iter().map(|d| d.exercise_id).collect();
            assert_eq!(
                not_attempted_ids,
                [exercise_ids[3], exercise_ids[4]].into_iter().collect()
            );

            assert!(result.stopped_for_auth);
        }
        other => panic!("expected MoocExerciseDownload, got {other:?}"),
    }
    refresh_rejected.assert();
    assert!(
        !credentials_path.exists(),
        "a permanently rejected refresh must delete the credentials"
    );
}

#[test]
fn dispatches_list_local_course_exercises() {
    // `list-local-course-exercises --course-id` reads the local config (no HTTP),
    // returning each downloaded exercise's slug (= on-disk dir), id, and path.
    let server = mockito::Server::new();
    let course_id = Uuid::new_v4();
    let exercise_id = Uuid::new_v4();
    let task_id = Uuid::new_v4();

    let config_dir = tempfile::tempdir().unwrap();
    let projects_root = tempfile::tempdir().unwrap();
    // the CLI derives the projects dir as <root>/<client-name>; client is "test".
    let projects_dir = projects_root.path().join("test");
    let course_config = projects_dir.join("mooc/my-course/course_config.toml");
    std::fs::create_dir_all(course_config.parent().unwrap()).unwrap();
    std::fs::write(
        &course_config,
        format!(
            r#"
course_id = "{course_id}"
instance_id = "{course_id}"
course = "My Course"
directory = "my-course"

[exercises."{exercise_id}"]
name = "Exercise 1"
task_id = "{task_id}"
checksum = "abcd1234"
directory = "ex-1"
"#
        ),
    )
    .unwrap();
    // the exercise dir must exist or the config loader prunes the entry
    std::fs::create_dir_all(projects_dir.join("mooc/my-course/ex-1")).unwrap();

    let output = run_mooc_in(
        &server,
        &[
            "list-local-course-exercises",
            "--course-id",
            &course_id.to_string(),
        ],
        config_dir.path(),
        projects_root.path(),
    )
    .unwrap();

    match data_of(output) {
        DataKind::LocalMoocExercises(exercises) => {
            assert_eq!(exercises.len(), 1);
            assert_eq!(exercises[0].exercise_id, exercise_id);
            assert_eq!(exercises[0].exercise_slug, "ex-1");
            assert!(exercises[0].exercise_path.ends_with("mooc/my-course/ex-1"));
        }
        other => panic!("expected LocalMoocExercises, got {other:?}"),
    }
}

/// Creates a temp directory holding a minimal submittable project. The
/// `requirements.txt` marker makes the python3 plugin recognize it, so
/// `compress_project_to` finds a matching plugin during submit.
fn submittable_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.py"), b"print('hi')").unwrap();
    std::fs::write(dir.path().join("requirements.txt"), b"").unwrap();
    dir
}

/// Serializes a `MoocSubmissionStatus` payload to JSON so tests can assert on the
/// externally-tagged `"NoGradingYet" | {"Grading":{...}}` wire shape without
/// depending on the enum types directly.
fn submission_status_json(output: CliOutput) -> serde_json::Value {
    match data_of(output) {
        DataKind::MoocSubmissionStatus(status) => serde_json::to_value(&status).unwrap(),
        other => panic!("expected MoocSubmissionStatus, got {other:?}"),
    }
}

/// IDs shared by the submit/grading dispatch tests.
const SUBMIT_EXERCISE_ID: &str = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
const SUBMIT_SLIDE_ID: &str = "e7bd5a07-1b83-4c97-91f2-e48cccf66b2a";
const SUBMIT_TASK_ID: &str = "816ac03a-a713-4804-9ea6-3eb5e278ec2b";

/// Mounts the exercise-slide lookup the submit flow does to resolve the slide
/// and editor task ids from the exercise id.
fn mock_exercise_for_submit(server: &mut mockito::Server) -> mockito::Mock {
    server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/exercises/{SUBMIT_EXERCISE_ID}").as_str(),
        )
        .with_body(
            serde_json::json!({
                "slide_id": SUBMIT_SLIDE_ID,
                "exercise_id": SUBMIT_EXERCISE_ID,
                "course_id": Uuid::new_v4(),
                "exercise_name": "Submittable",
                "exercise_order_number": 0,
                "tasks": [{
                    "task_id": SUBMIT_TASK_ID,
                    "order_number": 0,
                    "assignment": [],
                    "public_spec": {
                        "type": "editor",
                        "archive_name": "stub.tar.zst",
                        "stub_download_url": "http://example.com/stub.tar.zst",
                        "student_file_paths": ["src/main.py"],
                        "checksum": "abcd1234"
                    },
                    "model_solution_spec": null,
                    "exercise_service_slug": "tmc"
                }],
            })
            .to_string(),
        )
        .expect_at_least(1)
        .create()
}

/// Mounts a submit endpoint returning `submission_id`.
fn mock_submit(server: &mut mockito::Server, submission_id: &str) -> mockito::Mock {
    server
        .mock(
            "POST",
            format!("/api/v0/exercise-services/client/exercises/{SUBMIT_EXERCISE_ID}/submit")
                .as_str(),
        )
        .with_body(serde_json::json!({ "submission_id": submission_id }).to_string())
        .create()
}

fn grading_path(submission_id: &str) -> String {
    format!("/api/v0/exercise-services/client/submissions/{submission_id}/grading")
}

#[test]
fn blocking_submit_polls_until_fully_graded() {
    // A blocking submit polls grading until terminal: first `NoGradingYet`, then
    // `FullyGraded`.
    let mut server = mockito::Server::new();
    let submission_id = "11111111-1111-1111-1111-111111111111";
    let _exercise = mock_exercise_for_submit(&mut server);
    let _submit = mock_submit(&mut server, submission_id);
    // First poll: not graded yet (bounded so the next mock takes over).
    let _pending = server
        .mock("GET", grading_path(submission_id).as_str())
        .with_body(serde_json::json!("NoGradingYet").to_string())
        .expect(1)
        .create();
    // Subsequent polls: fully graded.
    let _graded = server
        .mock("GET", grading_path(submission_id).as_str())
        .with_body(
            serde_json::json!({
                "Grading": {
                    "grading_progress": "FullyGraded",
                    "score_given": 1.0,
                    "grading_started_at": "2026-07-21T00:00:00Z",
                    "grading_completed_at": "2026-07-21T00:00:01Z",
                    "feedback_json": null,
                    "feedback_text": "All tests passed"
                }
            })
            .to_string(),
        )
        .expect_at_least(1)
        .create();

    let project = submittable_project();
    let output = run_mooc(
        &server,
        &[
            "submit",
            "--exercise-id",
            SUBMIT_EXERCISE_ID,
            "--submission-path",
            project.path().to_str().unwrap(),
        ],
    )
    .unwrap();

    let json = submission_status_json(output);
    assert_eq!(json["Grading"]["grading_progress"], "FullyGraded");
    assert_eq!(json["Grading"]["score_given"], 1.0);
    assert_eq!(json["Grading"]["feedback_text"], "All tests passed");
}

#[test]
fn blocking_submit_tolerates_transient_grading_errors() {
    // A transient poll error (two 500s) must not abort the blocking submit; it
    // keeps polling until the terminal status.
    let mut server = mockito::Server::new();
    let submission_id = "44444444-4444-4444-4444-444444444444";
    let _exercise = mock_exercise_for_submit(&mut server);
    let _submit = mock_submit(&mut server, submission_id);
    // First two polls fail (bounded so the next mock takes over).
    let _errors = server
        .mock("GET", grading_path(submission_id).as_str())
        .with_status(500)
        .expect(2)
        .create();
    // Subsequent polls: fully graded.
    let _graded = server
        .mock("GET", grading_path(submission_id).as_str())
        .with_body(
            serde_json::json!({
                "Grading": {
                    "grading_progress": "FullyGraded",
                    "score_given": 1.0,
                    "grading_started_at": "2026-07-21T00:00:00Z",
                    "grading_completed_at": "2026-07-21T00:00:01Z",
                    "feedback_json": null,
                    "feedback_text": "All tests passed"
                }
            })
            .to_string(),
        )
        .expect_at_least(1)
        .create();

    let project = submittable_project();
    let output = run_mooc(
        &server,
        &[
            "submit",
            "--exercise-id",
            SUBMIT_EXERCISE_ID,
            "--submission-path",
            project.path().to_str().unwrap(),
        ],
    )
    .unwrap();

    let json = submission_status_json(output);
    assert_eq!(json["Grading"]["grading_progress"], "FullyGraded");
    assert_eq!(json["Grading"]["feedback_text"], "All tests passed");
}

#[test]
fn blocking_submit_errors_when_grading_fails_until_timeout() {
    // Grading failing for the entire poll window surfaces as `Err` at the deadline.
    let mut server = mockito::Server::new();
    let submission_id = "55555555-5555-5555-5555-555555555555";
    let _exercise = mock_exercise_for_submit(&mut server);
    let _submit = mock_submit(&mut server, submission_id);
    let _errors = server
        .mock("GET", grading_path(submission_id).as_str())
        .with_status(500)
        .expect_at_least(1)
        .create();

    let project = submittable_project();
    let config_dir = tempfile::tempdir().unwrap();
    let projects_dir = tempfile::tempdir().unwrap();
    let result = run_mooc_in_with_poll(
        &server,
        &[
            "submit",
            "--exercise-id",
            SUBMIT_EXERCISE_ID,
            "--submission-path",
            project.path().to_str().unwrap(),
        ],
        config_dir.path(),
        projects_dir.path(),
        5,
        40,
    );
    assert!(result.is_err(), "expected the blocking submit to fail");
}

/// Like [`run_mooc_in_expect_error`] but with an explicit grading poll
/// interval and timeout, so a test proving a fast failure isn't defeated by
/// the (real, minutes-long) default poll timeout.
fn run_mooc_in_expect_error_with_poll(
    server: &mockito::Server,
    args: &[&str],
    config_dir: &std::path::Path,
    projects_dir: &std::path::Path,
    poll_interval_ms: u64,
    poll_timeout_ms: u64,
) -> tmc_langs_cli::CliError {
    let _guard = env_lock();
    // SAFETY: all env access in these tests is serialized by ENV_LOCK.
    unsafe {
        std::env::set_var("TMC_LANGS_MOOC_ROOT_URL", server.url());
        std::env::set_var("TMC_LANGS_CONFIG_DIR", config_dir);
        std::env::set_var("TMC_LANGS_DEFAULT_PROJECTS_DIR", projects_dir);
        std::env::set_var(
            "TMC_LANGS_MOOC_POLL_INTERVAL_MS",
            poll_interval_ms.to_string(),
        );
        std::env::set_var(
            "TMC_LANGS_MOOC_POLL_TIMEOUT_MS",
            poll_timeout_ms.to_string(),
        );
    }
    let mut full = vec!["tmc-langs-cli", "mooc", "--client-name", "test"];
    full.extend_from_slice(args);
    let cli = Cli::parse_from(full);
    tmc_langs_cli::run(cli).expect_err("expected the command to fail")
}

#[test]
fn blocking_submit_transient_401_during_grading_poll_does_not_resubmit() {
    // A 401 mid-poll (not on the initial submit) must be refreshed and
    // retried in place by `call_with_refresh`: the poll resumes on the same
    // submission id, and the backend must see exactly one create-submission
    // POST, never a second one from replaying the whole `Submit` dispatch.
    let mut server = mockito::Server::new();
    let submission_id = "66666666-6666-6666-6666-666666666666";
    let _exercise = mock_exercise_for_submit(&mut server);
    let submit = mock_submit(&mut server, submission_id);

    // First poll: rejected.
    let _rejected = server
        .mock("GET", grading_path(submission_id).as_str())
        .with_status(401)
        .with_body(r#"{"message":"invalid token"}"#)
        .expect(1)
        .create();
    // The refresh the 401 triggers succeeds.
    let refresh = mock_oauth_refresh(&mut server, "refreshed-access", "refreshed-refresh");
    // The retried poll (still inside the same `call_with_refresh` call):
    // not graded yet.
    let _pending = server
        .mock("GET", grading_path(submission_id).as_str())
        .with_body(serde_json::json!("NoGradingYet").to_string())
        .expect(1)
        .create();
    // A later, ordinary poll: fully graded.
    let _graded = server
        .mock("GET", grading_path(submission_id).as_str())
        .with_body(
            serde_json::json!({
                "Grading": {
                    "grading_progress": "FullyGraded",
                    "score_given": 1.0,
                    "grading_started_at": "2026-07-21T00:00:00Z",
                    "grading_completed_at": "2026-07-21T00:00:01Z",
                    "feedback_json": null,
                    "feedback_text": "All tests passed"
                }
            })
            .to_string(),
        )
        .expect_at_least(1)
        .create();

    let config_dir = tempfile::tempdir().unwrap();
    let projects_dir = tempfile::tempdir().unwrap();
    let credentials_path = write_valid_refreshable_credentials(config_dir.path());

    let project = submittable_project();
    let output = run_mooc_in(
        &server,
        &[
            "submit",
            "--exercise-id",
            SUBMIT_EXERCISE_ID,
            "--submission-path",
            project.path().to_str().unwrap(),
        ],
        config_dir.path(),
        projects_dir.path(),
    )
    .unwrap();

    let json = submission_status_json(output);
    assert_eq!(json["Grading"]["grading_progress"], "FullyGraded");

    // Exactly one submission was ever created, despite the 401 mid-poll.
    submit.assert();
    refresh.assert();
    assert!(
        credentials_path.exists(),
        "a transient auth hiccup mid-poll must not delete valid credentials"
    );
}

#[test]
fn blocking_submit_permanent_refresh_rejection_during_grading_poll_fails_fast() {
    // A permanent refresh rejection mid-poll must abort the wait immediately
    // -- not burn through the rest of the (potentially very long) poll
    // timeout -- and must not have caused a second submission to be created.
    let mut server = mockito::Server::new();
    let submission_id = "77777777-7777-7777-7777-777777777777";
    let _exercise = mock_exercise_for_submit(&mut server);
    let submit = mock_submit(&mut server, submission_id);

    let _rejected = server
        .mock("GET", grading_path(submission_id).as_str())
        .with_status(401)
        .with_body(r#"{"message":"invalid token"}"#)
        .expect(1)
        .create();
    let refresh_rejected = server
        .mock("POST", "/api/v0/main-frontend/oauth/token")
        .with_status(400)
        .with_body(r#"{"error":"invalid_grant"}"#)
        .expect(1)
        .create();

    let config_dir = tempfile::tempdir().unwrap();
    let projects_dir = tempfile::tempdir().unwrap();
    let credentials_path = write_valid_refreshable_credentials(config_dir.path());

    let project = submittable_project();
    let start = std::time::Instant::now();
    let error = run_mooc_in_expect_error_with_poll(
        &server,
        &[
            "submit",
            "--exercise-id",
            SUBMIT_EXERCISE_ID,
            "--submission-path",
            project.path().to_str().unwrap(),
        ],
        config_dir.path(),
        projects_dir.path(),
        50,
        // A poll timeout long enough that a regression to "wait it out"
        // would make this test very obviously slow (and eventually fail
        // outright with a plain timeout error, not an invalid-token one).
        60_000,
    );
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "a permanent auth failure must fail fast, not wait out the poll timeout \
         (took {elapsed:?})"
    );
    assert_error_kind(error, "invalid-token");
    submit.assert();
    refresh_rejected.assert();
    assert!(
        !credentials_path.exists(),
        "a permanently rejected refresh must delete the credentials"
    );
}

#[test]
fn submit_dont_block_returns_submission_id_without_polling() {
    // `--dont-block` returns the submission id without polling: no grading mock is
    // mounted, so any poll would hit an unmocked route and fail.
    let mut server = mockito::Server::new();
    let submission_id = "22222222-2222-2222-2222-222222222222";
    let _exercise = mock_exercise_for_submit(&mut server);
    let _submit = mock_submit(&mut server, submission_id);

    let project = submittable_project();
    let output = run_mooc(
        &server,
        &[
            "submit",
            "--exercise-id",
            SUBMIT_EXERCISE_ID,
            "--submission-path",
            project.path().to_str().unwrap(),
            "--dont-block",
        ],
    )
    .unwrap();

    match data_of(output) {
        DataKind::MoocSubmissionFinished(result) => {
            assert_eq!(result.submission_id.to_string(), submission_id);
        }
        other => panic!("expected MoocSubmissionFinished, got {other:?}"),
    }
}

#[test]
fn dispatches_wait_for_grading() {
    // `wait-for-grading` polls grading for an existing submission, without submitting.
    let mut server = mockito::Server::new();
    let submission_id = "33333333-3333-3333-3333-333333333333";
    let _graded = server
        .mock("GET", grading_path(submission_id).as_str())
        .with_body(
            serde_json::json!({
                "Grading": {
                    "grading_progress": "Failed",
                    "score_given": 0.0,
                    "grading_started_at": null,
                    "grading_completed_at": null,
                    "feedback_json": null,
                    "feedback_text": "Compilation error"
                }
            })
            .to_string(),
        )
        .expect_at_least(1)
        .create();

    let output = run_mooc(
        &server,
        &["wait-for-grading", "--submission-id", submission_id],
    )
    .unwrap();

    let json = submission_status_json(output);
    assert_eq!(json["Grading"]["grading_progress"], "Failed");
    assert_eq!(json["Grading"]["feedback_text"], "Compilation error");
}

#[test]
fn wait_for_grading_treats_pending_manual_as_terminal() {
    // `PendingManual` is terminal-for-student: the loop stops and returns it (with
    // any partial score) rather than waiting for a human.
    let mut server = mockito::Server::new();
    let submission_id = "44444444-4444-4444-4444-444444444444";
    let _pending_manual = server
        .mock("GET", grading_path(submission_id).as_str())
        .with_body(
            serde_json::json!({
                "Grading": {
                    "grading_progress": "PendingManual",
                    "score_given": 0.5,
                    "grading_started_at": null,
                    "grading_completed_at": null,
                    "feedback_json": null,
                    "feedback_text": null
                }
            })
            .to_string(),
        )
        .expect_at_least(1)
        .create();

    let output = run_mooc(
        &server,
        &["wait-for-grading", "--submission-id", submission_id],
    )
    .unwrap();

    let json = submission_status_json(output);
    assert_eq!(json["Grading"]["grading_progress"], "PendingManual");
    assert_eq!(json["Grading"]["score_given"], 0.5);
}

#[test]
fn wait_for_grading_times_out_returning_latest_status() {
    // A never-terminal grading times out and returns the latest status as data,
    // not an error.
    let mut server = mockito::Server::new();
    let submission_id = "55555555-5555-5555-5555-555555555555";
    let _never_terminal = server
        .mock("GET", grading_path(submission_id).as_str())
        .with_body(
            serde_json::json!({
                "Grading": {
                    "grading_progress": "Pending",
                    "score_given": null,
                    "grading_started_at": null,
                    "grading_completed_at": null,
                    "feedback_json": null,
                    "feedback_text": null
                }
            })
            .to_string(),
        )
        .expect_at_least(1)
        .create();

    let config_dir = tempfile::tempdir().unwrap();
    let projects_dir = tempfile::tempdir().unwrap();
    // Short timeout (40 ms) with a 5 ms interval so the loop gives up quickly.
    let output = run_mooc_in_with_poll(
        &server,
        &["wait-for-grading", "--submission-id", submission_id],
        config_dir.path(),
        projects_dir.path(),
        5,
        40,
    )
    .unwrap();

    let json = submission_status_json(output);
    assert_eq!(json["Grading"]["grading_progress"], "Pending");
}

#[test]
fn dispatches_get_exercise_submissions() {
    // `get-exercise-submissions --exercise-id` returns the user's past
    // submissions (slide-submission ids), newest first, as `MoocSubmissions`.
    let mut server = mockito::Server::new();
    let exercise_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
    let newest = Uuid::new_v4();
    let oldest = Uuid::new_v4();
    server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/exercises/{exercise_id}/submissions")
                .as_str(),
        )
        .with_body(
            serde_json::json!([
                {
                    "id": newest, "exercise_id": exercise_id,
                    "created_at": "2026-07-21T12:00:00Z",
                    "score_given": 1.0, "grading_progress": "FullyGraded"
                },
                {
                    "id": oldest, "exercise_id": exercise_id,
                    "created_at": "2026-07-21T10:00:00Z",
                    "score_given": 0.0, "grading_progress": "Failed"
                }
            ])
            .to_string(),
        )
        .create();

    let output = run_mooc(
        &server,
        &["get-exercise-submissions", "--exercise-id", exercise_id],
    )
    .unwrap();

    match data_of(output) {
        DataKind::MoocSubmissions(subs) => {
            assert_eq!(subs.len(), 2);
            // preserved server order (newest first)
            assert_eq!(subs[0].id, newest);
            assert_eq!(subs[0].score_given, Some(1.0));
            assert_eq!(subs[1].id, oldest);
            assert_eq!(subs[1].score_given, Some(0.0));
        }
        other => panic!("expected MoocSubmissions, got {other:?}"),
    }
}

/// An exercise slide fixture with a single editor task whose stub archive lives
/// at `stub_url`, used by the old-submission download tests (which resolve the
/// stub via `download_exercise`, and the slide/task ids via `submit_exercise`).
fn editor_slide_with_stub(
    exercise_id: &str,
    slide_id: &str,
    task_id: &str,
    stub_url: &str,
) -> String {
    serde_json::json!({
        "slide_id": slide_id,
        "exercise_id": exercise_id,
        "course_id": Uuid::new_v4(),
        "exercise_name": "Old Submission Exercise",
        "exercise_order_number": 0,
        "tasks": [{
            "task_id": task_id,
            "order_number": 0,
            "assignment": [],
            "public_spec": {
                "type": "editor",
                "archive_name": "stub.tar.zst",
                "stub_download_url": stub_url,
                "student_file_paths": ["src/main.py"],
                "checksum": "abcd1234"
            },
            "model_solution_spec": null,
            "exercise_service_slug": "tmc"
        }],
    })
    .to_string()
}

#[test]
fn download_old_submission_restores_student_files_over_fresh_stub() {
    // Overlays only the submission's STUDENT files on a fresh stub: student source
    // comes from the submission, non-student files (a `test/` file) from the stub.
    // The whole output path is replaced, dropping stale local files.
    let mut server = mockito::Server::new();
    let exercise_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
    let slide_id = "e7bd5a07-1b83-4c97-91f2-e48cccf66b2a";
    let task_id = "816ac03a-a713-4804-9ea6-3eb5e278ec2b";
    let submission_id = "99999999-9999-9999-9999-999999999999";
    let stub_url = format!("{}/files/stub.tar.zst", server.url());
    let old_url = format!("{}/files/old.tar.zst", server.url());

    server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/exercises/{exercise_id}").as_str(),
        )
        .with_body(editor_slide_with_stub(
            exercise_id,
            slide_id,
            task_id,
            &stub_url,
        ))
        .expect_at_least(1)
        .create();
    // Fresh stub: requirements marker (python3), template source, real test file.
    server
        .mock("GET", "/files/stub.tar.zst")
        .with_body(make_tar_zst(&[
            ("requirements.txt", b""),
            ("src/main.py", b"# TODO: implement"),
            ("test/test_main.py", b"# real tests"),
        ]))
        .create();
    server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/submissions/{submission_id}/download")
                .as_str(),
        )
        .with_body(serde_json::json!({ "archive_download_url": old_url }).to_string())
        .create();
    // Old submission: student solution + a tampered test file that must NOT win.
    server
        .mock("GET", "/files/old.tar.zst")
        .with_body(make_tar_zst(&[
            ("requirements.txt", b""),
            ("src/main.py", b"print('my old solution')"),
            ("test/test_main.py", b"# HACKED"),
        ]))
        .create();

    let output_dir = tempfile::tempdir().unwrap();
    // Seed a "current" state that must be fully replaced.
    std::fs::create_dir_all(output_dir.path().join("src")).unwrap();
    std::fs::write(
        output_dir.path().join("src/main.py"),
        b"print('current work')",
    )
    .unwrap();
    std::fs::write(output_dir.path().join("stale.txt"), b"stale").unwrap();

    let output = run_mooc(
        &server,
        &[
            "download-old-submission",
            "--submission-id",
            submission_id,
            "--exercise-id",
            exercise_id,
            "--output-path",
            output_dir.path().to_str().unwrap(),
        ],
    )
    .unwrap();
    assert!(matches!(
        output_data(output).result,
        OutputResult::ExecutedCommand
    ));

    // student file restored from the old submission
    assert_eq!(
        std::fs::read_to_string(output_dir.path().join("src/main.py")).unwrap(),
        "print('my old solution')"
    );
    // non-student file (under test/) preserved from the fresh stub, not tampered
    assert_eq!(
        std::fs::read_to_string(output_dir.path().join("test/test_main.py")).unwrap(),
        "# real tests"
    );
    // stale current-state file is gone: the output path was fully replaced
    assert!(!output_dir.path().join("stale.txt").exists());
}

#[test]
fn download_old_submission_save_old_state_submits_first() {
    // `--save-old-state` submits the current state before overwriting, so nothing
    // the student wrote is lost; then the old submission is restored.
    let mut server = mockito::Server::new();
    let exercise_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
    let slide_id = "e7bd5a07-1b83-4c97-91f2-e48cccf66b2a";
    let task_id = "816ac03a-a713-4804-9ea6-3eb5e278ec2b";
    let submission_id = "99999999-9999-9999-9999-999999999999";
    let stub_url = format!("{}/files/stub.tar.zst", server.url());
    let old_url = format!("{}/files/old.tar.zst", server.url());

    server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/exercises/{exercise_id}").as_str(),
        )
        .with_body(editor_slide_with_stub(
            exercise_id,
            slide_id,
            task_id,
            &stub_url,
        ))
        .expect_at_least(1)
        .create();
    // The save-old-state submit must POST to the submit endpoint.
    let submit = server
        .mock(
            "POST",
            format!("/api/v0/exercise-services/client/exercises/{exercise_id}/submit").as_str(),
        )
        .with_body(serde_json::json!({ "submission_id": Uuid::new_v4() }).to_string())
        .expect_at_least(1)
        .create();
    server
        .mock("GET", "/files/stub.tar.zst")
        .with_body(make_tar_zst(&[
            ("requirements.txt", b""),
            ("src/main.py", b"# TODO: implement"),
        ]))
        .create();
    server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/submissions/{submission_id}/download")
                .as_str(),
        )
        .with_body(serde_json::json!({ "archive_download_url": old_url }).to_string())
        .create();
    server
        .mock("GET", "/files/old.tar.zst")
        .with_body(make_tar_zst(&[
            ("requirements.txt", b""),
            ("src/main.py", b"print('restored')"),
        ]))
        .create();

    // The output path must be a valid, compressible project (python3 marker) so
    // the save-old-state submit can package it.
    let output_dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(output_dir.path().join("src")).unwrap();
    std::fs::write(output_dir.path().join("requirements.txt"), b"").unwrap();
    std::fs::write(
        output_dir.path().join("src/main.py"),
        b"print('current work')",
    )
    .unwrap();

    let output = run_mooc(
        &server,
        &[
            "download-old-submission",
            "--submission-id",
            submission_id,
            "--exercise-id",
            exercise_id,
            "--output-path",
            output_dir.path().to_str().unwrap(),
            "--save-old-state",
        ],
    )
    .unwrap();
    assert!(matches!(
        output_data(output).result,
        OutputResult::ExecutedCommand
    ));

    // the current state was submitted before overwriting
    submit.assert();
    // and the old submission was restored
    assert_eq!(
        std::fs::read_to_string(output_dir.path().join("src/main.py")).unwrap(),
        "print('restored')"
    );
}

#[test]
fn reset_exercise_401_between_save_old_state_submit_and_download_does_not_resubmit() {
    // `--save-old-state` submits the current state, then downloads a fresh
    // stub to reset with. A 401 on the download step (after the submit
    // already succeeded) must be refreshed and retried in place: the submit
    // must never be repeated.
    let mut server = mockito::Server::new();
    let exercise_id = "df5ee6c1-57d1-43b6-b39e-5d72119edb5f";
    let slide_id = "e7bd5a07-1b83-4c97-91f2-e48cccf66b2a";
    let task_id = "816ac03a-a713-4804-9ea6-3eb5e278ec2b";
    let stub_url = format!("{}/files/stub.tar.zst", server.url());

    let _slide = server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/exercises/{exercise_id}").as_str(),
        )
        .with_body(editor_slide_with_stub(
            exercise_id,
            slide_id,
            task_id,
            &stub_url,
        ))
        .expect_at_least(1)
        .create();
    // The save-old-state submit: must happen exactly once.
    let submit = server
        .mock(
            "POST",
            format!("/api/v0/exercise-services/client/exercises/{exercise_id}/submit").as_str(),
        )
        .with_body(serde_json::json!({ "submission_id": Uuid::new_v4() }).to_string())
        .create();
    // The first download attempt (after the submit) is rejected...
    let _rejected = server
        .mock("GET", "/files/stub.tar.zst")
        .with_status(401)
        .with_body(r#"{"message":"invalid token"}"#)
        .expect(1)
        .create();
    // ...the refresh it triggers succeeds...
    let refresh = mock_oauth_refresh(&mut server, "refreshed-access", "refreshed-refresh");
    // ...and the retried download succeeds.
    server
        .mock("GET", "/files/stub.tar.zst")
        .with_body(make_tar_zst(&[("src/main.py", b"print('fresh stub')")]))
        .create();

    let config_dir = tempfile::tempdir().unwrap();
    let projects_dir = tempfile::tempdir().unwrap();
    let credentials_path = write_valid_refreshable_credentials(config_dir.path());

    // A valid, compressible project (python3 marker) so the save-old-state
    // submit can package it.
    let exercise_dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(exercise_dir.path().join("src")).unwrap();
    std::fs::write(exercise_dir.path().join("requirements.txt"), b"").unwrap();
    std::fs::write(
        exercise_dir.path().join("src/main.py"),
        b"print('current work')",
    )
    .unwrap();

    let output = run_mooc_in(
        &server,
        &[
            "reset-exercise",
            "--exercise-id",
            exercise_id,
            "--exercise-path",
            exercise_dir.path().to_str().unwrap(),
            "--save-old-state",
        ],
        config_dir.path(),
        projects_dir.path(),
    )
    .unwrap();

    assert!(matches!(
        output_data(output).result,
        OutputResult::ExecutedCommand
    ));
    // Exactly one submit call, despite the retried download.
    submit.assert();
    refresh.assert();
    assert_eq!(
        std::fs::read_to_string(exercise_dir.path().join("src/main.py")).unwrap(),
        "print('fresh stub')"
    );
    assert!(
        credentials_path.exists(),
        "a transient 401 recovered by a refresh must not delete credentials"
    );
}

#[test]
fn paste_submits_then_shares_the_slide_submission() {
    // `mooc paste` submits (non-blocking) then shares the submission. The submit
    // returns a TASK-submission id, but share takes a SLIDE-submission id, so paste
    // resolves the slide id from the submissions list (newest first). The share
    // endpoint is mounted ONLY at the slide id, so sharing the task id would fail.
    let mut server = mockito::Server::new();
    let task_submission_id = "aaaaaaaa-0000-0000-0000-000000000000";
    let slide_submission_id = "bbbbbbbb-1111-1111-1111-111111111111";

    let _exercise = mock_exercise_for_submit(&mut server);
    let _submit = mock_submit(&mut server, task_submission_id);
    // Submissions list, newest first: the newest item's id is the slide-submission id.
    let _submissions = server
        .mock(
            "GET",
            format!("/api/v0/exercise-services/client/exercises/{SUBMIT_EXERCISE_ID}/submissions")
                .as_str(),
        )
        .with_body(
            serde_json::json!([
                {
                    "id": slide_submission_id, "exercise_id": SUBMIT_EXERCISE_ID,
                    "created_at": "2026-07-22T12:00:00Z",
                    "score_given": null, "grading_progress": null
                }
            ])
            .to_string(),
        )
        .create();
    // Share endpoint mounted ONLY at the slide-submission id.
    let share = server
        .mock(
            "POST",
            format!("/api/v0/exercise-services/client/submissions/{slide_submission_id}/share")
                .as_str(),
        )
        .with_body(
            serde_json::json!({ "paste_url": "http://example.com/shared-submissions/tok123" })
                .to_string(),
        )
        .expect(1)
        .create();

    let project = submittable_project();
    let output = run_mooc(
        &server,
        &[
            "paste",
            "--exercise-id",
            SUBMIT_EXERCISE_ID,
            "--submission-path",
            project.path().to_str().unwrap(),
        ],
    )
    .unwrap();

    share.assert();
    match data_of(output) {
        DataKind::MoocPaste(paste) => {
            assert_eq!(
                paste.paste_url,
                "http://example.com/shared-submissions/tok123"
            );
        }
        other => panic!("expected MoocPaste, got {other:?}"),
    }
}

// --- device-flow login / logged-in / logout ---------------------------------

/// Runs a `mooc login`-family command in-process, returning the raw result so
/// tests can assert on either the success output or the error `Kind`. Sets a
/// tiny device poll interval so the login poll loop does not sleep the real
/// multi-second interval.
fn run_mooc_auth(
    server: &mockito::Server,
    args: &[&str],
    config_dir: &std::path::Path,
) -> Result<CliOutput, tmc_langs_cli::CliError> {
    let _guard = env_lock();
    // SAFETY: all env access in these tests is serialized by ENV_LOCK.
    unsafe {
        std::env::set_var("TMC_LANGS_MOOC_ROOT_URL", server.url());
        std::env::set_var("TMC_LANGS_CONFIG_DIR", config_dir);
        std::env::set_var("TMC_LANGS_MOOC_DEVICE_POLL_INTERVAL_MS", "5");
    }
    let mut full = vec!["tmc-langs-cli", "mooc", "--client-name", "test"];
    full.extend_from_slice(args);
    let cli = Cli::parse_from(full);
    tmc_langs_cli::run(cli)
}

/// Mounts the device-authorization endpoint returning a fixed device/user code.
fn mock_device_authorization(server: &mut mockito::Server) -> mockito::Mock {
    server
        .mock("POST", "/api/v0/main-frontend/oauth/device_authorization")
        .with_body(
            serde_json::json!({
                "device_code": "dev-code",
                "user_code": "WXYZ-1234",
                "verification_uri": "https://courses.mooc.fi/oauth_device",
                "verification_uri_complete":
                    "https://courses.mooc.fi/oauth_device?user_code=WXYZ-1234",
                "expires_in": 900,
                "interval": 1
            })
            .to_string(),
        )
        .expect_at_least(1)
        .create()
}

fn token_endpoint() -> &'static str {
    "/api/v0/main-frontend/oauth/token"
}

#[test]
fn mooc_login_pending_slow_down_then_success() {
    // The poll loop tolerates `authorization_pending` then `slow_down` before
    // the grant is approved, then saves the issued token.
    let mut server = mockito::Server::new();
    let _device = mock_device_authorization(&mut server);
    // Ordered token responses: pending, then slow_down, then success.
    let _pending = server
        .mock("POST", token_endpoint())
        .with_status(400)
        .with_body(r#"{"error":"authorization_pending"}"#)
        .expect(1)
        .create();
    let _slow_down = server
        .mock("POST", token_endpoint())
        .with_status(400)
        .with_body(r#"{"error":"slow_down"}"#)
        .expect(1)
        .create();
    let _authorized = server
        .mock("POST", token_endpoint())
        .with_body(
            serde_json::json!({
                "access_token": "at",
                "refresh_token": "rt",
                "token_type": "bearer",
                "expires_in": 3600
            })
            .to_string(),
        )
        .expect_at_least(1)
        .create();

    let config_dir = tempfile::tempdir().unwrap();
    let output = run_mooc_auth(&server, &["login"], config_dir.path()).unwrap();
    assert!(matches!(output_data(output).result, OutputResult::LoggedIn));

    // The issued token pair was persisted in the wrapper shape.
    let creds_path = config_dir.path().join("tmc-test/credentials_mooc.json");
    assert!(
        creds_path.exists(),
        "login should save credentials_mooc.json"
    );
    let stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&creds_path).unwrap()).unwrap();
    assert_eq!(stored["token"]["access_token"], "at");
    assert_eq!(stored["token"]["refresh_token"], "rt");
    assert!(stored.get("obtained_at").is_some());
}

#[test]
fn mooc_login_denied_maps_to_not_logged_in() {
    let mut server = mockito::Server::new();
    let _device = mock_device_authorization(&mut server);
    let _denied = server
        .mock("POST", token_endpoint())
        .with_status(400)
        .with_body(r#"{"error":"access_denied"}"#)
        .expect_at_least(1)
        .create();

    let config_dir = tempfile::tempdir().unwrap();
    let error = run_mooc_auth(&server, &["login"], config_dir.path())
        .expect_err("a denied device login should fail");
    assert_error_kind(error, "not-logged-in");
    assert!(
        !config_dir
            .path()
            .join("tmc-test/credentials_mooc.json")
            .exists(),
        "a failed login must not persist credentials"
    );
}

#[test]
fn mooc_login_expired_maps_to_not_logged_in() {
    let mut server = mockito::Server::new();
    let _device = mock_device_authorization(&mut server);
    let _expired = server
        .mock("POST", token_endpoint())
        .with_status(400)
        .with_body(r#"{"error":"expired_token"}"#)
        .expect_at_least(1)
        .create();

    let config_dir = tempfile::tempdir().unwrap();
    let error = run_mooc_auth(&server, &["login"], config_dir.path())
        .expect_err("an expired device code should fail");
    assert_error_kind(error, "not-logged-in");
}

#[test]
fn mooc_logged_in_reports_stored_credentials() {
    let server = mockito::Server::new();
    let config_dir = tempfile::tempdir().unwrap();
    write_test_credentials(config_dir.path());

    let output = run_mooc_auth(&server, &["logged-in"], config_dir.path()).unwrap();
    let data = output_data(output);
    assert!(matches!(data.result, OutputResult::LoggedIn));
    assert!(matches!(data.data, Some(DataKind::Token(_))));
}

#[test]
fn mooc_logged_in_without_credentials_reports_not_logged_in() {
    let server = mockito::Server::new();
    let config_dir = tempfile::tempdir().unwrap();

    let output = run_mooc_auth(&server, &["logged-in"], config_dir.path()).unwrap();
    let data = output_data(output);
    assert!(matches!(data.result, OutputResult::NotLoggedIn));
    assert!(data.data.is_none());
}

#[test]
fn mooc_logout_removes_credentials() {
    let server = mockito::Server::new();
    let config_dir = tempfile::tempdir().unwrap();
    let creds_path = write_test_credentials(config_dir.path());
    assert!(creds_path.exists());

    let output = run_mooc_auth(&server, &["logout"], config_dir.path()).unwrap();
    assert!(matches!(
        output_data(output).result,
        OutputResult::LoggedOut
    ));
    assert!(
        !creds_path.exists(),
        "logout should delete the credentials file"
    );
}

/// Builds a `.tar.zst` archive from (relative path, contents) pairs.
fn make_tar_zst(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar_buf = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_buf);
        for (path, contents) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, path, *contents).unwrap();
        }
        builder.finish().unwrap();
    }
    zstd::encode_all(std::io::Cursor::new(tar_buf), 0).unwrap()
}
