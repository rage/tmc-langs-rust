//! Tests for the core commands using the actual TMC API

use dotenv::dotenv;
use std::env;
use std::io::Write;
use std::process::{Command, Output, Stdio};
use tmc_client::*;

fn init() {
    dotenv().ok();
    use log::*;
    use simple_logger::*;
    let _ = SimpleLogger::new()
        .with_level(LevelFilter::Debug)
        .with_module_level("hyper", LevelFilter::Warn)
        .with_module_level("tokio_reactor", LevelFilter::Warn)
        .with_module_level("reqwest", LevelFilter::Warn)
        .init();
}

fn run_core_cmd(args: &[&str]) -> Output {
    let email = env::var("EMAIL").unwrap();
    let password = env::var("PASSWORD").unwrap();

    let path = env!("CARGO_BIN_EXE_tmc-langs-cli");
    let mut child = Command::new(path)
        .stdout(Stdio::piped())
        .stdin(Stdio::piped())
        .args(&["core", "--email", &email])
        .args(args)
        .spawn()
        .unwrap();
    let child_stdin = child.stdin.as_mut().unwrap();
    let password_write = format!("{}\n", password);
    child_stdin.write_all(password_write.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
#[ignore]
fn downloads_model_solution() {
    todo!()
}

#[test]
#[ignore]
fn downloads_or_updates_exercises() {
    todo!()
}

#[test]
#[ignore]
fn gets_course_details() {
    todo!()
}

#[test]
#[ignore]
fn gets_exercise_updates() {
    todo!()
}

#[test]
#[ignore]
fn gets_organizations() {
    init();
    let out = run_core_cmd(&["get-organizations"]);
    assert!(out.status.success());
    let out = String::from_utf8(out.stdout).unwrap();
    let _orgs: Vec<Organization> = serde_json::from_str(&out).unwrap();
}

#[test]
#[ignore]
fn gets_unread_reviews() {
    todo!()
}

#[test]
#[ignore]
fn lists_courses() {
    init();
    let out = run_core_cmd(&["list-courses", "--organization", "hy"]);
    assert!(out.status.success());
    let out = String::from_utf8(out.stdout).unwrap();
    let _courses: Vec<Course> = serde_json::from_str(&out).unwrap();
}

#[test]
#[ignore]
fn marks_review_as_read() {
    todo!()
}

#[test]
#[ignore]
fn pastes_with_comment() {
    todo!()
}

#[test]
#[ignore]
fn requests_code_review() {
    todo!()
}

#[test]
#[ignore]
fn runs_checkstyle() {
    todo!()
}

#[test]
#[ignore]
fn runs_tests() {
    todo!()
}

#[test]
#[ignore]
fn sends_feedback() {
    todo!()
}

#[test]
#[ignore]
fn submits() {
    todo!()
}
