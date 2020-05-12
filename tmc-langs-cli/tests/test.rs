use std::env;
use std::process::{Command, Output};
use tempdir::TempDir;
use walkdir::WalkDir;

fn run_cmd(args: &[&str]) -> Output {
    let path = env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let path = path.parent().unwrap().join("tmc-langs-cli");
    Command::new(path).args(args).output().unwrap()
}

fn test_dir(dir: &str) -> String {
    format!("tests/data/{}", dir)
}

#[test]
fn compress_project() {
    let temp = TempDir::new("compress-project").unwrap();
    let out = run_cmd(&[
        "compress-project",
        "--exercisePath",
        &test_dir("project"),
        "--outputPath",
        temp.path().to_str().unwrap(),
    ]);
    println!("out: {}", String::from_utf8(out.stdout).unwrap());
    println!("err: {}", String::from_utf8(out.stderr).unwrap());
}
