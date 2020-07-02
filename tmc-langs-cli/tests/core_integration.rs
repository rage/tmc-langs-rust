//! Tests for the core commands using the actual TMC API

use dotenv::dotenv;
use std::env;
use std::io::Write;
use std::process::{Command, Output, Stdio};
use tmc_langs_core::*;

fn init() {
    dotenv().ok();
    if env::var("RUST_LOG").is_err() {
        env::set_var(
            "RUST_LOG",
            "debug,hyper=warn,tokio_reactor=warn,reqwest=warn",
        );
    }
    let _ = env_logger::builder().is_test(true).try_init();
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
    let out = child.wait_with_output().unwrap();
    out
}

#[test]
#[ignore]
fn download_model_solution() {
    todo!()
}

#[test]
#[ignore]
fn download_or_update_exercises() {
    todo!()
}

#[test]
#[ignore]
fn get_course_details() {
    todo!()
}

#[test]
#[ignore]
fn get_exercise_updates() {
    todo!()
}

#[test]
#[ignore]
fn get_organizations() {
    init();
    let out = run_core_cmd(&["get-organizations"]);
    assert!(out.status.success());
    let out = String::from_utf8(out.stdout).unwrap();
    let _orgs: Vec<Organization> = serde_json::from_str(&out).unwrap();
}

#[test]
#[ignore]
fn get_unread_reviews() {
    todo!()
}

#[test]
#[ignore]
fn list_courses() {
    init();
    let out = run_core_cmd(&["list-courses", "--organization", "hy"]);
    assert!(out.status.success());
    let out = String::from_utf8(out.stdout).unwrap();
    let _courses: Vec<Course> = serde_json::from_str(&out).unwrap();
}

#[test]
#[ignore]
fn mark_review_as_read() {
    todo!()
}

#[test]
#[ignore]
fn paste_with_comment() {
    todo!()
}

#[test]
#[ignore]
fn request_code_review() {
    todo!()
}

#[test]
#[ignore]
fn run_checkstyle() {
    todo!()
}

#[test]
#[ignore]
fn run_tests() {
    todo!()
}

#[test]
#[ignore]
fn send_feedback() {
    todo!()
}

#[test]
#[ignore]
fn submit() {
    todo!()
}
