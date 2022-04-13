//! Contains the main plugin struct.

use crate::{
    check_log::CheckLog, error::MakeError, policy::MakeStudentFilePolicy, valgrind_log::ValgrindLog,
};
use once_cell::sync::Lazy;
use regex::Regex;
use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader, Read, Seek},
    ops::ControlFlow::{Break, Continue},
    path::{Path, PathBuf},
    time::Duration,
};
use tmc_langs_framework::{
    nom::{bytes, character, combinator, error::VerboseError, sequence, IResult},
    Archive, CommandError, ExerciseDesc, LanguagePlugin, Output, PopenError, RunResult, RunStatus,
    TestDesc, TmcCommand, TmcError, TmcProjectYml,
};
use tmc_langs_util::{file_util, path_util, FileError};

#[derive(Default)]
pub struct MakePlugin {}

impl MakePlugin {
    pub fn new() -> Self {
        Self {}
    }

    /// Parses tmc_available_points.txt which is output by the TMC tests and
    /// contains lines like "[test] [test_one] 1.1 1.2 1.3" = "[type] [name] points".
    fn parse_exercise_desc(
        &self,
        available_points: &Path,
        exercise_name: String,
    ) -> Result<ExerciseDesc, MakeError> {
        // "[test] [test_one] 1.1 1.2 1.3" = "[type] [name] points"
        // TODO: use parser lib
        #[allow(clippy::unwrap_used)]
        static RE: Lazy<Regex> = Lazy::new(|| {
            Regex::new(r#"\[(?P<type>.*)\] \[(?P<name>.*)\] (?P<points>.*)"#).unwrap()
        });

        let mut tests = vec![];

        let file = file_util::open_file(available_points)?;

        let reader = BufReader::new(file);
        for line in reader.lines() {
            let line = line.map_err(|e| FileError::FileRead(available_points.to_path_buf(), e))?;

            if let Some(captures) = RE.captures(&line) {
                if &captures["type"] == "test" {
                    let name = captures["name"].to_string();
                    let points = captures["points"]
                        .split_whitespace()
                        .map(str::to_string)
                        .collect();
                    tests.push(TestDesc { name, points });
                }
            }
        }

        Ok(ExerciseDesc {
            name: exercise_name,
            tests,
        })
    }

    /// Runs tests with or without valgrind according to the argument.
    /// Returns an error if the command finishes unsuccessfully.
    /// TODO: no option for timeout
    fn run_tests_with_valgrind(
        &self,
        path: &Path,
        run_valgrind: bool,
    ) -> Result<Output, MakeError> {
        let arg = if run_valgrind {
            "run-test-with-valgrind"
        } else {
            "run-test"
        };
        log::info!("Running make {}", arg);

        let output = TmcCommand::piped("make")
            .with(|e| e.cwd(path).arg(arg))
            .output()?;

        log::trace!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::debug!("stderr: {}", stderr);

        if !output.status.success() {
            if run_valgrind {
                return Err(MakeError::RunningTestsWithValgrind(
                    output.status,
                    stderr.into_owned(),
                ));
            } else {
                return Err(MakeError::RunningTests(output.status, stderr.into_owned()));
            }
        }

        Ok(output)
    }

    /// Tries to build the project at the given directory, returns whether
    /// the process finished successfully or not.
    fn build(&self, dir: &Path) -> Result<Output, MakeError> {
        log::debug!("building {}", dir.display());
        let output = TmcCommand::piped("make")
            .with(|e| e.cwd(dir).arg("test"))
            .output()?;

        log::trace!("stdout:\n{}", String::from_utf8_lossy(&output.stdout));
        log::debug!("stderr:\n{}", String::from_utf8_lossy(&output.stderr));

        Ok(output)
    }
}

/// Project directory:
/// Contains a src directory and a Makefile file
impl LanguagePlugin for MakePlugin {
    const PLUGIN_NAME: &'static str = "make";
    const LINE_COMMENT: &'static str = "//";
    const BLOCK_COMMENT: Option<(&'static str, &'static str)> = Some(("/*", "*/"));
    type StudentFilePolicy = MakeStudentFilePolicy;

    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc, TmcError> {
        if !Self::is_exercise_type_correct(path) {
            return MakeError::NoExerciseFound(path.to_path_buf()).into();
        }

        self.run_tests_with_valgrind(path, false)?;

        let available_points_path = path.join("test/tmc_available_points.txt");

        if !available_points_path.exists() {
            return MakeError::CantFindAvailablePoints(available_points_path).into();
        }

        Ok(self.parse_exercise_desc(&available_points_path, exercise_name)?)
    }

    fn run_tests_with_timeout(
        &self,
        path: &Path,
        _timeout: Option<Duration>,
    ) -> Result<RunResult, TmcError> {
        let output = self.build(path)?;
        if !output.status.success() {
            let mut logs = HashMap::new();
            logs.insert(
                "stdout".to_string(),
                String::from_utf8_lossy(&output.stdout).into_owned(),
            );
            logs.insert(
                "stderr".to_string(),
                String::from_utf8_lossy(&output.stderr).into_owned(),
            );
            return Ok(RunResult {
                status: RunStatus::CompileFailed,
                test_results: vec![],
                logs,
            });
        }

        // try to run valgrind
        let mut ran_valgrind = true;
        let valgrind_run = self.run_tests_with_valgrind(path, true);
        let output = match valgrind_run {
            Ok(output) => output,
            Err(error) => match error {
                MakeError::Tmc(TmcError::Command(command_error)) => {
                    match command_error {
                        CommandError::Popen(_, PopenError::IoError(io_error))
                        | CommandError::FailedToRun(_, PopenError::IoError(io_error))
                            if io_error.kind() == io::ErrorKind::PermissionDenied =>
                        {
                            // failed due to lacking permissions, try to clean and rerun
                            let _output = self.clean(path)?;
                            match self.run_tests_with_valgrind(path, false) {
                                Ok(output) => output,
                                Err(err) => {
                                    log::error!(
                                        "Running with valgrind failed after trying to clean! {}",
                                        err
                                    );
                                    ran_valgrind = false;
                                    log::info!("Running without valgrind");
                                    self.run_tests_with_valgrind(path, false)?
                                }
                            }
                        }
                        _ => {
                            ran_valgrind = false;
                            log::info!("Running without valgrind");
                            self.run_tests_with_valgrind(path, false)?
                        }
                    }
                }
                MakeError::RunningTestsWithValgrind(..) => {
                    ran_valgrind = false;
                    log::info!("Running without valgrind");
                    self.run_tests_with_valgrind(path, false)?
                }
                err => {
                    log::warn!("unexpected error {:?}", err);
                    return Err(err.into());
                }
            },
        };
        let base_test_path = path.join("test");

        // fails on valgrind by default
        let fail_on_valgrind_error = match TmcProjectYml::load_or_default(path) {
            Ok(parsed) => parsed.fail_on_valgrind_error.unwrap_or(true),
            Err(_) => true,
        };

        // valgrind logs are only interesting if fail on valgrind error is on
        let valgrind_log = if ran_valgrind && fail_on_valgrind_error {
            let valgrind_path = base_test_path.join("valgrind.log");
            Some(ValgrindLog::from(&valgrind_path)?)
        } else {
            None
        };

        // parse available points into a mapping from test name to test point list
        let available_points_path = base_test_path.join("tmc_available_points.txt");
        let tests = self
            .parse_exercise_desc(&available_points_path, "unused".to_string())?
            .tests;
        let mut ids_to_points = HashMap::new();
        for test in tests {
            ids_to_points.insert(test.name, test.points);
        }

        // parse test results into RunResult
        let test_results_path = base_test_path.join("tmc_test_results.xml");

        let file_bytes = file_util::read_file(&test_results_path)?;

        // xml may contain invalid utf-8, ignore invalid characters
        let file_string = String::from_utf8_lossy(&file_bytes);

        let check_log: CheckLog = serde_xml_rs::from_str(&file_string)
            .map_err(|e| MakeError::XmlParseError(test_results_path, e))?;
        let mut logs = HashMap::new();
        logs.insert(
            "stdout".to_string(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        );
        logs.insert(
            "stderr".to_string(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        );
        let mut run_result = check_log.into_run_result(ids_to_points, logs);

        if let Some(valgrind_log) = valgrind_log {
            if valgrind_log.errors {
                // valgrind failed
                run_result.status = RunStatus::TestsFailed;
                // TODO: tests and valgrind results are not guaranteed to be in the same order
                for (test_result, valgrind_result) in run_result
                    .test_results
                    .iter_mut()
                    .zip(valgrind_log.results.into_iter())
                {
                    if valgrind_result.errors {
                        if test_result.successful {
                            test_result.message += " - Failed due to errors in valgrind log; see log below. Try submitting to server, some leaks might be platform dependent";
                        }
                        test_result.exception.extend(valgrind_result.log);
                    }
                }
            }
        }

        Ok(run_result)
    }

    fn find_project_dir_in_archive<R: Read + Seek>(
        archive: &mut Archive<R>,
    ) -> Result<PathBuf, TmcError> {
        let mut iter = archive.iter()?;

        let mut makefile_parents = vec![];
        let mut src_parents = vec![];
        let project_dir = loop {
            let next = iter.with_next(|file| {
                let file_path = file.path()?;

                if file.is_file() {
                    // check for Makefile
                    if let Some(parent) = path_util::get_parent_of(&file_path, "Makefile") {
                        if src_parents.contains(&parent) {
                            return Ok(Break(Some(parent)));
                        } else {
                            makefile_parents.push(parent);
                        }
                    }
                } else if file.is_dir() {
                    // check for src
                    if let Some(parent) = path_util::get_parent_of_dir(&file_path, "src") {
                        if makefile_parents.contains(&parent) {
                            return Ok(Break(Some(parent)));
                        } else {
                            src_parents.push(parent);
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
        if let Some(project_dir) = project_dir {
            Ok(project_dir)
        } else {
            Err(TmcError::NoProjectDirInArchive)
        }
    }

    /// Checks if the directory has a src dir and a Makefile file in it.
    fn is_exercise_type_correct(path: &Path) -> bool {
        path.join("src").is_dir() && path.join("Makefile").is_file()
    }

    // does not check for success
    fn clean(&self, path: &Path) -> Result<(), TmcError> {
        let output = TmcCommand::piped("make")
            .with(|e| e.cwd(path).arg("clean"))
            .output()?;

        if output.status.success() {
            log::info!("Cleaned make project");
        } else {
            log::warn!("Cleaning make project was not successful");
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
        fn tmc_register_test_parser(i: &str) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
            sequence::delimited(
                sequence::tuple((
                    bytes::complete::tag("tmc_register_test"),
                    character::complete::multispace0,
                    character::complete::char('('),
                    character::complete::multispace0,
                    arg_parser,
                    arg_parser,
                )),
                string_parser,
                sequence::tuple((
                    character::complete::multispace0,
                    character::complete::char(')'),
                )),
            )(i)
            .map(|(a, b)| (a, vec![b]))
        }

        // todo: currently cannot handle function calls with multiple parameters, probably not a problem
        fn arg_parser(i: &str) -> IResult<&str, &str, VerboseError<&str>> {
            combinator::value(
                "",
                sequence::tuple((
                    bytes::complete::take_till(|c: char| c.is_whitespace() || c == ','),
                    character::complete::char(','),
                    character::complete::multispace0,
                )),
            )(i)
        }

        fn string_parser(i: &str) -> IResult<&str, &str, VerboseError<&str>> {
            sequence::delimited(
                character::complete::char('"'),
                bytes::complete::is_not("\""),
                character::complete::char('"'),
            )(i)
        }

        tmc_register_test_parser(i)
    }
}

#[cfg(test)]
#[cfg(target_os = "linux")] // check not installed on other CI platforms
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new()
            .with_level(LevelFilter::Debug)
            // serde_xml_rs logs a lot
            .with_module_level("serde_xml_rs", LevelFilter::Warn)
            .init();
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

    fn dir_to(
        target_dir: impl AsRef<std::path::Path>,
        target_relative: impl AsRef<std::path::Path>,
    ) -> PathBuf {
        let target = target_dir.as_ref().join(target_relative);
        std::fs::create_dir_all(&target).unwrap();
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

    #[test]
    fn parses_exercise_desc() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        let available_points = file_to(
            &temp_dir,
            "available_points.txt",
            r#"
[test] [test1] point1 point2 point3 point4
[test] [test2] point5
[nontest] [nontest1] nonpoint
test [invalid] point6
[test] invalid point6
"#,
        );

        let plugin = MakePlugin::new();
        let exercise_desc = plugin
            .parse_exercise_desc(&available_points, "ex".to_string())
            .unwrap();
        assert_eq!(exercise_desc.tests.len(), 2);
        assert_eq!(exercise_desc.tests[0].points.len(), 4);
    }

    #[test]
    fn scans_exercise() {
        init();

        let temp = dir_to_temp("tests/data/passing-exercise");
        let plugin = MakePlugin::new();
        let exercise_desc = plugin
            .scan_exercise(temp.path(), "test".to_string())
            .unwrap();

        assert_eq!(exercise_desc.name, "test");
        assert_eq!(exercise_desc.tests.len(), 1);
        let test = &exercise_desc.tests[0];
        assert_eq!(test.name, "test_one");
        assert_eq!(test.points.len(), 1);
        assert_eq!(test.points[0], "1.1");
    }

    #[test]
    fn runs_tests() {
        init();

        let temp = dir_to_temp("tests/data/passing-exercise");
        let plugin = MakePlugin::new();
        let run_result = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(run_result.status, RunStatus::Passed);
        let test_results = run_result.test_results;
        assert_eq!(test_results.len(), 1);
        let test_result = &test_results[0];
        assert_eq!(test_result.name, "test_one");
        assert!(test_result.successful);
        assert_eq!(test_result.message, "Passed");
        assert!(test_result.exception.is_empty());
        let points = &test_result.points;
        assert_eq!(points.len(), 1);
        let point = &points[0];
        assert_eq!(point, "1.1");
    }

    #[test]
    fn runs_tests_failing() {
        init();

        let temp = dir_to_temp("tests/data/failing-exercise");
        let plugin = MakePlugin::new();
        let run_result = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(run_result.status, RunStatus::TestsFailed);
        let test_results = &run_result.test_results;
        assert_eq!(test_results.len(), 1);
        let test_result = &test_results[0];
        assert_eq!(test_result.name, "test_one");
        assert!(!test_result.successful);
        assert!(test_result.message.contains("Should have returned: 1"));
        let points = &test_result.points;
        assert_eq!(points.len(), 1);
        assert_eq!(points[0], "1.1");
    }

    // if this test causes problems just disable it, valgrind might be writing the results in a random order
    #[test]
    fn runs_tests_failing_valgrind() {
        init();

        let temp = dir_to_temp("tests/data/valgrind-failing-exercise");
        let plugin = MakePlugin::new();
        let run_result = plugin.run_tests(temp.path()).unwrap();
        assert_eq!(run_result.status, RunStatus::TestsFailed);
        let test_results = &run_result.test_results;
        assert_eq!(test_results.len(), 2);

        let test_one = &test_results[0];
        assert_eq!(test_one.name, "test_one");
        assert!(test_one.successful);
        assert_eq!(test_one.points.len(), 1);
        assert_eq!(test_one.points[0], "1.1");

        let test_two = &test_results[1];
        assert_eq!(test_two.name, "test_two");
        assert!(test_two.successful);
        assert_eq!(test_two.points.len(), 1);
        assert_eq!(test_two.points[0], "1.2");
    }

    #[test]
    fn finds_project_dir_in_zip() {
        init();
        let temp_dir = tempfile::tempdir().unwrap();
        dir_to(&temp_dir, "Outer/Inner/make_project/src");
        file_to(&temp_dir, "Outer/Inner/make_project/Makefile", "");

        let zip_contents = dir_to_zip(&temp_dir);
        let mut zip = Archive::zip(std::io::Cursor::new(zip_contents)).unwrap();
        let dir = MakePlugin::find_project_dir_in_archive(&mut zip).unwrap();
        assert_eq!(dir, Path::new("Outer/Inner/make_project"));
    }

    #[test]
    fn doesnt_find_project_dir_in_zip() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        dir_to(&temp_dir, "Outer/Inner/make_project/src");
        file_to(&temp_dir, "Outer/Inner/make_project/Makefil", "");

        let zip_contents = dir_to_zip(&temp_dir);
        let mut zip = Archive::zip(std::io::Cursor::new(zip_contents)).unwrap();
        let dir = MakePlugin::find_project_dir_in_archive(&mut zip);
        assert!(dir.is_err());
    }

    #[test]
    fn parses_points() {
        assert!(MakePlugin::points_parser(
            "tmc_register_test(s, test_insertion_empty_list, \"dlink_insert);",
        )
        .is_err());

        assert_eq!(
            MakePlugin::points_parser(
                "tmc_register_test(s, test_insertion_empty_list, \"dlink_insert\");",
            )
            .unwrap()
            .1[0],
            "dlink_insert"
        );
    }

    #[test]
    fn does_not_parse_check_function() {
        assert!(MakePlugin::points_parser(
            r#"tmc_register_test(Suite *s, TFun tf, const char *fname, const char *points)
{
    // stuff
}

int tmc_run_tests(int argc, const char **argv, Suite *s)
{
    func("--print-available-points")
}
"#
        )
        .is_err())
    }
}
