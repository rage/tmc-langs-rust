use clap::Parser;
use std::path::Path;
use tempfile::{tempdir, NamedTempFile, TempDir};
use tmc_langs::Compression;
use tmc_langs_cli::app::Cli;
use walkdir::WalkDir;

fn cp_exercise(path: &Path) -> TempDir {
    let path_parent = path.parent().unwrap();
    let temp = tempdir().unwrap();
    for file in WalkDir::new(path) {
        let file = file.unwrap();
        let relative = file.path().strip_prefix(path_parent).unwrap();
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

fn compress_naive(path: &impl AsRef<Path>, target: &impl AsRef<Path>, compression: Compression) {
    let cli = Cli::parse_from([
        "tmc-langs-cli",
        "--pretty",
        "compress-project",
        "--exercise-path",
        path_str(path),
        "--output-path",
        path_str(target),
        "--compression",
        &compression.to_string(),
        "--naive",
    ]);
    tmc_langs_cli::run(cli).unwrap();
}

fn extract_naive(path: &impl AsRef<Path>, target: &impl AsRef<Path>, compression: Compression) {
    let cli = Cli::parse_from([
        "tmc-langs-cli",
        "--pretty",
        "extract-project",
        "--archive-path",
        path_str(path),
        "--output-path",
        path_str(target),
        "--compression",
        &compression.to_string(),
        "--naive",
    ]);
    tmc_langs_cli::run(cli).unwrap();
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
    let _ = env_logger::try_init();
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
        insta::glob!("../../../", "sample_exercises/*/*", |exercise| {
            let dir_name = exercise.file_name().unwrap();
            let exercise = cp_exercise(&exercise);
            println!("testing {:?}", exercise.path().join(dir_name));
            f(&exercise.path().join(dir_name))
        })
    })
}

#[test]
fn checkstyle() {
    test(|exercise| {
        let out = NamedTempFile::new().unwrap();
        let cli = Cli::parse_from([
            "tmc-langs-cli",
            "--pretty",
            "checkstyle",
            "--exercise-path",
            path_str(&exercise),
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
        let cli = Cli::parse_from([
            "tmc-langs-cli",
            "--pretty",
            "clean",
            "--exercise-path",
            path_str(&exercise),
        ]);
        let output = tmc_langs_cli::run(cli).unwrap();
        insta::assert_yaml_snapshot!(output);
        let files = sorted_list_of_files(&exercise);
        insta::assert_yaml_snapshot!(files);
    })
}

#[test]
fn compress_project_tar() {
    test(|exercise| {
        let target = NamedTempFile::new().unwrap();
        let cli = Cli::parse_from([
            "tmc-langs-cli",
            "--pretty",
            "compress-project",
            "--exercise-path",
            path_str(&exercise),
            "--output-path",
            path_str(&target),
            "--compression",
            "tar",
        ]);
        let output = tmc_langs_cli::run(cli).unwrap();
        insta::assert_yaml_snapshot!(output);

        let extracted = tempdir().unwrap();
        extract_naive(&target, &extracted, Compression::Tar);
        let files = sorted_list_of_files(&extracted);
        insta::assert_yaml_snapshot!(files);
    })
}

#[test]
fn compress_project_zip() {
    test(|exercise| {
        let target = NamedTempFile::new().unwrap();
        let cli = Cli::parse_from([
            "tmc-langs-cli",
            "--pretty",
            "compress-project",
            "--exercise-path",
            path_str(&exercise),
            "--output-path",
            path_str(&target),
            // zip should be the default
            // "--compression",
            // "zip",
        ]);
        let output = tmc_langs_cli::run(cli).unwrap();
        insta::assert_yaml_snapshot!(output);

        let extracted = tempdir().unwrap();
        extract_naive(&target, &extracted, Compression::Zip);
        let files = sorted_list_of_files(&extracted);
        insta::assert_yaml_snapshot!(files);
    })
}

#[test]
fn compress_project_zstd() {
    test(|exercise| {
        let target = NamedTempFile::new().unwrap();
        let cli = Cli::parse_from([
            "tmc-langs-cli",
            "--pretty",
            "compress-project",
            "--exercise-path",
            path_str(&exercise),
            "--output-path",
            path_str(&target),
            "--compression",
            "zstd",
        ]);
        let output = tmc_langs_cli::run(cli).unwrap();
        insta::assert_yaml_snapshot!(output);

        let extracted = tempdir().unwrap();
        extract_naive(&target, &extracted, Compression::TarZstd);
        let files = sorted_list_of_files(&extracted);
        insta::assert_yaml_snapshot!(files);
    })
}

#[test]
fn extract_project_tar() {
    test(|exercise| {
        let compressed = NamedTempFile::new().unwrap();
        compress_naive(&exercise, &compressed, Compression::Tar);
        let target = tempdir().unwrap();
        let cli = Cli::parse_from([
            "tmc-langs-cli",
            "--pretty",
            "extract-project",
            "--archive-path",
            path_str(&compressed),
            "--output-path",
            path_str(&target),
            "--compression",
            "tar",
        ]);
        let output = tmc_langs_cli::run(cli).unwrap();
        insta::assert_yaml_snapshot!(output);
        let files = sorted_list_of_files(&target);
        insta::assert_yaml_snapshot!(files);
    })
}

#[test]
fn extract_project_zip() {
    test(|exercise| {
        let compressed = NamedTempFile::new().unwrap();
        compress_naive(&exercise, &compressed, Compression::Zip);
        let target = tempdir().unwrap();
        let cli = Cli::parse_from([
            "tmc-langs-cli",
            "--pretty",
            "extract-project",
            "--archive-path",
            path_str(&compressed),
            "--output-path",
            path_str(&target),
            // zip should be the default
            // "--compression",
            // "zip",
        ]);
        let output = tmc_langs_cli::run(cli).unwrap();
        insta::assert_yaml_snapshot!(output);
        let files = sorted_list_of_files(&target);
        insta::assert_yaml_snapshot!(files);
    })
}

#[test]
fn extract_project_zstd() {
    test(|exercise| {
        let compressed = NamedTempFile::new().unwrap();
        compress_naive(&exercise, &compressed, Compression::TarZstd);
        let target = tempdir().unwrap();
        let cli = Cli::parse_from([
            "tmc-langs-cli",
            "--pretty",
            "extract-project",
            "--archive-path",
            path_str(&compressed),
            "--output-path",
            path_str(&target),
            "--compression",
            "zstd",
        ]);
        let output = tmc_langs_cli::run(cli).unwrap();
        insta::assert_yaml_snapshot!(output);
        let files = sorted_list_of_files(&target);
        insta::assert_yaml_snapshot!(files);
    })
}
