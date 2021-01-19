use std::env;
use std::process::{Command, Output};
use tempfile::{tempdir, NamedTempFile};

pub fn run_cmd(args: &[&str]) -> Output {
    let path = env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let path = path.parent().unwrap().join("tmc-langs-cli");
    Command::new(path).args(args).output().unwrap()
}

pub fn run_assert_success(args: &[&str]) {
    let out = run_cmd(args);
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    println!("stdout {}", stdout);
    println!("stderr {}", stderr);
    // if there's an issue with the argument parsing, stdout will be empty and not parse as json
    let last_line = stdout.lines().last().unwrap();
    let _json: serde_json::Value = serde_json::from_str(&last_line).unwrap();
    assert!(stderr.is_empty());
}

#[test]
fn sanity() {
    let out = run_cmd(&["non-existent-command"]);
    assert!(out.stdout.is_empty());
    assert!(!out.stderr.is_empty());
}

#[test]
fn checkstyle() {
    let temp = NamedTempFile::new().unwrap();
    let path = temp.path().to_str().unwrap();
    run_assert_success(&[
        "checkstyle",
        "--exercise-path",
        path,
        "--locale",
        "fi",
        "--output-path",
        path,
    ]);
}

#[test]
fn clean() {
    let temp = NamedTempFile::new().unwrap();
    run_assert_success(&["clean", "--exercise-path", temp.path().to_str().unwrap()]);
}

#[test]
fn compress_project() {
    let temp = NamedTempFile::new().unwrap();
    let path = temp.path().to_str().unwrap();
    run_assert_success(&[
        "compress-project",
        "--exercise-path",
        path,
        "--output-path",
        path,
    ]);
}

// core is in a separate file

#[test]
fn disk_space() {
    let temp = NamedTempFile::new().unwrap();
    let path = temp.path().to_str().unwrap();
    run_assert_success(&["disk-space", "--path", path]);
}

#[test]
fn extract_project() {
    let temp = NamedTempFile::new().unwrap();
    let path = temp.path().to_str().unwrap();
    run_assert_success(&[
        "extract-project",
        "--archive-path",
        path,
        "--output-path",
        path,
    ]);
}

#[test]
fn fast_available_points() {
    let temp = NamedTempFile::new().unwrap();
    let path = temp.path().to_str().unwrap();
    run_assert_success(&["fast-available-points", "--exercise-path", path]);
}

#[test]
fn find_exercises() {
    let temp = tempdir().unwrap();
    let path = temp.path().to_str().unwrap();
    run_assert_success(&[
        "find-exercises",
        "--exercise-path",
        path,
        "--output-path",
        path,
    ]);
}

#[test]
fn get_exercise_packaging_configuration() {
    let temp = NamedTempFile::new().unwrap();
    let path = temp.path().to_str().unwrap();
    run_assert_success(&[
        "get-exercise-packaging-configuration",
        "--exercise-path",
        path,
        "--output-path",
        path,
    ]);
}

#[test]
fn list_local_course_exercises() {
    run_assert_success(&[
        "list-local-course-exercises",
        "--client-name",
        "client",
        "--course-slug",
        "slug",
    ]);
}

#[test]
fn prepare_solutions() {
    let temp = NamedTempFile::new().unwrap();
    let path = temp.path().to_str().unwrap();
    run_assert_success(&[
        "prepare-solutions",
        "--exercise-path",
        path,
        "--output-path",
        path,
    ]);
}

#[test]
fn prepare_stubs() {
    let temp = NamedTempFile::new().unwrap();
    let path = temp.path().to_str().unwrap();
    run_assert_success(&[
        "prepare-stubs",
        "--exercise-path",
        path,
        "--output-path",
        path,
    ]);
}

#[test]
fn prepare_submission() {
    let temp = NamedTempFile::new().unwrap();
    let path = temp.path().to_str().unwrap();
    run_assert_success(&[
        "prepare-submission",
        "--clone-path",
        path,
        "--output-format",
        "tar",
        "--output-path",
        path,
        "--stub-zip-path",
        path,
        "--submission-path",
        path,
        "--tmc-param",
        "a=b",
        "--tmc-param",
        "c=d",
    ]);
}

#[test]
fn refresh_course() {
    let temp = NamedTempFile::new().unwrap();
    let path = temp.path().to_str().unwrap();
    run_assert_success(&[
        "refresh-course",
        "--cache-path",
        path,
        "--cache-root",
        path,
        "--chgrp-uid",
        "1234",
        "--chmod-bits",
        "1234",
        "--clone-path",
        path,
        "--course-name",
        "name",
        "--exercise",
        "name",
        path,
        "10,11,12",
        "--exercise",
        "second",
        path,
        "20,21,22",
        "--git-branch",
        "main",
        "--no-background-operations",
        "--no-directory-changes",
        "--rails-root",
        path,
        "--solution-path",
        path,
        "--solution-zip-path",
        path,
        "--source-backend",
        "git",
        "--source-url",
        "example.com",
        "--stub-path",
        path,
        "--stub-zip-path",
        path,
    ]);
}

#[test]
fn run_tests() {
    let temp = NamedTempFile::new().unwrap();
    let path = temp.path().to_str().unwrap();
    run_assert_success(&[
        "run-tests",
        "--checkstyle-output-path",
        path,
        "--exercise-path",
        path,
        "--locale",
        "fi",
        "--output-path",
        path,
    ]);
}

// settings in a separate file

#[test]
fn scan_exercise() {
    let temp = NamedTempFile::new().unwrap();
    let path = temp.path().to_str().unwrap();
    run_assert_success(&[
        "scan-exercise",
        "--exercise-path",
        path,
        "--output-path",
        path,
    ]);
}
