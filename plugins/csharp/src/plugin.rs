//! An implementation of LanguagePlugin for C#.

use crate::{cs_test_result::CSTestResult, policy::CSharpStudentFilePolicy, CSharpError};
use std::{
    collections::{HashMap, HashSet},
    env,
    ffi::{OsStr, OsString},
    io::{BufReader, Cursor, Read, Seek},
    ops::ControlFlow::{Break, Continue},
    path::{Path, PathBuf},
    time::Duration,
};
use tmc_langs_framework::{
    nom::{bytes, character, combinator, error::VerboseError, sequence, IResult},
    Archive, CommandError, ExerciseDesc, Language, LanguagePlugin, RunResult, RunStatus,
    StyleValidationResult, StyleValidationStrategy, TestDesc, TestResult, TmcCommand, TmcError,
};
use tmc_langs_util::{deserialize, file_util, parse_util, path_util, FileError};
use walkdir::WalkDir;
use zip::ZipArchive;

const TMC_CSHARP_RUNNER: &[u8] = include_bytes!("../deps/tmc-csharp-runner-1.1.1.zip");
const TMC_CSHARP_RUNNER_VERSION: &str = "1.1.1";

#[derive(Default)]
pub struct CSharpPlugin {}

impl CSharpPlugin {
    pub fn new() -> Self {
        Self {}
    }

    /// Extracts the bundled tmc-csharp-runner to the given path.
    fn extract_runner_to_dir(target: &Path) -> Result<(), CSharpError> {
        log::debug!("extracting C# runner to {}", target.display());

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
                    .map_err(|e| FileError::FileRead(file_path, e))?;
                file_util::write_to_file(&mut bytes.as_slice(), target_file_path)?;
            }
        }
        Ok(())
    }

    /// Returns the directory of the TMC C# runner, writing it to the cache dir if it doesn't exist there yet.
    ///
    /// NOTE: May cause issues if called concurrently.
    /// TODO: Currently this is checked every time when necessary. It could also be done in the constructor, but then it would be done in cases where unnecessary (when checking code style, for example)
    fn get_or_init_runner_dir() -> Result<PathBuf, CSharpError> {
        log::debug!("getting C# runner dir");
        match dirs::cache_dir() {
            Some(cache_dir) => {
                let runner_dir = cache_dir.join("tmc").join("tmc-csharp-runner");
                let version_path = runner_dir.join("VERSION");

                let needs_update = if version_path.exists() {
                    let version = file_util::read_file_to_string(&version_path)?;
                    version != TMC_CSHARP_RUNNER_VERSION
                } else {
                    true
                };

                if needs_update {
                    log::debug!("updating the cached C# runner");
                    if runner_dir.exists() {
                        // clear the directory if it exists
                        file_util::remove_dir_all(&runner_dir)?;
                    }
                    Self::extract_runner_to_dir(&runner_dir)?;
                    file_util::write_to_file(TMC_CSHARP_RUNNER_VERSION.as_bytes(), version_path)?;
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
            let runner_path = Self::get_or_init_runner_dir()?;
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
    ) -> Result<(RunStatus, Vec<TestResult>), CSharpError> {
        log::debug!("parsing C# test results");
        let test_results = file_util::open_file(test_results_path)?;
        let test_results: Vec<CSTestResult> = deserialize::json_from_reader(test_results)
            .map_err(|e| CSharpError::ParseTestResults(test_results_path.to_path_buf(), e))?;

        let mut status = RunStatus::Passed;
        let mut failed_points = HashSet::new();
        for test_result in &test_results {
            if !test_result.passed {
                status = RunStatus::TestsFailed;
                failed_points.extend(test_result.points.iter().cloned());
            }
        }

        // convert the parsed C# test results into TMC test results
        let test_results = test_results
            .into_iter()
            .map(|t| t.into_test_result(&failed_points))
            .collect();
        Ok((status, test_results))
    }
}

/// Project directory:
/// Contains a src directory which contains a .csproj file (which may be inside a subdirectory).
impl LanguagePlugin for CSharpPlugin {
    const PLUGIN_NAME: &'static str = "csharp";
    const LINE_COMMENT: &'static str = "//";
    const BLOCK_COMMENT: Option<(&'static str, &'static str)> = Some(("/*", "*/"));
    type StudentFilePolicy = CSharpStudentFilePolicy;

    fn is_exercise_type_correct(path: &Path) -> bool {
        WalkDir::new(path.join("src"))
            .max_depth(2)
            .into_iter()
            .filter_map(|e| e.ok())
            .any(|e| e.path().extension() == Some(&OsString::from("csproj")))
    }

    fn find_project_dir_in_archive<R: Read + Seek>(
        archive: &mut Archive<R>,
    ) -> Result<PathBuf, TmcError> {
        let mut iter = archive.iter()?;
        let project_dir = loop {
            let next = iter.with_next(|entry| {
                let file_path = entry.path()?;

                if entry.is_file()
                    && file_path.extension() == Some(OsStr::new("csproj"))
                    && !file_path.components().any(|c| c.as_os_str() == "__MACOSX")
                {
                    if let Some(parent) = file_path.parent() {
                        if let Some(src_parent) = path_util::get_parent_of(parent, "src") {
                            return Ok(Break(Some(src_parent)));
                        }
                        if let Some(parent) = parent.parent() {
                            if let Some(src_parent) = path_util::get_parent_of(parent, "src") {
                                return Ok(Break(Some(src_parent)));
                            }
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
            None => Err(TmcError::NoProjectDirInArchive),
        }
    }

    /// Runs --generate-points-file and parses the generated .tmc_available_points.json.
    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc, TmcError> {
        // clean old points file
        let exercise_desc_json_path = path.join(".tmc_available_points.json");
        if exercise_desc_json_path.exists() {
            file_util::remove_file(&exercise_desc_json_path)?;
        }

        // run command
        let bootstrap_path = Self::get_bootstrap_path()?;
        let _output = TmcCommand::piped("dotnet")
            .with(|e| {
                e.cwd(path)
                    .arg(bootstrap_path)
                    .arg("--generate-points-file")
            })
            .output_checked()?;

        // TODO: the command above can fail silently in some edge cases
        // parse result file
        let exercise_desc_json = file_util::open_file(&exercise_desc_json_path)?;
        let json: HashMap<String, Vec<String>> =
            deserialize::json_from_reader(BufReader::new(exercise_desc_json))
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
    ) -> Result<RunResult, TmcError> {
        // clean old file
        let test_results_path = path.join(".tmc_test_results.json");
        if test_results_path.exists() {
            file_util::remove_file(&test_results_path)?;
        }

        // run command
        let bootstrap_path = Self::get_bootstrap_path()?;
        let command = TmcCommand::piped("dotnet")
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
                if !output.status.success() {
                    log::warn!("stdout: {}", stdout);
                    log::error!("stderr: {}", stderr);
                    let mut logs = HashMap::new();
                    logs.insert("stdout".to_string(), stdout.into_owned());
                    logs.insert("stderr".to_string(), stderr.into_owned());
                    return Ok(RunResult {
                        status: RunStatus::CompileFailed,
                        test_results: vec![],
                        logs,
                    });
                }

                log::trace!("stdout: {}", stdout);
                log::debug!("stderr: {}", stderr);

                if !test_results_path.exists() {
                    return Err(CSharpError::MissingTestResults {
                        path: test_results_path,
                        stdout: stdout.into_owned(),
                        stderr: stderr.into_owned(),
                    }
                    .into());
                }
                let (status, test_results) = Self::parse_test_results(&test_results_path)?;
                file_util::remove_file(&test_results_path)?;

                let mut logs = HashMap::new();
                logs.insert("stdout".to_string(), stdout.into_owned());
                logs.insert("stderr".to_string(), stderr.into_owned());
                Ok(RunResult {
                    status,
                    test_results,
                    logs,
                })
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
    ) -> Result<Option<StyleValidationResult>, TmcError> {
        Ok(Some(StyleValidationResult {
            strategy: StyleValidationStrategy::Disabled,
            validation_errors: None,
        }))
    }

    /// Removes all bin and obj sub-directories.
    fn clean(&self, path: &Path) -> Result<(), TmcError> {
        // clean old result file
        let test_results_path = path.join(".tmc_test_results.json");
        if test_results_path.exists() {
            log::info!("removing old test results file");
            file_util::remove_file(&test_results_path)?;
        }

        // delete bin and obj directories
        for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
            let file_name = entry.path().file_name();
            if entry.path().is_dir()
                && (file_name == Some(&OsString::from("bin"))
                    || file_name == Some(&OsString::from("obj")))
            {
                log::info!("cleaning directory {}", entry.path().display());
                file_util::remove_dir_all(entry.path())?;
            }
        }
        Ok(())
    }

    fn get_default_student_file_paths() -> Vec<PathBuf> {
        vec![PathBuf::from("src")]
    }

    fn get_default_exercise_file_paths() -> Vec<PathBuf> {
        vec![PathBuf::from("test")]
    }

    fn points_parser(i: &str) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
        combinator::map(
            sequence::delimited(
                sequence::tuple((
                    character::complete::char('['),
                    character::complete::multispace0,
                    bytes::complete::tag_no_case("points"),
                    character::complete::multispace0,
                    character::complete::char('('),
                    character::complete::multispace0,
                )),
                parse_util::comma_separated_strings,
                sequence::tuple((
                    character::complete::multispace0,
                    character::complete::char(')'),
                    character::complete::multispace0,
                    character::complete::char(']'),
                )),
            ),
            // splits each point by whitespace
            |points| {
                points
                    .into_iter()
                    .flat_map(|p| p.split_whitespace())
                    .collect()
            },
        )(i)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use once_cell::sync::Lazy;
    use std::sync::{Mutex, Once};
    use tempfile::TempDir;

    static INIT_RUNNER: Once = Once::new();
    // running the runner in parallel seems to sometimes make tests run for an excessively long time
    static MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new().with_level(LevelFilter::Debug).init();
        INIT_RUNNER.call_once(|| {
            let _ = CSharpPlugin::get_or_init_runner_dir().unwrap();
        });
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
        use std::io::Write;

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
                zip.add_directory(rela, zip::write::FileOptions::default())
                    .unwrap();
            } else if entry.path().is_file() {
                zip.start_file(rela, zip::write::FileOptions::default())
                    .unwrap();
                let bytes = std::fs::read(entry.path()).unwrap();
                zip.write_all(&bytes).unwrap();
            }
        }

        zip.finish().unwrap();
        drop(zip);
        target
    }

    fn dir_to_temp(source_dir: impl AsRef<std::path::Path>) -> tempfile::TempDir {
        let temp = tempfile::TempDir::new().unwrap();
        for entry in walkdir::WalkDir::new(&source_dir).min_depth(1) {
            let entry = entry.unwrap();
            let rela = entry.path().strip_prefix(&source_dir).unwrap();
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
    fn extracts_runner_to_dir() {
        init();

        let temp = tempfile::TempDir::new().unwrap();
        CSharpPlugin::extract_runner_to_dir(temp.path()).unwrap();
        assert!(temp.path().join("TestMyCode.CSharp.Bootstrap.dll").exists());
    }

    #[test]
    fn gets_bootstrap_path() {
        init();

        let path = CSharpPlugin::get_bootstrap_path().unwrap();
        assert!(path
            .to_string_lossy()
            .contains("TestMyCode.CSharp.Bootstrap.dll"));
    }

    #[test]
    fn parses_test_results() {
        init();

        let temp = tempfile::TempDir::new().unwrap();
        let json = file_to(
            &temp,
            ".tmc_test_results.json",
            r#"
[
    {
        "Name": "n1",
        "Passed": true,
        "Message": "m1",
        "Points": ["1", "2"],
        "ErrorStackTrace": []
    },
    {
        "Name": "n2",
        "Passed": false,
        "Message": "m2",
        "Points": [],
        "ErrorStackTrace": ["err"]
    }
]
"#,
        );
        let (status, test_results) = CSharpPlugin::parse_test_results(&json).unwrap();
        assert_eq!(status, RunStatus::TestsFailed);
        assert_eq!(test_results.len(), 2);
    }

    #[test]
    fn exercise_type_is_correct() {
        init();

        let temp = TempDir::new().unwrap();
        file_to(&temp, "src/dir/sample.csproj", "");
        assert!(CSharpPlugin::is_exercise_type_correct(temp.path()));
    }

    #[test]
    fn exercise_type_is_incorrect() {
        init();

        let temp = TempDir::new().unwrap();
        file_to(&temp, "src/dir/dir/dir/sample.csproj", "");
        file_to(&temp, "sample.csproj", "");
        assert!(!CSharpPlugin::is_exercise_type_correct(temp.path()));
    }

    #[test]
    fn finds_project_dir_in_zip() {
        init();

        let temp = TempDir::new().unwrap();
        file_to(&temp, "dir1/dir2/dir3/src/dir4/sample.csproj", "");
        let bytes = dir_to_zip(&temp);
        let mut zip = Archive::zip(std::io::Cursor::new(bytes)).unwrap();
        let dir = CSharpPlugin::find_project_dir_in_archive(&mut zip).unwrap();
        assert_eq!(dir, Path::new("dir1/dir2/dir3"))
    }

    #[test]
    fn no_project_dir_in_zip() {
        init();

        let temp = TempDir::new().unwrap();
        file_to(&temp, "dir1/dir2/dir3/not src/directly in src.csproj", "");
        file_to(&temp, "dir1/dir2/dir3/src/__MACOSX/under macosx.csproj", "");
        file_to(&temp, "dir1/__MACOSX/dir3/src/dir/under macosx.csproj", "");
        let bytes = dir_to_zip(&temp);
        let mut zip = Archive::zip(std::io::Cursor::new(bytes)).unwrap();
        let dir = CSharpPlugin::find_project_dir_in_archive(&mut zip);
        assert!(dir.is_err())
    }

    #[test]
    fn scans_exercise() {
        init();
        let _lock = MUTEX.lock().unwrap();

        let temp = dir_to_temp("tests/data/passing-exercise");
        let plugin = CSharpPlugin::new();
        let scan = plugin
            .scan_exercise(temp.path(), "name".to_string())
            .unwrap();
        assert_eq!(scan.name, "name");
        assert_eq!(scan.tests.len(), 2);
    }

    #[test]
    fn runs_tests_passing() {
        init();
        let _lock = MUTEX.lock().unwrap();

        let temp = dir_to_temp("tests/data/passing-exercise");
        let plugin = CSharpPlugin::new();
        let res = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(res.status, RunStatus::Passed);
        assert_eq!(res.test_results.len(), 2);
        for tr in res.test_results {
            assert!(tr.successful);
        }
        assert!(res.logs.get("stdout").unwrap().is_empty());
        assert!(res.logs.get("stderr").unwrap().is_empty());
    }

    #[test]
    fn runs_tests_failing() {
        init();
        let _lock = MUTEX.lock().unwrap();

        let temp = dir_to_temp("tests/data/failing-exercise");
        let plugin = CSharpPlugin::new();
        let res = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(res.status, RunStatus::TestsFailed);
        assert_eq!(res.test_results.len(), 1);
        let test_result = &res.test_results[0];
        assert!(!test_result.successful);
        assert!(test_result.points.is_empty());
        assert!(test_result.message.contains("Expected: False"));
        assert_eq!(test_result.exception.len(), 2);
        assert!(res.logs.get("stdout").unwrap().is_empty());
        assert!(res.logs.get("stderr").unwrap().is_empty());
    }

    #[test]
    fn runs_tests_compile_err() {
        init();
        let _lock = MUTEX.lock().unwrap();

        let temp = dir_to_temp("tests/data/non-compiling-exercise");
        let plugin = CSharpPlugin::new();
        let res = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(res.status, RunStatus::CompileFailed);
        assert!(!res.logs.is_empty());
        log::debug!("{:?}", res.logs.get("stdout"));
        assert!(res
            .logs
            .get("stdout")
            .unwrap()
            .contains("This is a compile error"));
    }

    #[test]
    fn runs_tests_timeout() {
        init();
        let _lock = MUTEX.lock().unwrap();

        let temp = dir_to_temp("tests/data/passing-exercise");
        let plugin = CSharpPlugin::new();
        let res = plugin
            .run_tests_with_timeout(temp.path(), Some(std::time::Duration::from_nanos(1)))
            .unwrap();
        assert_eq!(res.status, RunStatus::TestsFailed);
    }

    #[test]
    fn cleans() {
        init();
        let _lock = MUTEX.lock().unwrap();

        let temp = dir_to_temp("tests/data/passing-exercise");
        let plugin = CSharpPlugin::new();
        let bin_path = temp.path().join("src").join("PassingSample").join("bin");
        let obj_path_test = temp
            .path()
            .join("test")
            .join("PassingSampleTests")
            .join("obj");
        assert!(!bin_path.exists());
        assert!(!obj_path_test.exists());
        plugin.run_tests(temp.path()).unwrap();
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

        let res = CSharpPlugin::points_parser("[Points(\"1\")]").unwrap();
        assert_eq!(res.1, &["1"]);

        let res = CSharpPlugin::points_parser("[  pOiNtS  (  \"  1  \"  )  ]").unwrap();
        assert_eq!(res.1, &["1"]);

        let res = CSharpPlugin::points_parser("[Points(\"1\", \"2\"  ,  \"3\")]").unwrap();
        assert_eq!(res.1, &["1", "2", "3"]);
    }

    #[test]
    fn doesnt_give_points_unless_all_relevant_exercises_pass() {
        init();
        let _lock = MUTEX.lock().unwrap();

        let temp = dir_to_temp("tests/data/partially-passing");
        let plugin = CSharpPlugin::new();
        let results = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(results.status, RunStatus::TestsFailed);
        let mut got_point = false;
        for test in results.test_results {
            got_point = got_point || test.points.contains(&"1.2".to_string());
            assert!(!test.points.contains(&"1".to_string()));
            assert!(!test.points.contains(&"1.1".to_string()));
        }
    }
}
