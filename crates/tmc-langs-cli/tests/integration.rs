use clap::Parser;
use std::path::Path;
use tempfile::{tempdir, NamedTempFile, TempDir};
use tmc_langs_cli::app::Cli;
use walkdir::WalkDir;

fn cp_exercise(path: &Path) -> TempDir {
    let temp = tempdir().unwrap();
    for file in WalkDir::new(path) {
        let file = file.unwrap();
        let relative = file.path().strip_prefix(path).unwrap();
        let target = temp.path().join(relative);
        if file.file_type().is_dir() {
            std::fs::create_dir_all(target).unwrap();
        } else if file.file_type().is_file() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::copy(file.path(), target).unwrap();
        }
    }
    temp
}

fn path_str(path: &impl AsRef<Path>) -> &str {
    path.as_ref().to_str().unwrap()
}

fn sorted_list_of_files(path: &impl AsRef<Path>) -> Vec<String> {
    let mut files = vec![];
    for entry in WalkDir::new(path).min_depth(1) {
        let entry = entry.unwrap();
        let path = entry.path().strip_prefix(path.as_ref()).unwrap();
        let path = path_str(&path).replace('\\', "/");
        files.push(path);
    }
    files.sort();
    files
}

// wrapper for all sample exercise tests
fn test(f: impl Fn(&Path)) {
    insta::with_settings!({
        filters => vec![
            // replace Windows-style path separators
            // the yaml serialization doubles the backslashes so
            // we need to filter \\\\
            (r"\\\\", "/"),
            // remove quotes, some keys and values get quoted on Windows due to backslash path separators
            ("\"", ""),

            // replace all /tmp/ (linux), /var/ (macos) and C:/[..]/Temp/ (win) paths which vary each test run
            // note that we already turned \s to /s
            (r"/tmp/\S*", "[PATH]"),
            (r"/var/\S*", "[PATH]"),
            (r"C:/\S*/Temp/\S*", "[PATH]"),
        ],
    }, {
        insta::glob!("sample_exercises/*/*", |exercise| {
            f(exercise)
        })
    })
}

#[test]
fn checkstyle() {
    test(|exercise| {
        let ex = cp_exercise(exercise);
        let out = NamedTempFile::new().unwrap();
        let cli = Cli::parse_from([
            "tmc-langs-cli",
            "--pretty",
            "checkstyle",
            "--exercise-path",
            path_str(&ex),
            "--locale",
            "eng",
            "--output-path",
            path_str(&out),
        ]);
        let output = tmc_langs_cli::run(cli).unwrap();
        insta::assert_yaml_snapshot!(output);
    })
}

#[test]
fn clean() {
    test(|exercise| {
        let ex = cp_exercise(exercise);
        let cli = Cli::parse_from([
            "tmc-langs-cli",
            "--pretty",
            "clean",
            "--exercise-path",
            path_str(&ex),
        ]);
        let output = tmc_langs_cli::run(cli).unwrap();
        insta::assert_yaml_snapshot!(output);
        let files = sorted_list_of_files(&ex);
        insta::assert_yaml_snapshot!(files);
    })
}
