//! Contains the Python3Plugin struct

use crate::{
    error::PythonError, policy::Python3StudentFilePolicy, python_test_result::PythonTestResult,
};
use hmac::{Hmac, Mac};
use once_cell::sync::Lazy;
use rand::Rng;
use sha2::Sha256;
use std::{
    collections::{HashMap, HashSet},
    env,
    ffi::OsStr,
    io::{BufReader, Read, Seek},
    ops::ControlFlow::{Break, Continue},
    path::{Component, Path, PathBuf},
    time::Duration,
};
use tmc_langs_framework::{
    Archive, CommandError, ExerciseDesc, LanguagePlugin, Output, PythonVer, RunResult, RunStatus,
    TestDesc, TestResult, TmcCommand, TmcError, TmcProjectYml,
    nom::{IResult, Parser, bytes, character, combinator, sequence},
    nom_language::error::VerboseError,
};
use tmc_langs_util::{
    deserialize, file_util,
    notification_reporter::{self, Notification},
    parse_util, path_util,
};
use walkdir::WalkDir;

pub struct Python3Plugin;

impl Python3Plugin {
    pub const fn new() -> Self {
        Self
    }

    fn get_local_python_command() -> TmcCommand {
        // the correct python command is platform-dependent
        static LOCAL_PY: Lazy<LocalPy> = Lazy::new(|| {
            if let Ok(python_exec) = env::var("TMC_LANGS_PYTHON_EXEC") {
                log::debug!(
                    "using Python from environment variable TMC_LANGS_PYTHON_EXEC={python_exec}"
                );
                return LocalPy::Custom { python_exec };
            }

            if cfg!(windows) {
                // Check for Conda
                let conda = env::var("CONDA_PYTHON_EXE");
                if let Ok(conda_path) = conda {
                    if PathBuf::from(&conda_path).exists() {
                        log::debug!("detected conda on windows");
                        return LocalPy::WindowsConda { conda_path };
                    }
                }
                log::debug!("detected windows");
                LocalPy::Windows
            } else {
                log::debug!("detected unix");
                LocalPy::Unix
            }
        });

        enum LocalPy {
            Unix,
            Windows,
            WindowsConda { conda_path: String },
            Custom { python_exec: String },
        }

        match &*LOCAL_PY {
            LocalPy::Unix => TmcCommand::piped("python3"),
            LocalPy::Windows => TmcCommand::piped("py").with(|e| e.arg("-3")),
            LocalPy::WindowsConda { conda_path } => TmcCommand::piped(conda_path),
            LocalPy::Custom { python_exec } => TmcCommand::piped(python_exec),
        }
    }

    fn get_local_python_ver() -> Result<(u32, u32, u32), PythonError> {
        let output = Self::get_local_python_command()
        .with(|e| e.args(&["-c", "import sys; print(sys.version_info.major); print(sys.version_info.minor); print(sys.version_info.micro);"]))
        .output_checked()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut lines = stdout.lines();
        let major: u32 = lines
            .next()
            .ok_or_else(|| PythonError::VersionPrintError(stdout.clone().into_owned()))?
            .trim()
            .parse()
            .map_err(|e| PythonError::VersionParseError(stdout.clone().into_owned(), e))?;
        let minor: u32 = lines
            .next()
            .ok_or_else(|| PythonError::VersionPrintError(stdout.clone().into_owned()))?
            .trim()
            .parse()
            .map_err(|e| PythonError::VersionParseError(stdout.clone().into_owned(), e))?;
        let patch: u32 = lines
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
        stdin: Option<String>,
    ) -> Result<Output, PythonError> {
        let minimum = TmcProjectYml::load_or_default(path)?
            .minimum_python_version
            .unwrap_or_default()
            .min();
        let recommended = PythonVer::recommended();
        let local = Self::get_local_python_ver()?;

        if local < recommended {
            notification_reporter::notify(Notification::warning(format!(
                "Your Python is out of date. Minimum maintained release is {}.{}.{},\
                your Python version was detected as {}.{}.{}. Updating to a newer release is recommended.",
                minimum.0, minimum.1, minimum.2, local.0, local.1, local.2
            )));
        }
        if local < minimum {
            return Err(PythonError::OldPythonVersion {
                found: format!("{}.{}.{}", local.0, local.1, local.2),
                minimum_required: format!("{}.{}.{}", minimum.0, minimum.1, minimum.2),
            });
        }

        let path = dunce::canonicalize(path)
            .map_err(|e| PythonError::Canonicalize(path.to_path_buf(), e))?;
        log::debug!("running tmc command at {}", path.display());
        let common_args = ["-m", "tmc"];

        let command = Self::get_local_python_command();
        let command = command.with(|e| e.args(&common_args).args(extra_args).cwd(path));
        let command = if let Some(stdin) = stdin {
            command.set_stdin_data(stdin)
        } else {
            command
        };

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
    fn parse_exercise_description(
        available_points_json: &Path,
    ) -> Result<Vec<TestDesc>, PythonError> {
        let mut test_descs = vec![];
        let file = file_util::open_file(available_points_json)?;
        // TODO: deserialize directly into Vec<TestDesc>?
        let json: HashMap<String, Vec<String>> =
            deserialize::json_from_reader(BufReader::new(file))
                .map_err(|e| PythonError::Deserialize(available_points_json.to_path_buf(), e))?;
        for (key, value) in json {
            test_descs.push(TestDesc::new(key, value));
        }
        Ok(test_descs)
    }

    /// Parse test result file
    fn parse_and_verify_test_result(
        test_results_json: &Path,
        hmac_data: Option<(String, String)>,
    ) -> Result<(RunStatus, Vec<TestResult>), PythonError> {
        let results = file_util::read_file_to_string(test_results_json)?;

        // verify test results
        if let Some((hmac_secret, test_runner_hmac_hex)) = hmac_data {
            let mut mac = Hmac::<Sha256>::new_from_slice(hmac_secret.as_bytes())
                .expect("HMAC can take key of any size");
            mac.update(results.as_bytes());
            let bytes =
                hex::decode(test_runner_hmac_hex).map_err(|_| PythonError::UnexpectedHmac)?;
            mac.verify_slice(&bytes)
                .map_err(|_| PythonError::InvalidHmac)?;
        }

        let test_results: Vec<PythonTestResult> = deserialize::json_from_str(&results)
            .map_err(|e| PythonError::Deserialize(test_results_json.to_path_buf(), e))?;

        let mut status = RunStatus::Passed;
        let mut failed_points = HashSet::new();
        for result in &test_results {
            if !result.passed {
                status = RunStatus::TestsFailed;
                failed_points.extend(result.points.iter().cloned());
            }
        }

        let test_results: Vec<TestResult> = test_results
            .into_iter()
            .map(|r| r.into_test_result(&failed_points))
            .collect();
        Ok((status, test_results))
    }
}

impl Default for Python3Plugin {
    fn default() -> Self {
        Self::new()
    }
}

/// Project directory:
/// Contains setup.py, requirements.txt, test/__init__.py, or tmc/__main__.py
/// OR
/// Contains an .ipynb file. This is given lower priority than the prior rule, and if there are multiple .ipynb files, the shallowest directory is returned.
impl LanguagePlugin for Python3Plugin {
    const PLUGIN_NAME: &'static str = "python3";
    const DEFAULT_SANDBOX_IMAGE: &'static str = "eu.gcr.io/moocfi-public/tmc-sandbox-python:latest";
    const LINE_COMMENT: &'static str = "#";
    const BLOCK_COMMENT: Option<(&'static str, &'static str)> = Some(("\"\"\"", "\"\"\""));
    type StudentFilePolicy = Python3StudentFilePolicy;

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

        if let Err(error) =
            Self::run_tmc_command(exercise_directory, &["available_points"], None, None)
        {
            log::error!("Failed to scan exercise. {error}");
        }

        let test_descs_res = Self::parse_exercise_description(&available_points_json);
        // remove file regardless of parse success
        if available_points_json.exists() {
            file_util::remove_file(&available_points_json)?;
        }
        let tests = test_descs_res?;
        Ok(ExerciseDesc::new(exercise_name, tests))
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

        let (output, random_string) = if exercise_directory.join("tmc/hmac_writer.py").exists() {
            // has hmac writer
            let random_string: String = rand::rng()
                .sample_iter(rand::distr::Alphanumeric)
                .take(32)
                .map(char::from)
                .collect();
            let output = Self::run_tmc_command(
                exercise_directory,
                &["--wait-for-secret"],
                timeout,
                Some(random_string.clone()),
            );
            (output, Some(random_string))
        } else {
            let output = Self::run_tmc_command(exercise_directory, &[], timeout, None);
            (output, None)
        };

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                log::trace!("stdout: {stdout}");
                log::debug!("stderr: {stderr}");

                // TODO: is it OK to not check output.status.success()?

                let hmac_data = if let Some(random_string) = random_string {
                    let hmac_result_path = exercise_directory.join(".tmc_test_results.hmac.sha256");
                    let test_runner_hmac = file_util::read_file_to_string(hmac_result_path)?;
                    Some((random_string, test_runner_hmac))
                } else {
                    None
                };

                if !test_results_json.exists() {
                    return Err(PythonError::MissingTestResults {
                        path: test_results_json,
                        stdout: stdout.into_owned(),
                        stderr: stderr.into_owned(),
                    }
                    .into());
                }
                let (status, mut test_results) =
                    Self::parse_and_verify_test_result(&test_results_json, hmac_data)?;
                file_util::remove_file(&test_results_json)?;

                // remove points associated with any failed tests
                let mut failed_points = HashSet::new();
                for test_result in &test_results {
                    if !test_result.successful {
                        failed_points.extend(test_result.points.iter().cloned());
                    }
                }
                for test_result in &mut test_results {
                    test_result.points.retain(|p| !failed_points.contains(p));
                }

                let mut logs = HashMap::new();
                logs.insert("stdout".to_string(), stdout.into_owned());
                logs.insert("stderr".to_string(), stderr.into_owned());
                Ok(RunResult {
                    status,
                    test_results,
                    logs,
                })
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

    fn find_project_dir_in_archive<R: Read + Seek>(
        archive: &mut Archive<R>,
    ) -> Result<PathBuf, TmcError> {
        let mut iter = archive.iter()?;
        let mut shallowest_ipynb_path: Option<PathBuf> = None;

        let project_dir = loop {
            let next = iter.with_next(|file| {
                // archives don't necessarily contain entries for intermediate directories
                let file_path = file.path()?;

                // skip pycache paths
                if file_path
                    .components()
                    .map(Component::as_os_str)
                    .flat_map(OsStr::to_str)
                    .any(|c| c == "__pycache__")
                {
                    return Ok(Continue(()));
                }

                if file.is_file() {
                    // for all .py files...
                    if file_path
                        .extension()
                        .and_then(OsStr::to_str)
                        .map(|ext| ext == "py")
                        .unwrap_or_default()
                    {
                        // check if the parent is src and return src's parent dir if so
                        if let Some(parent) = file_path.parent() {
                            if let Some(parent) = path_util::get_parent_of_named(parent, "src") {
                                return Ok(Break(Some(parent)));
                            }
                        }
                    }
                    if let Some(parent) = path_util::get_parent_of_named(&file_path, "setup.py") {
                        return Ok(Break(Some(parent)));
                    }
                    if let Some(parent) =
                        path_util::get_parent_of_named(&file_path, "requirements.txt")
                    {
                        return Ok(Break(Some(parent)));
                    }
                    if let Some(parent) = path_util::get_parent_of_named(&file_path, "__init__.py")
                    {
                        if let Some(parent) = path_util::get_parent_of_named(&parent, "test") {
                            return Ok(Break(Some(parent)));
                        }
                    }
                    if let Some(parent) = path_util::get_parent_of_named(&file_path, "__main__.py")
                    {
                        if let Some(parent) = path_util::get_parent_of_named(&parent, "tmc") {
                            return Ok(Break(Some(parent)));
                        }
                    }
                    // check for .ipynb file, ignore __MACOSX
                    if file_path.extension() == Some(OsStr::new("ipynb"))
                        && !file_path.components().any(|c| c.as_os_str() == "__MACOSX")
                    {
                        if let Some(ipynb_path) = shallowest_ipynb_path.as_mut() {
                            // make sure we maintain the shallowest ipynb path in the archive
                            if ipynb_path.components().count() > file_path.components().count() {
                                *ipynb_path = file_path
                                    .parent()
                                    .map(PathBuf::from)
                                    .unwrap_or_else(|| PathBuf::from(""));
                            }
                        } else {
                            shallowest_ipynb_path = Some(
                                file_path
                                    .parent()
                                    .map(PathBuf::from)
                                    .unwrap_or_else(|| PathBuf::from("")),
                            );
                        }
                    }
                }
                Ok(Continue(()))
            });
            match next? {
                Continue(_) => continue,
                Break(project_dir) => break project_dir,
            }
        };

        match project_dir {
            Some(project_dir) => Ok(project_dir),
            None => {
                // no src found, use shallowest ipynb path if any
                if let Some(ipynb_path) = shallowest_ipynb_path {
                    Ok(ipynb_path)
                } else {
                    Err(TmcError::NoProjectDirInArchive)
                }
            }
        }
    }

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

    fn get_default_student_file_paths() -> Vec<PathBuf> {
        vec![PathBuf::from("src")]
    }

    fn get_default_exercise_file_paths() -> Vec<PathBuf> {
        vec![PathBuf::from("test"), PathBuf::from("tmc")]
    }

    fn points_parser(i: &str) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
        combinator::map(
            sequence::delimited(
                (
                    character::complete::char('@'),
                    character::complete::multispace0,
                    bytes::complete::tag_no_case("points"),
                    character::complete::multispace0,
                    character::complete::char('('),
                    character::complete::multispace0,
                ),
                parse_util::comma_separated_strings_either,
                (
                    character::complete::multispace0,
                    character::complete::char(')'),
                ),
            ),
            // splits each point by whitespace
            |points| {
                points
                    .into_iter()
                    .flat_map(|p| p.split_whitespace())
                    .collect()
            },
        )
        .parse(i)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use std::{
        io::Write,
        path::{Path, PathBuf},
    };
    use tmc_langs_framework::{LanguagePlugin, RunStatus};
    use zip::write::SimpleFileOptions;

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new().with_level(LevelFilter::Debug).init();
    }

    fn file_to(
        target_dir: impl AsRef<std::path::Path>,
        target_relative: impl AsRef<std::path::Path>,
        contents: impl AsRef<[u8]>,
    ) -> PathBuf {
        let target = target_dir.as_ref().join(target_relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&target, contents.as_ref()).unwrap();
        target
    }

    fn dir_to_zip(source_dir: impl AsRef<std::path::Path>) -> Vec<u8> {
        let mut target = vec![];
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut target));

        for entry in walkdir::WalkDir::new(&source_dir)
            .min_depth(1)
            .sort_by(|a, b| a.path().cmp(b.path()))
        {
            let entry = entry.unwrap();
            let rela = entry
                .path()
                .strip_prefix(&source_dir)
                .unwrap()
                .to_str()
                .unwrap();
            if entry.path().is_dir() {
                zip.add_directory(rela, SimpleFileOptions::default())
                    .unwrap();
            } else if entry.path().is_file() {
                zip.start_file(rela, SimpleFileOptions::default()).unwrap();
                let bytes = std::fs::read(entry.path()).unwrap();
                zip.write_all(&bytes).unwrap();
            }
        }

        zip.finish().unwrap();
        target
    }

    fn temp_with_tmc() -> tempfile::TempDir {
        let temp = tempfile::TempDir::new().unwrap();
        for entry in walkdir::WalkDir::new("tests/data/tmc") {
            let entry = entry.unwrap();
            let rela = entry.path().strip_prefix("tests/data").unwrap();
            let target = temp.path().join(rela);
            if entry.path().is_dir() {
                std::fs::create_dir(target).unwrap();
            } else if entry.path().is_file() {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
        temp
    }

    #[test]
    fn gets_local_python_command() {
        init();

        let _cmd = Python3Plugin::get_local_python_command();
    }

    #[test]
    fn gets_local_python_ver() {
        init();

        let (_major, _minor, _patch) = Python3Plugin::get_local_python_ver().unwrap();
    }

    #[test]
    fn parses_test_result() {
        init();
    }

    #[test]
    fn scans_exercise() {
        init();

        let temp_dir = temp_with_tmc();
        file_to(&temp_dir, "test/__init__.py", "");
        file_to(
            &temp_dir,
            "test/test_file.py",
            r#"
import unittest
from tmc import points

@points('1.1')
class TestClass(unittest.TestCase):
    @points('1.2', '2.2')
    def test_func(self):
        pass
"#,
        );

        let plugin = Python3Plugin::new();
        let ex_desc = plugin.scan_exercise(temp_dir.path(), "ex".into()).unwrap();
        assert_eq!(ex_desc.name, "ex");
        assert_eq!(&ex_desc.tests[0].name, "test.test_file.TestClass.test_func");
        assert!(ex_desc.tests[0].points.contains(&"1.1".into()));
        assert!(ex_desc.tests[0].points.contains(&"1.2".into()));
        assert!(ex_desc.tests[0].points.contains(&"2.2".into()));
        assert_eq!(ex_desc.tests[0].points.len(), 3);
    }

    #[test]
    fn runs_tests_successful() {
        init();

        let temp_dir = temp_with_tmc();
        file_to(&temp_dir, "test/__init__.py", "");
        file_to(
            &temp_dir,
            "test/test_file.py",
            r#"
import unittest
from tmc import points

@points('1.1')
class TestPassing(unittest.TestCase):
    def test_func(self):
        self.assertEqual("a", "a")
"#,
        );

        let plugin = Python3Plugin::new();
        let run_result = plugin.run_tests(temp_dir.path()).unwrap();
        assert_eq!(run_result.status, RunStatus::Passed);
        assert_eq!(run_result.test_results[0].name, "TestPassing: test_func");
        assert!(run_result.test_results[0].successful);
        assert!(run_result.test_results[0].points.contains(&"1.1".into()));
        assert_eq!(run_result.test_results[0].points.len(), 1);
        assert!(run_result.test_results[0].message.is_empty());
        assert!(run_result.test_results[0].exception.is_empty());
        assert_eq!(run_result.test_results.len(), 1);
    }

    #[test]
    fn runs_tests_failure() {
        init();

        let temp_dir = temp_with_tmc();
        file_to(&temp_dir, "test/__init__.py", "");
        file_to(
            &temp_dir,
            "test/test_file.py",
            r#"
import unittest
from tmc import points

@points('1.1')
class TestFailing(unittest.TestCase):
    def test_func(self):
        self.assertEqual("a", "b")
"#,
        );

        let plugin = Python3Plugin::new();
        let run_result = plugin.run_tests(temp_dir.path()).unwrap();
        log::debug!("{run_result:#?}");
        assert_eq!(run_result.status, RunStatus::TestsFailed);
        assert_eq!(run_result.test_results[0].name, "TestFailing: test_func");
        assert!(!run_result.test_results[0].successful);
        assert!(run_result.test_results[0].message.starts_with("'a' != 'b'"));
        assert!(!run_result.test_results[0].exception.is_empty());
        assert_eq!(run_result.test_results.len(), 1);
    }

    #[test]
    fn runs_tests_erroring() {
        init();

        let temp_dir = temp_with_tmc();
        file_to(&temp_dir, "test/__init__.py", "");
        file_to(
            &temp_dir,
            "test/test_file.py",
            r#"
import unittest
from tmc import points

@points('1.1')
class TestErroring(unittest.TestCase):
    def test_func(self):
        doSomethingIllegal()
"#,
        );

        let plugin = Python3Plugin::new();
        let run_result = plugin.run_tests(temp_dir.path()).unwrap();
        log::debug!("{run_result:#?}");
        assert_eq!(run_result.status, RunStatus::TestsFailed);
        assert_eq!(run_result.test_results[0].name, "TestErroring: test_func");
        assert!(!run_result.test_results[0].successful);
        assert_eq!(
            run_result.test_results[0].message,
            "name 'doSomethingIllegal' is not defined"
        );
        assert!(!run_result.test_results[0].exception.is_empty());
        assert_eq!(run_result.test_results.len(), 1);
    }

    #[test]
    fn runs_tests_timeout() {
        init();

        let temp_dir = temp_with_tmc();
        file_to(&temp_dir, "test/__init__.py", "");
        file_to(
            &temp_dir,
            "test/test_file.py",
            r#"
import unittest

class TestErroring(unittest.TestCase):
    pass
"#,
        );

        let plugin = Python3Plugin::new();
        let run_result = plugin
            .run_tests_with_timeout(temp_dir.path(), Some(std::time::Duration::from_nanos(1)))
            .unwrap();
        assert_eq!(run_result.status, RunStatus::TestsFailed);
        assert_eq!(run_result.test_results[0].name, "Timeout test");
        assert!(
            run_result.test_results[0]
                .message
                .starts_with("Tests timed out.")
        );
    }

    #[test]
    fn exercise_type_is_correct() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(&temp_dir, "setup.py", "");
        assert!(Python3Plugin::is_exercise_type_correct(temp_dir.path()));

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(&temp_dir, "requirements.txt", "");
        assert!(Python3Plugin::is_exercise_type_correct(temp_dir.path()));

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(&temp_dir, "test/__init__.py", "");
        assert!(Python3Plugin::is_exercise_type_correct(temp_dir.path()));

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(&temp_dir, "tmc/__main__.py", "");
        assert!(Python3Plugin::is_exercise_type_correct(temp_dir.path()));
    }

    #[test]
    fn exercise_type_is_not_correct() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(&temp_dir, "setup", "");
        file_to(&temp_dir, "requirements.tt", "");
        file_to(&temp_dir, "dir/setup.py", "");
        file_to(&temp_dir, "dir/requirements.txt", "");
        file_to(&temp_dir, "dir/test/__init__.py", "");
        file_to(&temp_dir, "tmc/main.py", "");
        assert!(!Python3Plugin::is_exercise_type_correct(temp_dir.path()));
    }

    #[test]
    fn cleans() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        let f1 = file_to(&temp_dir, ".available_points.json", "");
        let f2 = file_to(&temp_dir, "dir/.tmc_test_results.json", "");
        let f3 = file_to(&temp_dir, "__pycache__/cachefile", "");
        let f4 = file_to(&temp_dir, "leave", "");

        assert!(f1.exists());
        assert!(f2.exists());
        assert!(f3.exists());
        assert!(f4.exists());

        Python3Plugin::new().clean(temp_dir.path()).unwrap();

        assert!(!f1.exists());
        assert!(!f2.exists());
        assert!(!f3.exists());
        assert!(f4.exists());
    }

    #[test]
    fn parses_points() {
        assert_eq!(
            Python3Plugin::points_parser("@points('p1')").unwrap().1,
            &["p1"]
        );
        assert_eq!(
            Python3Plugin::points_parser("@  pOiNtS  (  '  p2  '  )  ")
                .unwrap()
                .1,
            &["p2"]
        );
        assert_eq!(
            Python3Plugin::points_parser(r#"@points("p3")"#).unwrap().1,
            &["p3"]
        );
        assert_eq!(
            Python3Plugin::points_parser(r#"@points("p3", 'p4', "p5", "p6 p7")"#)
                .unwrap()
                .1,
            &["p3", "p4", "p5", "p6", "p7"]
        );
        assert!(Python3Plugin::points_parser(r#"@points("p3')"#).is_err());
    }

    #[test]
    fn finds_project_dir_in_zip() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(&temp_dir, "Outer/Inner/project/setup.py", "");

        let bytes = dir_to_zip(&temp_dir);
        let mut zip = Archive::zip(std::io::Cursor::new(bytes)).unwrap();
        let dir = Python3Plugin::find_project_dir_in_archive(&mut zip).unwrap();
        assert_eq!(dir, Path::new("Outer/Inner/project"));
    }

    #[test]
    fn doesnt_find_project_dir_in_zip() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(&temp_dir, "Outer/Inner/project/srcb/main.py", "");

        let bytes = dir_to_zip(&temp_dir);
        let mut zip = Archive::zip(std::io::Cursor::new(bytes)).unwrap();
        let res = Python3Plugin::find_project_dir_in_archive(&mut zip);
        assert!(res.is_err());
    }

    #[test]
    fn finds_project_dir_from_ipynb() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(&temp_dir, "inner/file.ipynb", "");
        file_to(&temp_dir, "file.ipynb", "");

        let bytes = dir_to_zip(&temp_dir);
        let mut zip = Archive::zip(std::io::Cursor::new(bytes)).unwrap();
        let dir = Python3Plugin::find_project_dir_in_archive(&mut zip).unwrap();
        assert_eq!(dir, Path::new(""));

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(&temp_dir, "dir/inner/file.ipynb", "");
        file_to(&temp_dir, "dir/file.ipynb", "");

        let bytes = dir_to_zip(&temp_dir);
        let mut zip = Archive::zip(std::io::Cursor::new(bytes)).unwrap();
        let dir = Python3Plugin::find_project_dir_in_archive(&mut zip).unwrap();
        assert_eq!(dir, Path::new("dir"));
    }

    #[test]
    fn doesnt_give_points_unless_all_relevant_exercises_pass() {
        init();

        let temp_dir = temp_with_tmc();
        file_to(&temp_dir, "test/__init__.py", "");
        file_to(
            &temp_dir,
            "test/test_file.py",
            r#"
import unittest
from tmc import points

@points('1')
class TestClass(unittest.TestCase):
    @points('1.1', '1.2')
    def test_func1(self):
        self.assertTrue(False)

    @points('1.1', '1.3')
    def test_func2(self):
        self.assertTrue(True)
"#,
        );

        let plugin = Python3Plugin::new();
        let results = plugin.run_tests(temp_dir.path()).unwrap();
        assert_eq!(results.status, RunStatus::TestsFailed);
        let mut got_point = false;
        for test in results.test_results {
            got_point = got_point || test.points.contains(&"1.3".to_string());
            assert!(!test.points.contains(&"1".to_string()));
            assert!(!test.points.contains(&"1.1".to_string()));
            assert!(!test.points.contains(&"1.2".to_string()));
        }
        assert!(got_point);
    }

    #[test]
    fn verifies_test_results_success() {
        init();

        let mut temp = tempfile::NamedTempFile::new().unwrap();
        temp.write_all(br#"[{"name": "test.test_hello_world.HelloWorld.test_first", "status": "passed", "message": "", "passed": true, "points": ["p01-01.1"], "backtrace": []}]"#).unwrap();

        let hmac_secret = "047QzQx8RAYLR3lf0UfB75WX5EFnx7AV".to_string();
        let test_runner_hmac =
            "b379817c66cc7b1610d03ac263f02fa11f7b0153e6aeff3262ecc0598bf0be21".to_string();
        Python3Plugin::parse_and_verify_test_result(
            temp.path(),
            Some((hmac_secret, test_runner_hmac)),
        )
        .unwrap();
    }

    #[test]
    fn verifies_test_results_failure() {
        init();

        let mut temp = tempfile::NamedTempFile::new().unwrap();
        temp.write_all(br#"[{"name": "test.test_hello_world.HelloWorld.test_first", "status": "passed", "message": "", "passed": true, "points": ["p01-01.1"], "backtrace": []}]"#).unwrap();

        let hmac_secret = "047QzQx8RAYLR3lf0UfB75WX5EFnx7AV".to_string();
        let test_runner_hmac =
            "b379817c66cc7b1610d03ac263f02fa11f7b0153e6aeff3262ecc0598bf0be22".to_string();
        let res = Python3Plugin::parse_and_verify_test_result(
            temp.path(),
            Some((hmac_secret, test_runner_hmac)),
        );
        assert!(res.is_err());
    }
}
