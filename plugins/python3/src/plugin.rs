//! Contains the Python3Plugin struct

use crate::error::PythonError;
use crate::policy::Python3StudentFilePolicy;
use crate::{LocalPy, PythonTestResult, LOCAL_PY};
use std::collections::HashMap;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tmc_langs_framework::{
    command::{Output, TmcCommand},
    domain::{ExerciseDesc, RunResult, RunStatus, TestDesc, TestResult, TmcProjectYml},
    error::CommandError,
    io::file_util,
    nom::{branch, bytes, character, combinator, sequence, IResult},
    plugin::LanguagePlugin,
    TmcError,
};
use walkdir::WalkDir;

pub struct Python3Plugin {}

impl Python3Plugin {
    pub const fn new() -> Self {
        Self {}
    }
}

impl LanguagePlugin for Python3Plugin {
    const PLUGIN_NAME: &'static str = "python3";
    const LINE_COMMENT: &'static str = "#";
    const BLOCK_COMMENT: Option<(&'static str, &'static str)> = Some(("\"\"\"", "\"\"\""));
    type StudentFilePolicy = Python3StudentFilePolicy;

    fn get_student_file_policy(project_path: &Path) -> Self::StudentFilePolicy {
        Python3StudentFilePolicy::new(project_path.to_owned())
    }

    fn scan_exercise(
        &self,
        exercise_directory: &Path,
        exercise_name: String,
    ) -> Result<ExerciseDesc, TmcError> {
        let available_points_json = exercise_directory.join(".available_points.json");
        // remove any existing points json
        if available_points_json.exists() {
            file_util::remove_file(&available_points_json)?;
        }

        let run_result = run_tmc_command(exercise_directory, &["available_points"], None);
        if let Err(error) = run_result {
            log::error!("Failed to scan exercise. {}", error);
        }

        let test_descs_res = parse_exercise_description(&available_points_json);
        // remove file regardless of parse success
        if available_points_json.exists() {
            file_util::remove_file(&available_points_json)?;
        }
        Ok(ExerciseDesc::new(exercise_name, test_descs_res?))
    }

    fn run_tests_with_timeout(
        &self,
        exercise_directory: &Path,
        timeout: Option<Duration>,
    ) -> Result<RunResult, TmcError> {
        let test_results_json = exercise_directory.join(".tmc_test_results.json");
        // remove any existing results json
        if test_results_json.exists() {
            file_util::remove_file(&test_results_json)?;
        }

        let output = run_tmc_command(exercise_directory, &[], timeout);

        match output {
            Ok(output) => {
                let mut logs = HashMap::new();
                logs.insert(
                    "stdout".to_string(),
                    String::from_utf8_lossy(&output.stdout).into_owned(),
                );
                logs.insert(
                    "stderr".to_string(),
                    String::from_utf8_lossy(&output.stderr).into_owned(),
                );
                let parse_res = parse_test_result(&test_results_json, logs);
                // remove file regardless of parse success
                if test_results_json.exists() {
                    file_util::remove_file(&test_results_json)?;
                }
                Ok(parse_res?)
            }
            Err(PythonError::Tmc(TmcError::Command(CommandError::TimeOut {
                stdout,
                stderr,
                ..
            }))) => {
                let mut logs = HashMap::new();
                logs.insert("stdout".to_string(), stdout);
                logs.insert("stderr".to_string(), stderr);
                Ok(RunResult {
                    status: RunStatus::TestsFailed,
                    test_results: vec![TestResult {
                        name: "Timeout test".to_string(),
                        successful: false,
                        points: vec![],
                        message:
                            "Tests timed out.\nMake sure you don't have an infinite loop in your code."
                                .to_string(),
                        exception: vec![],
                    }],
                    logs,
                })
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Checks if the directory has one of setup.py, requirements.txt., test/__init__.py, or tmc/__main__.py
    fn is_exercise_type_correct(path: &Path) -> bool {
        let setup = path.join("setup.py");
        let requirements = path.join("requirements.txt");
        let test = path.join("test").join("__init__.py");
        let tmc = path.join("tmc").join("__main__.py");

        setup.exists() || requirements.exists() || test.exists() || tmc.exists()
    }

    fn clean(&self, exercise_path: &Path) -> Result<(), TmcError> {
        for entry in WalkDir::new(exercise_path)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_name() == ".available_points.json"
                || entry.file_name() == ".tmc_test_results.json"
                || entry.file_name() == "__pycache__"
            {
                if entry.path().is_file() {
                    file_util::remove_file(entry.path())?;
                } else {
                    file_util::remove_dir_all(entry.path())?;
                }
            }
        }
        Ok(())
    }

    fn get_default_student_file_paths(&self) -> Vec<PathBuf> {
        vec![PathBuf::from("src")]
    }

    fn get_default_exercise_file_paths(&self) -> Vec<PathBuf> {
        vec![PathBuf::from("test"), PathBuf::from("tmc")]
    }

    fn points_parser<'a>(i: &'a str) -> IResult<&'a str, &'a str> {
        combinator::map(
            sequence::delimited(
                sequence::tuple((
                    bytes::complete::tag("@"),
                    character::complete::multispace0,
                    bytes::complete::tag_no_case("points"),
                    character::complete::multispace0,
                    character::complete::char('('),
                    character::complete::multispace0,
                )),
                branch::alt((
                    sequence::delimited(
                        character::complete::char('"'),
                        bytes::complete::is_not("\""),
                        character::complete::char('"'),
                    ),
                    sequence::delimited(
                        character::complete::char('\''),
                        bytes::complete::is_not("'"),
                        character::complete::char('\''),
                    ),
                )),
                sequence::tuple((
                    character::complete::multispace0,
                    character::complete::char(')'),
                )),
            ),
            str::trim,
        )(i)
    }
}

fn get_local_python_command() -> Result<TmcCommand, PythonError> {
    let command = match &*LOCAL_PY {
        LocalPy::Unix => TmcCommand::new_with_file_io("python3")?,
        LocalPy::Windows => TmcCommand::new_with_file_io("py")?.with(|e| e.arg("-3")),
        LocalPy::WindowsConda { conda_path } => TmcCommand::new_with_file_io(conda_path)?,
        LocalPy::Custom { python_exec } => TmcCommand::new_with_file_io(python_exec)?,
    };
    Ok(command)
}

fn get_local_python_ver() -> Result<(usize, usize, usize), PythonError> {
    let output = get_local_python_command()?
        .with(|e| e.args(&["-c", "import sys; print(sys.version_info.major); print(sys.version_info.minor); print(sys.version_info.micro);"]))
        .output_checked()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();
    let major: usize = lines
        .next()
        .ok_or_else(|| PythonError::VersionPrintError(stdout.clone().into_owned()))?
        .trim()
        .parse()
        .map_err(|e| PythonError::VersionParseError(stdout.clone().into_owned(), e))?;
    let minor: usize = lines
        .next()
        .ok_or_else(|| PythonError::VersionPrintError(stdout.clone().into_owned()))?
        .trim()
        .parse()
        .map_err(|e| PythonError::VersionParseError(stdout.clone().into_owned(), e))?;
    let patch: usize = lines
        .next()
        .ok_or_else(|| PythonError::VersionPrintError(stdout.clone().into_owned()))?
        .trim()
        .parse()
        .map_err(|e| PythonError::VersionParseError(stdout.clone().into_owned(), e))?;

    Ok((major, minor, patch))
}

fn run_tmc_command(
    path: &Path,
    extra_args: &[&str],
    timeout: Option<Duration>,
) -> Result<Output, PythonError> {
    let minimum_python_version = TmcProjectYml::from(path)?
        .minimum_python_version
        .unwrap_or_default();
    // default minimum version is 3.0.0
    let minimum_major = minimum_python_version.major.unwrap_or(3);
    let minimum_minor = minimum_python_version.minor.unwrap_or(0);
    let minimum_patch = minimum_python_version.patch.unwrap_or(0);

    let (major, minor, patch) = get_local_python_ver()?;

    let found_ver = format!("{}.{}.{}", major, minor, patch);
    let minimum_ver = format!("{}.{}.{}", minimum_major, minimum_minor, minimum_patch);

    if major < minimum_major
        || major == minimum_major && minor < minimum_minor
        || major == minimum_major && minor == minimum_minor && patch < minimum_patch
    {
        return Err(PythonError::OldPythonVersion {
            found: found_ver,
            minimum_required: minimum_ver,
        });
    }

    let path =
        dunce::canonicalize(path).map_err(|e| PythonError::Canonicalize(path.to_path_buf(), e))?;
    log::debug!("running tmc command at {}", path.display());
    let common_args = ["-m", "tmc"];

    let command = get_local_python_command()?;
    let command = command.with(|e| e.args(&common_args).args(extra_args).cwd(path));
    let output = if let Some(timeout) = timeout {
        command.output_with_timeout(timeout)?
    } else {
        command.output()?
    };

    log::trace!("stdout: {}", String::from_utf8_lossy(&output.stdout));
    log::debug!("stderr: {}", String::from_utf8_lossy(&output.stderr));
    Ok(output)
}

/// Parse exercise description file
fn parse_exercise_description(available_points_json: &Path) -> Result<Vec<TestDesc>, PythonError> {
    let mut test_descs = vec![];
    let file = file_util::open_file(&available_points_json)?;
    // TODO: deserialize directly into Vec<TestDesc>?
    let json: HashMap<String, Vec<String>> = serde_json::from_reader(BufReader::new(file))
        .map_err(|e| PythonError::Deserialize(available_points_json.to_path_buf(), e))?;
    for (key, value) in json {
        test_descs.push(TestDesc::new(key, value));
    }
    Ok(test_descs)
}

/// Parse test result file
fn parse_test_result(
    test_results_json: &Path,
    logs: HashMap<String, String>,
) -> Result<RunResult, PythonError> {
    let results_file = file_util::open_file(&test_results_json)?;
    let test_results: Vec<PythonTestResult> = serde_json::from_reader(BufReader::new(results_file))
        .map_err(|e| PythonError::Deserialize(test_results_json.to_path_buf(), e))?;
    let test_results: Vec<TestResult> = test_results
        .into_iter()
        .map(PythonTestResult::into_test_result)
        .collect();

    let mut status = RunStatus::Passed;
    for result in &test_results {
        if !result.successful {
            status = RunStatus::TestsFailed;
        }
    }
    Ok(RunResult::new(status, test_results, logs))
}

#[cfg(test)]
mod test {
    use super::*;
    use std::fs::{self, File};
    use std::path::{Path, PathBuf};
    use tempfile::{tempdir, TempDir};
    use tmc_langs_framework::zip::ZipArchive;
    use tmc_langs_framework::{domain::RunStatus, plugin::LanguagePlugin};

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    // copies the target exercise and tmc to a temp directory
    fn copy_test(dir: &str) -> TempDir {
        let path = Path::new(dir);
        let temp = tempdir().unwrap();
        for entry in walkdir::WalkDir::new(path) {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                let entry_path: PathBuf = entry
                    .path()
                    .components()
                    .skip(path.components().count())
                    .collect();
                let temp_path = temp.path().join(entry_path);
                temp_path.parent().map(|p| std::fs::create_dir_all(&p)); // ignore result, errors on windows
                log::trace!(
                    "copying {} -> {}",
                    entry.path().display(),
                    temp_path.display()
                );
                std::fs::copy(entry.path(), temp_path).unwrap();
            }
        }
        let _ = fs::remove_file(temp.path().join("tmc")); // delete symlink
        for entry in walkdir::WalkDir::new("tests/data/tmc") {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                let entry_path: PathBuf = entry.path().components().skip(2).collect();
                let temp_path = temp.path().join(entry_path);
                temp_path.parent().map(|p| std::fs::create_dir_all(&p)); // ignore result, errors on windows
                log::trace!(
                    "copying {} -> {}",
                    entry.path().display(),
                    temp_path.display()
                );
                std::fs::copy(entry.path(), temp_path).unwrap();
            }
        }
        temp
    }

    #[test]
    fn scans_exercise() {
        init();

        let temp = copy_test("tests/data/project");
        let plugin = Python3Plugin::new();
        let ex_desc = plugin.scan_exercise(temp.path(), "name".into()).unwrap();
        assert_eq!(ex_desc.name, "name");
        assert_eq!(
            &ex_desc.tests[0].name,
            "test.test_points.TestEverything.test_new"
        );
        assert!(ex_desc.tests[0].points.contains(&"1.1".into()));
        assert!(ex_desc.tests[0].points.contains(&"1.2".into()));
        assert!(ex_desc.tests[0].points.contains(&"2.2".into()));
        assert_eq!(ex_desc.tests[0].points.len(), 3);
    }

    #[test]
    fn runs_tests() {
        init();
        let plugin = Python3Plugin::new();

        let temp = copy_test("tests/data/project");
        let run_result = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(run_result.status, RunStatus::Passed);
        assert_eq!(run_result.test_results[0].name, "TestEverything: test_new");
        assert!(run_result.test_results[0].successful);
        assert!(run_result.test_results[0].points.contains(&"1.1".into()));
        assert!(run_result.test_results[0].points.contains(&"1.2".into()));
        assert!(run_result.test_results[0].points.contains(&"2.2".into()));
        assert_eq!(run_result.test_results[0].points.len(), 3);
        assert!(run_result.test_results[0].message.is_empty());
        assert!(run_result.test_results[0].exception.is_empty());
        assert_eq!(run_result.test_results.len(), 1);
        // assert!(run_result.logs.is_empty());

        let temp = copy_test("tests/data/failing");
        let run_result = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(run_result.status, RunStatus::TestsFailed);
        assert_eq!(run_result.test_results[0].name, "TestFailing: test_new");
        assert!(!run_result.test_results[0].successful);
        assert!(run_result.test_results[0].points.contains(&"1.1".into()));
        assert!(run_result.test_results[0].points.contains(&"1.2".into()));
        assert!(run_result.test_results[0].points.contains(&"2.2".into()));
        assert!(run_result.test_results[0].message.starts_with("'a' != 'b'"));
        assert!(!run_result.test_results[0].exception.is_empty());
        assert_eq!(run_result.test_results.len(), 1);
        // assert!(run_result.logs.is_empty());

        let temp = copy_test("tests/data/erroring");
        let run_result = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(run_result.status, RunStatus::TestsFailed);
        assert_eq!(
            run_result.test_results[0].name,
            "TestErroring: test_erroring"
        );
        assert!(!run_result.test_results[0].successful);
        assert!(run_result.test_results[0].points.contains(&"1.1".into()));
        assert!(run_result.test_results[0].points.contains(&"1.2".into()));
        assert!(run_result.test_results[0].points.contains(&"2.2".into()));
        assert_eq!(
            run_result.test_results[0].message,
            "name 'doSomethingIllegal' is not defined"
        );
        assert!(!run_result.test_results[0].exception.is_empty());
        assert_eq!(run_result.test_results.len(), 1);
        // assert!(run_result.logs.is_empty());
    }

    #[test]
    fn exercise_type_is_correct() {
        init();
        let _plugin = Python3Plugin::new();

        let correct = Python3Plugin::is_exercise_type_correct(Path::new("tests/data"));
        assert!(correct);

        let correct = Python3Plugin::is_exercise_type_correct(Path::new("./"));
        assert!(!correct);
    }

    #[test]
    fn clean() {
        init();
        let plugin = Python3Plugin::new();

        let temp = copy_test("tests/data/clean_target");
        let temp_path = temp.path();

        assert!(temp_path.join(".available_points.json").exists());
        assert!(temp_path
            .join("subdirectory/.tmc_test_results.json")
            .exists());
        assert!(temp_path
            .join("subdirectory/__pycache__/cachefile")
            .exists());
        plugin.clean(temp.path()).unwrap();
        assert!(!temp_path.join(".available_points.json").exists());
        assert!(!temp_path
            .join("subdirectory/.tmc_test_results.json")
            .exists());
        assert!(!temp_path
            .join("subdirectory/__pycache__/cachefile")
            .exists());
        assert!(temp_path.join("leave").exists());
    }

    #[test]
    fn timeout() {
        init();
        let plugin = Python3Plugin::new();

        let temp = copy_test("tests/data/timeout");
        let timeout = plugin
            .run_tests_with_timeout(temp.path(), Some(std::time::Duration::from_millis(1)))
            .unwrap();
        assert_eq!(timeout.test_results[0].name, "Timeout test");
    }

    #[test]
    fn finds_project_dir_in_zip() {
        init();
        let file = File::open("tests/data/PythonProject.zip").unwrap();
        let mut zip = ZipArchive::new(file).unwrap();
        let dir = Python3Plugin::find_project_dir_in_zip(&mut zip).unwrap();
        assert_eq!(dir, Path::new("Outer/Inner/project"));
    }

    #[test]
    fn doesnt_find_project_dir_in_zip() {
        init();
        let file = File::open("tests/data/PythonWithoutSrc.zip").unwrap();
        let mut zip = ZipArchive::new(file).unwrap();
        let dir = Python3Plugin::find_project_dir_in_zip(&mut zip);
        assert!(dir.is_err());
    }

    #[test]
    fn extracts_project() {
        init();
        let plugin = Python3Plugin::new();
        let archive = Path::new("tests/data/student_exercise.zip");
        let temp = tempfile::tempdir().unwrap();
        assert!(!temp.path().join("src/source.py").exists());
        plugin.extract_project(archive, temp.path(), false).unwrap();
        assert!(temp.path().join("src/source.py").exists());
        assert!(temp.path().join("test/test.py").exists());
        assert!(temp.path().join("tmc/tmc").exists());
    }

    #[test]
    fn extracts_project_over_existing() {
        init();
        let plugin = Python3Plugin::new();
        let archive = Path::new("tests/data/student_exercise.zip");
        let temp = copy_test("tests/data/student_exercise");
        assert_eq!(
            fs::read_to_string(temp.path().join("src/source.py")).unwrap(),
            "NEW"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("test/test.py")).unwrap(),
            "NEW"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("tmc/tmc")).unwrap(),
            "NEW"
        );
        plugin.extract_project(archive, temp.path(), false).unwrap();
        assert_eq!(
            fs::read_to_string(temp.path().join("src/source.py")).unwrap(),
            "NEW"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("test/test.py")).unwrap(),
            "OLD"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("tmc/tmc")).unwrap(),
            "OLD"
        );
        assert!(temp.path().join("src/new.py").exists());
        assert!(temp.path().join("test/new.py").exists());
        assert!(temp.path().join("tmc/new").exists());
    }

    #[test]
    fn extracts_project_over_existing_clean() {
        init();
        let plugin = Python3Plugin::new();
        let archive = Path::new("tests/data/student_exercise.zip");
        let temp = copy_test("tests/data/student_exercise");
        assert_eq!(
            fs::read_to_string(temp.path().join("src/source.py")).unwrap(),
            "NEW"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("test/test.py")).unwrap(),
            "NEW"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("tmc/tmc")).unwrap(),
            "NEW"
        );
        plugin.extract_project(archive, temp.path(), true).unwrap();
        assert_eq!(
            fs::read_to_string(temp.path().join("src/source.py")).unwrap(),
            "NEW"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("test/test.py")).unwrap(),
            "OLD"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("tmc/tmc")).unwrap(),
            "OLD"
        );
        assert!(temp.path().join("src/new.py").exists());
        assert!(!temp.path().join("test/new.py").exists());
        assert!(!temp.path().join("tmc/new").exists());
    }

    #[test]
    fn parse_python_ver() {
        let (_major, _minor, _patch) = get_local_python_ver().unwrap();
    }
}
