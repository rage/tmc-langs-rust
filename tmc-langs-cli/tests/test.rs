use std::env;
use std::process::{Command, Output};
use tempfile::tempdir;


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
    let temp = tempdir().unwrap();
    let out = run_cmd(&[
        "compress-project",
        "--exercisePath",
        &test_dir("project"),
        "--outputPath",
        temp.path().join("zip.zip").to_str().unwrap(),
    ]);
    println!("out:\n{}", String::from_utf8(out.stdout).unwrap());
    println!("err:\n{}", String::from_utf8(out.stderr).unwrap());
    // TODO
}
