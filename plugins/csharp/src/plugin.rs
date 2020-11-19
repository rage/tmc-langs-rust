//! Implementation of LanguagePlugin for C#

mod policy;

pub use self::policy::CSharpStudentFilePolicy;

use crate::{CSTestResult, CSharpError};
use std::collections::HashMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{BufReader, Cursor, Read, Seek};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tmc_langs_framework::{
    anyhow,
    command::TmcCommand,
    domain::{
        ExerciseDesc, RunResult, RunStatus, Strategy, TestDesc, TestResult, ValidationResult,
    },
    error::{CommandError, FileIo},
    io::file_util,
    nom::{bytes, character, combinator, sequence, IResult},
    plugin::Language,
    zip::ZipArchive,
    LanguagePlugin, TmcError,
};
use walkdir::WalkDir;

#[derive(Default)]
pub struct CSharpPlugin {}

impl CSharpPlugin {
    pub fn new() -> Self {
        Self {}
    }

    /// Extracts the bundled tmc-csharp-runner to the given path.
    fn extract_runner(target: &Path) -> Result<(), CSharpError> {
        log::debug!("extracting C# runner to {}", target.display());
        const TMC_CSHARP_RUNNER: &[u8] = include_bytes!("../tmc-csharp-runner-1.0.1.zip");

        let mut zip = ZipArchive::new(Cursor::new(TMC_CSHARP_RUNNER))?;
        for i in 0..zip.len() {
            let file = zip.by_index(i)?;
            if file.is_file() {
                let target_file_path = target.join(Path::new(file.name()));
                if let Some(parent) = target_file_path.parent() {
                    file_util::create_dir_all(&parent)?;
                }

                let file_path = PathBuf::from(file.name());
                let bytes: Vec<u8> = file
                    .bytes()
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| FileIo::FileRead(file_path, e))?;
                file_util::write_to_file(&mut bytes.as_slice(), target_file_path)?;
            }
        }
        Ok(())
    }

    /// Returns the directory of the TMC C# runner, writing it to the cache dir if it doesn't exist there yet.
    fn get_runner_dir() -> Result<PathBuf, CSharpError> {
        match dirs::cache_dir() {
            Some(cache_dir) => {
                let runner_dir = cache_dir.join("tmc").join("tmc-csharp-runner");
                if !runner_dir.exists() {
                    Self::extract_runner(&runner_dir)?;
                }
                Ok(runner_dir)
            }
            None => Err(CSharpError::CacheDir),
        }
    }

    /// Returns the path to the TMC C# runner in the cache. If TMC_CSHARP_BOOTSTRAP_PATH is set, it is returned instead.
    fn get_bootstrap_path() -> Result<PathBuf, CSharpError> {
        if let Ok(var) = env::var("TMC_CSHARP_BOOTSTRAP_PATH") {
            log::debug!("using bootstrap path TMC_CSHARP_BOOTSTRAP_PATH={}", var);
            Ok(PathBuf::from(var))
        } else {
            let runner_path = Self::get_runner_dir()?;
            let bootstrap_path = runner_path.join("TestMyCode.CSharp.Bootstrap.dll");
            if bootstrap_path.exists() {
                log::debug!("found boostrap dll at {}", bootstrap_path.display());
                Ok(bootstrap_path)
            } else {
                Err(CSharpError::MissingBootstrapDll(bootstrap_path))
            }
        }
    }

    /// Parses the test results JSON file at the path argument.
    fn parse_test_results(
        test_results_path: &Path,
        logs: HashMap<String, String>,
    ) -> Result<RunResult, CSharpError> {
        let test_results = file_util::open_file(test_results_path)?;
        let test_results: Vec<CSTestResult> = serde_json::from_reader(test_results)
            .map_err(|e| CSharpError::ParseTestResults(test_results_path.to_path_buf(), e))?;

        let mut status = RunStatus::Passed;
        for test_result in &test_results {
            if !test_result.passed {
                status = RunStatus::TestsFailed;
                break;
            }
        }
        // convert the parsed C# test results into TMC test results
        let test_results = test_results.into_iter().map(|t| t.into()).collect();
        Ok(RunResult {
            status,
            test_results,
            logs,
        })
    }
}

impl LanguagePlugin for CSharpPlugin {
    const PLUGIN_NAME: &'static str = "csharp";
    const LINE_COMMENT: &'static str = "//";
    const BLOCK_COMMENT: Option<(&'static str, &'static str)> = Some(("/*", "*/"));
    type StudentFilePolicy = CSharpStudentFilePolicy;

    /// Checks the directories in src for csproj files, up to 2 subdirectories deep.
    fn is_exercise_type_correct(path: &Path) -> bool {
        WalkDir::new(path.join("src"))
            .max_depth(2)
            .into_iter()
            .filter_map(|e| e.ok())
            .any(|e| e.path().extension() == Some(&OsString::from("csproj")))
    }

    /// Finds any directory X which contains a X/src/*.csproj file.
    fn find_project_dir_in_zip<R: Read + Seek>(
        zip_archive: &mut ZipArchive<R>,
    ) -> Result<PathBuf, TmcError> {
        for i in 0..zip_archive.len() {
            let file = zip_archive.by_index(i)?;
            let file_path = Path::new(file.name());
            if file_path.extension() == Some(OsStr::new("csproj")) {
                if let Some(csproj_parent) = file_path.parent().and_then(Path::parent) {
                    if csproj_parent.file_name() == Some(OsStr::new("src")) {
                        if let Some(src_parent) = csproj_parent.parent() {
                            return Ok(src_parent.to_path_buf());
                        }
                    }
                }
            }
        }
        Err(TmcError::NoProjectDirInZip)
    }

    fn get_student_file_policy(project_path: &Path) -> Self::StudentFilePolicy {
        Self::StudentFilePolicy::new(project_path.to_path_buf())
    }

    /// Runs --generate-points-file and parses the generated .tmc_available_points.json.
    fn scan_exercise(
        &self,
        path: &Path,
        exercise_name: String,
        _warnings: &mut Vec<anyhow::Error>,
    ) -> Result<ExerciseDesc, TmcError> {
        let exercise_desc_json_path = path.join(".tmc_available_points.json");
        if exercise_desc_json_path.exists() {
            file_util::remove_file(&exercise_desc_json_path)?;
        }

        let bootstrap_path = Self::get_bootstrap_path()?;
        let _output = TmcCommand::new_with_file_io("dotnet")?
            .with(|e| {
                e.cwd(path)
                    .arg(bootstrap_path)
                    .arg("--generate-points-file")
            })
            .output_checked()?;

        let exercise_desc_json = file_util::open_file(&exercise_desc_json_path)?;
        let json: HashMap<String, Vec<String>> =
            serde_json::from_reader(BufReader::new(exercise_desc_json))
                .map_err(|e| CSharpError::ParseExerciseDesc(exercise_desc_json_path, e))?;

        let mut tests = vec![];
        for (key, value) in json {
            tests.push(TestDesc::new(key, value));
        }
        Ok(ExerciseDesc {
            name: exercise_name,
            tests,
        })
    }

    /// Runs --run-tests and parses the resulting .tmc_test_results.json.
    fn run_tests_with_timeout(
        &self,
        path: &Path,
        timeout: Option<Duration>,
        _warnings: &mut Vec<anyhow::Error>,
    ) -> Result<RunResult, TmcError> {
        let test_results_path = path.join(".tmc_test_results.json");
        if test_results_path.exists() {
            file_util::remove_file(&test_results_path)?;
        }

        let bootstrap_path = Self::get_bootstrap_path()?;
        let command = TmcCommand::new_with_file_io("dotnet")?
            .with(|e| e.cwd(path).arg(bootstrap_path).arg("--run-tests"));
        let output = if let Some(timeout) = timeout {
            command.output_with_timeout(timeout)
        } else {
            command.output()
        };

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                log::trace!("stdout: {}", stdout);
                log::debug!("stderr: {}", stderr);
                let mut logs = HashMap::new();
                logs.insert("stdout".to_string(), stdout.into_owned());
                logs.insert("stderr".to_string(), stderr.into_owned());

                if !output.status.success() {
                    return Ok(RunResult {
                        status: RunStatus::CompileFailed,
                        test_results: vec![],
                        logs,
                    });
                }
                Self::parse_test_results(&test_results_path, logs).map_err(|e| e.into())
            }
            Err(TmcError::Command(CommandError::TimeOut { stdout, stderr, .. })) => {
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
            Err(error) => Err(error),
        }
    }

    /// No-op for C#.
    fn check_code_style(
        &self,
        _path: &Path,
        _locale: Language,
    ) -> Result<Option<ValidationResult>, TmcError> {
        Ok(Some(ValidationResult {
            strategy: Strategy::Disabled,
            validation_errors: None,
        }))
    }

    /// Removes all bin and obj sub-directories.
    fn clean(&self, path: &Path) -> Result<(), TmcError> {
        let test_results_path = path.join(".tmc_test_results.json");
        if test_results_path.exists() {
            file_util::remove_file(&test_results_path)?;
        }
        for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
            let file_name = entry.path().file_name();
            if file_name == Some(&OsString::from("bin"))
                || file_name == Some(&OsString::from("obj"))
            {
                file_util::remove_dir_all(entry.path())?;
            }
        }
        Ok(())
    }

    fn get_default_student_file_paths(&self) -> Vec<PathBuf> {
        vec![PathBuf::from("src")]
    }

    fn get_default_exercise_file_paths(&self) -> Vec<PathBuf> {
        vec![PathBuf::from("test")]
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
                sequence::delimited(
                    character::complete::char('"'),
                    bytes::complete::is_not("\""),
                    character::complete::char('"'),
                ),
                sequence::tuple((
                    character::complete::multispace0,
                    character::complete::char(')'),
                )),
            ),
            str::trim,
        )(i)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::fs::{self, File};
    use std::sync::Once;
    use tempfile::TempDir;

    static INIT_RUNNER: Once = Once::new();

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
        INIT_RUNNER.call_once(|| {
            let _ = CSharpPlugin::get_runner_dir().unwrap();
        });
    }

    fn copy_test_dir(path: &str) -> TempDir {
        init();
        let path = Path::new(path);

        let temp = tempfile::tempdir().unwrap();
        for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
            let target = temp.path().join(entry.path().strip_prefix(path).unwrap());
            if entry.path().is_dir() {
                log::trace!("creating dirs {}", entry.path().display());
                fs::create_dir_all(target).unwrap();
            } else {
                log::trace!(
                    "copy from {} to {}",
                    entry.path().display(),
                    target.display()
                );
                fs::copy(entry.path(), target).unwrap();
            }
        }
        temp
    }

    #[test]
    fn exercise_type_is_correct() {
        init();
        let temp = copy_test_dir("tests/data/PassingProject");
        let is = CSharpPlugin::is_exercise_type_correct(temp.path());
        assert!(is);
    }

    #[test]
    fn exercise_type_is_incorrect() {
        init();
        let temp = copy_test_dir("tests/data");
        let is = CSharpPlugin::is_exercise_type_correct(temp.path());
        assert!(!is);
    }

    #[test]
    fn finds_project_dir_in_zip() {
        let file = File::open("tests/data/Zipped.zip").unwrap();
        let mut zip = ZipArchive::new(file).unwrap();
        let dir = CSharpPlugin::find_project_dir_in_zip(&mut zip).unwrap();
        assert_eq!(dir, Path::new("Outer/Inner/PassingProject"))
    }

    #[test]
    fn no_project_dir_in_zip() {
        let file = File::open("tests/data/test.zip").unwrap();
        let mut zip = ZipArchive::new(file).unwrap();
        let dir = CSharpPlugin::find_project_dir_in_zip(&mut zip);
        assert!(dir.is_err())
    }

    #[test]
    fn scans_exercise() {
        init();
        let plugin = CSharpPlugin::new();
        let temp = copy_test_dir("tests/data/PassingProject");
        let scan = plugin
            .scan_exercise(temp.path(), "name".to_string(), &mut vec![])
            .unwrap();
        assert_eq!(scan.name, "name");
        assert_eq!(scan.tests.len(), 6);
    }

    #[test]
    fn runs_tests_passing() {
        init();
        let plugin = CSharpPlugin::new();
        let temp = copy_test_dir("tests/data/PassingProject");
        let res = plugin.run_tests(temp.path(), &mut vec![]).unwrap();
        assert_eq!(res.status, RunStatus::Passed);
        assert_eq!(res.test_results.len(), 2);
        for tr in res.test_results {
            assert!(tr.successful);
        }
        // assert!(res.logs.is_empty());
    }

    #[test]
    fn runs_tests_failing() {
        init();
        let plugin = CSharpPlugin::new();
        let temp = copy_test_dir("tests/data/FailingProject");
        let res = plugin.run_tests(temp.path(), &mut vec![]).unwrap();
        assert_eq!(res.status, RunStatus::TestsFailed);
        assert_eq!(res.test_results.len(), 1);
        let test_result = &res.test_results[0];
        assert!(!test_result.successful);
        assert!(test_result.points.is_empty());
        assert!(test_result.message.contains("Expected: False"));
        assert_eq!(test_result.exception.len(), 2);
        // assert!(res.logs.is_empty());
    }

    #[test]
    fn runs_tests_compile_err() {
        init();
        let plugin = CSharpPlugin::new();
        let temp = copy_test_dir("tests/data/NonCompilingProject");
        let res = plugin.run_tests(temp.path(), &mut vec![]).unwrap();
        assert_eq!(res.status, RunStatus::CompileFailed);
        assert!(!res.logs.is_empty());
        assert!(res
            .logs
            .get("stdout")
            .unwrap()
            .contains("This is a compile error"));
    }

    #[test]
    fn cleans() {
        init();
        let plugin = CSharpPlugin::new();
        let temp = copy_test_dir("tests/data/PassingProject");
        let bin_path = temp.path().join("src").join("PassingSample").join("bin");
        let obj_path_test = temp
            .path()
            .join("test")
            .join("PassingSampleTests")
            .join("obj");
        assert!(!bin_path.exists());
        assert!(!obj_path_test.exists());
        plugin.run_tests(temp.path(), &mut vec![]).unwrap();
        assert!(bin_path.exists());
        assert!(obj_path_test.exists());
        plugin.clean(temp.path()).unwrap();
        assert!(!bin_path.exists());
        assert!(!obj_path_test.exists());
    }

    #[test]
    fn parses_points() {
        let res = CSharpPlugin::points_parser("asd");
        assert!(res.is_err());

        let res = CSharpPlugin::points_parser("@Points(\"1\")").unwrap();
        assert_eq!(res.1, "1");

        let res = CSharpPlugin::points_parser("@  pOiNtS  (  \"  1  \"  )  ").unwrap();
        assert_eq!(res.1, "1");
    }
}
