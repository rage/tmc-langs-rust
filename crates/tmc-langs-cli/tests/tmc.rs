//! In-process dispatch tests for the `tmc` subcommand tree.
//!
//! The CLI reads its config from process-global env vars
//! (`TMC_LANGS_TMC_ROOT_URL`, `TMC_LANGS_CONFIG_DIR`,
//! `TMC_LANGS_DEFAULT_PROJECTS_DIR`), so every test that sets them runs under a
//! shared lock.

use clap::Parser;
use std::sync::{Mutex, MutexGuard};
use tmc_langs_cli::{
    app::Cli,
    output::{CliOutput, DataKind},
};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Runs a `tmc --client-name test --client-version 1.0.0 <args>` command
/// in-process against `server`. Env vars are process-global, so the shared
/// `ENV_LOCK` is held across the whole run.
fn run_tmc_in(
    server: &mockito::Server,
    args: &[&str],
    config_dir: &std::path::Path,
) -> Result<CliOutput, tmc_langs_cli::CliError> {
    let projects_dir = tempfile::tempdir().unwrap();
    let _guard = env_lock();
    // SAFETY: all env access in these tests is serialized by ENV_LOCK.
    unsafe {
        std::env::set_var("TMC_LANGS_TMC_ROOT_URL", server.url());
        std::env::set_var("TMC_LANGS_CONFIG_DIR", config_dir);
        std::env::set_var("TMC_LANGS_DEFAULT_PROJECTS_DIR", projects_dir.path());
        // A trusted loopback host would let the tmc path adopt a mooc token,
        // which these tests never seed; keep the credential source unambiguous.
        std::env::remove_var("TMC_LANGS_MOOC_TRUST_LOCALHOST");
    }
    let mut full = vec![
        "tmc-langs-cli",
        "tmc",
        "--client-name",
        "test",
        "--client-version",
        "1.0.0",
    ];
    full.extend_from_slice(args);
    let cli = Cli::parse_from(full);
    tmc_langs_cli::run(cli)
}

fn run_tmc_in_expect_error(
    server: &mockito::Server,
    args: &[&str],
    config_dir: &std::path::Path,
) -> tmc_langs_cli::CliError {
    run_tmc_in(server, args, config_dir).expect_err("expected the command to fail")
}

/// Asserts on the error `Kind` the CLI would print to stdout.
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

/// Writes a legacy tmc `credentials.json` for `--client-name test`. Nothing issues
/// these any more, but a stored one keeps working until tmc-server rejects it.
fn write_stored_tmc_credentials(config_dir: &std::path::Path) -> std::path::PathBuf {
    let dir = config_dir.join("tmc-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("credentials.json");
    std::fs::write(
        &path,
        serde_json::json!({
            "access_token": "stored-tmc-token",
            "token_type": "bearer",
            "scope": "public"
        })
        .to_string(),
    )
    .unwrap();
    path
}

fn mock_course_status(server: &mut mockito::Server, status: usize) -> mockito::Mock {
    server
        .mock("GET", "/api/v8/courses/1")
        // Every v8 request carries `client`/`client_version` query parameters.
        .match_query(mockito::Matcher::Any)
        .with_status(status)
        .with_body(
            serde_json::json!({
                "error": "Authentication required",
                "obsolete_client": false
            })
            .to_string(),
        )
        .create()
}

#[test]
fn tmc_401_deletes_the_stored_credential_and_reports_invalid_token() {
    let mut server = mockito::Server::new();
    let _course = mock_course_status(&mut server, 401);
    let config_dir = tempfile::tempdir().unwrap();
    let credentials = write_stored_tmc_credentials(config_dir.path());

    let error = run_tmc_in_expect_error(
        &server,
        &["get-course-settings", "--course-id", "1"],
        config_dir.path(),
    );

    assert_error_kind(error, "invalid-token");
    assert!(
        !credentials.exists(),
        "credentials.json should have been deleted after a 401"
    );
}

#[test]
fn tmc_403_keeps_the_stored_credential_and_reports_forbidden() {
    let mut server = mockito::Server::new();
    let _course = mock_course_status(&mut server, 403);
    let config_dir = tempfile::tempdir().unwrap();
    let credentials = write_stored_tmc_credentials(config_dir.path());

    let error = run_tmc_in_expect_error(
        &server,
        &["get-course-settings", "--course-id", "1"],
        config_dir.path(),
    );

    assert_error_kind(error, "forbidden");
    assert!(
        credentials.exists(),
        "only a 401 may delete credentials.json"
    );
}

#[test]
fn tmc_404_for_an_exercise_reports_exercise_not_found() {
    let mut server = mockito::Server::new();
    let _exercise = server
        .mock("GET", "/api/v8/core/exercises/1")
        .match_query(mockito::Matcher::Any)
        .with_status(404)
        .with_body(r#"{"errors":["Couldn't find Exercise"]}"#)
        .create();
    let config_dir = tempfile::tempdir().unwrap();
    write_stored_tmc_credentials(config_dir.path());

    let error = run_tmc_in_expect_error(
        &server,
        &["get-exercise-details", "--exercise-id", "1"],
        config_dir.path(),
    );

    assert_error_kind(error, "exercise-not-found");
}

#[test]
fn tmc_404_for_a_course_reports_not_found() {
    let mut server = mockito::Server::new();
    let _course = mock_course_status(&mut server, 404);
    let config_dir = tempfile::tempdir().unwrap();
    write_stored_tmc_credentials(config_dir.path());

    let error = run_tmc_in_expect_error(
        &server,
        &["get-course-settings", "--course-id", "1"],
        config_dir.path(),
    );

    assert_error_kind(error, "not-found");
}

#[test]
fn tmc_without_any_credential_reports_not_logged_in() {
    let server = mockito::Server::new();
    let config_dir = tempfile::tempdir().unwrap();

    let error = run_tmc_in_expect_error(
        &server,
        &["get-course-settings", "--course-id", "1"],
        config_dir.path(),
    );

    assert_error_kind(error, "not-logged-in");
}

/// A projects dir as tmc-langs 0.39 left it must stay listed, and its config intact.
#[test]
fn list_local_tmc_course_exercises_reads_the_legacy_layout() {
    let config_dir = tempfile::tempdir().unwrap();
    let projects_dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(config_dir.path().join("tmc-test")).unwrap();
    std::fs::write(
        config_dir.path().join("tmc-test/config.toml"),
        format!(
            "projects-dir = '{}'\n",
            projects_dir
                .path()
                .display()
                .to_string()
                .replace('\\', "\\\\")
        ),
    )
    .unwrap();
    let course_config = projects_dir.path().join("course1/course_config.toml");
    std::fs::create_dir_all(projects_dir.path().join("course1/ex1")).unwrap();
    std::fs::write(
        &course_config,
        "course = 'course1'\n[exercises.ex1]\nid = 1\nchecksum = 'abc'\n",
    )
    .unwrap();

    let output = {
        let _guard = env_lock();
        // SAFETY: all env access in these tests is serialized by ENV_LOCK.
        unsafe {
            std::env::set_var("TMC_LANGS_CONFIG_DIR", config_dir.path());
        }
        let cli = Cli::parse_from([
            "tmc-langs-cli",
            "list-local-tmc-course-exercises",
            "--client-name",
            "test",
            "--course-slug",
            "course1",
        ]);
        tmc_langs_cli::run(cli).unwrap()
    };

    let CliOutput::OutputData(data) = output else {
        panic!("expected OutputData, got {output:?}");
    };
    let Some(DataKind::LocalTmcExercises(exercises)) = data.data else {
        panic!("expected LocalTmcExercises, got {:?}", data.data);
    };
    assert_eq!(exercises.len(), 1);
    assert_eq!(exercises[0].exercise_slug, "ex1");
    assert_eq!(
        exercises[0].exercise_path,
        projects_dir.path().join("course1/ex1")
    );
    assert!(
        std::fs::read_to_string(course_config)
            .unwrap()
            .contains("ex1")
    );
}

#[test]
fn logout_needs_no_network() {
    let server = mockito::Server::new();
    let config_dir = tempfile::tempdir().unwrap();
    let credentials = write_stored_tmc_credentials(config_dir.path());
    let _guard = env_lock();
    // SAFETY: all env access in these tests is serialized by ENV_LOCK.
    unsafe {
        std::env::set_var("TMC_LANGS_MOOC_ROOT_URL", "http://127.0.0.1:1");
    }
    drop(_guard);

    run_tmc_in(&server, &["logout"], config_dir.path()).unwrap();
    assert!(!credentials.exists());
}
