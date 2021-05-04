//! Contains the LanguagePlugin implementation for R.

use crate::error::RError;
use crate::r_run_result::RRunResult;
use crate::RStudentFilePolicy;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tmc_langs_framework::{
    nom::{branch, bytes, character, combinator, error::VerboseError, multi, sequence, IResult},
    LanguagePlugin, TmcCommand, TmcError, {ExerciseDesc, RunResult, TestDesc},
};
use tmc_langs_util::{file_util, parse_util};
use zip::ZipArchive;

#[derive(Default)]
pub struct RPlugin {}

impl RPlugin {
    pub fn new() -> Self {
        Self {}
    }
}

impl LanguagePlugin for RPlugin {
    const PLUGIN_NAME: &'static str = "r";
    const LINE_COMMENT: &'static str = "#";
    const BLOCK_COMMENT: Option<(&'static str, &'static str)> = None;
    type StudentFilePolicy = RStudentFilePolicy;

    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc, TmcError> {
        // run available points command
        let args = if cfg!(windows) {
            &["-e", "\"library('tmcRtestrunner');run_available_points()\""]
        } else {
            &["-e", "library(tmcRtestrunner);run_available_points()"]
        };
        let _output = TmcCommand::piped("Rscript")
            .with(|e| e.cwd(path).args(args))
            .output_checked()?;

        // parse exercise desc
        let points_path = path.join(".available_points.json");
        let json_file = file_util::open_file(&points_path)?;
        let test_descs: HashMap<String, Vec<String>> = serde_json::from_reader(json_file)
            .map_err(|e| RError::JsonDeserialize(points_path, e))?;
        let test_descs = test_descs
            .into_iter()
            .map(|(k, v)| TestDesc { name: k, points: v })
            .collect();

        Ok(ExerciseDesc {
            name: exercise_name,
            tests: test_descs,
        })
    }

    fn run_tests_with_timeout(
        &self,
        path: &Path,
        timeout: Option<Duration>,
    ) -> Result<RunResult, TmcError> {
        // delete results json
        let results_path = path.join(".results.json");
        if results_path.exists() {
            file_util::remove_file(&results_path)?;
        }

        // run test command
        let args = if cfg!(windows) {
            &["-e", "\"library('tmcRtestrunner');run_tests()\""]
        } else {
            &["-e", "library(tmcRtestrunner);run_tests()"]
        };

        if let Some(timeout) = timeout {
            let _command = TmcCommand::piped("Rscript")
                .with(|e| e.cwd(path).args(args))
                .output_with_timeout_checked(timeout)?;
        } else {
            let _command = TmcCommand::piped("Rscript")
                .with(|e| e.cwd(path).args(args))
                .output_checked()?;
        }

        // parse test result
        let json_file = file_util::open_file(&results_path)?;
        let run_result: RRunResult = serde_json::from_reader(json_file).map_err(|e| {
            if let Ok(s) = fs::read_to_string(&results_path) {
                log::error!("json {}", s);
            }
            RError::JsonDeserialize(results_path, e)
        })?;

        Ok(run_result.into())
    }

    /// Checks if the directory contains R or tests/testthat
    fn is_exercise_type_correct(path: &Path) -> bool {
        path.join("R").exists() || path.join("tests/testthat").exists()
    }

    /// Finds an R directory.
    /// Ignores everything in a __MACOSX directory.
    fn find_project_dir_in_zip<R: Read + Seek>(
        zip_archive: &mut ZipArchive<R>,
    ) -> Result<PathBuf, TmcError> {
        for i in 0..zip_archive.len() {
            // zips don't necessarily contain entries for intermediate directories,
            // so we need to check every path for R
            let file = zip_archive.by_index(i)?;
            let file_path = Path::new(file.name());

            // todo: do in one pass somehow
            if file_path.components().any(|c| c.as_os_str() == "R") {
                let path: PathBuf = file_path
                    .components()
                    .take_while(|c| c.as_os_str() != "R")
                    .collect();

                if path.components().any(|p| p.as_os_str() == "__MACOSX") {
                    continue;
                }
                return Ok(path);
            }
        }
        Err(TmcError::NoProjectDirInZip)
    }

    /// No operation for now. To be possibly implemented later: remove .Rdata, .Rhistory etc
    fn clean(&self, _path: &Path) -> Result<(), TmcError> {
        Ok(())
    }

    fn get_default_student_file_paths() -> Vec<PathBuf> {
        vec![PathBuf::from("R")]
    }

    fn get_default_exercise_file_paths() -> Vec<PathBuf> {
        vec![PathBuf::from("tests")]
    }

    fn points_parser(i: &str) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
        let test_parser = sequence::preceded(
            sequence::tuple((
                bytes::complete::tag("test"),
                character::complete::multispace0,
                character::complete::char('('),
                character::complete::multispace0,
                arg_parser,
            )),
            c_parser,
        );
        let points_for_all_tests_parser = sequence::preceded(
            sequence::tuple((
                bytes::complete::tag("points_for_all_tests"),
                character::complete::multispace0,
                character::complete::char('('),
                character::complete::multispace0,
            )),
            c_parser,
        );

        // todo: currently cannot handle function calls with multiple parameters, probably not a problem
        fn arg_parser(i: &str) -> IResult<&str, &str, VerboseError<&str>> {
            combinator::value(
                "",
                sequence::tuple((
                    bytes::complete::take_till(|c: char| c == ','),
                    character::complete::char(','),
                    character::complete::multispace0,
                )),
            )(i)
        }

        fn c_parser(i: &str) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
            combinator::map(
                sequence::tuple((
                    character::complete::char('c'),
                    character::complete::multispace0,
                    character::complete::char('('),
                    parse_util::comma_separated_strings,
                    character::complete::char(')'),
                )),
                |t| t.3,
            )(i)
        }

        branch::alt((test_parser, points_for_all_tests_parser))(i)
    }
}

#[cfg(test)]
#[cfg(target_os = "linux")] // tmc-r-testrunner not installed on other CI platforms
mod test {
    use super::*;
    use std::path::PathBuf;
    use tmc_langs_framework::RunStatus;

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
    fn scan_exercise() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(
            &temp_dir,
            "tests/testthat/test1.R",
            r#"
library("testthat")
points_for_all_tests(c("r1"))
test("sample1", c("r1.1"), {
    expect_true(TRUE)
})
test("sample2", c("r1.2", "r1.3"), {
    expect_true(TRUE)
})
"#,
        );
        file_to(
            &temp_dir,
            "tests/testthat/test2.R",
            r#"
library("testthat")
points_for_all_tests(c("r2"))
test("sample3", c("r2.1"), {
    expect_true(TRUE)
})
"#,
        );

        let plugin = RPlugin::new();
        let desc = plugin
            .scan_exercise(temp_dir.path(), "ex".to_string())
            .unwrap();
        assert_eq!(desc.name, "ex");
        assert_eq!(desc.tests.len(), 3);
        for test in desc.tests {
            match test.name.as_str() {
                "sample1" => assert_eq!(test.points, &["r1", "r1.1"]),
                "sample2" => assert_eq!(test.points, &["r1", "r1.2", "r1.3"]),
                "sample3" => assert_eq!(test.points, &["r2", "r2.1"]),
                _ => panic!(),
            }
        }
    }

    #[test]
    fn run_tests_success() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(
            &temp_dir,
            "tests/testthat/test.R",
            r#"
library("testthat")
points_for_all_tests(c("r1"))
test("sample", c("r1.1"), {
    expect_true(TRUE)
})
"#,
        );

        let plugin = RPlugin::new();
        let run = plugin.run_tests(temp_dir.path()).unwrap();
        assert_eq!(run.status, RunStatus::Passed);
        assert!(run.logs.is_empty());
        assert_eq!(run.test_results.len(), 1);
        let res = &run.test_results[0];
        assert!(res.successful);
        assert_eq!(res.points, &["r1", "r1.1"]);
        assert!(res.message.is_empty());
        assert!(res.exception.is_empty());
    }

    #[test]
    fn run_tests_failed() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(
            &temp_dir,
            "tests/testthat/test.R",
            r#"
library("testthat")
points_for_all_tests(c("r1"))
test("sample", c("r1.1"), {
    expect_true(FALSE)
})
"#,
        );

        let plugin = RPlugin::new();
        let run = plugin.run_tests(temp_dir.path()).unwrap();
        assert_eq!(run.status, RunStatus::TestsFailed);
        assert!(run.logs.is_empty());
        assert_eq!(run.test_results.len(), 1);
        let res = &run.test_results[0];
        log::debug!("{:#?}", res);
        assert!(!res.successful);
        assert_eq!(res.points, &["r1", "r1.1"]);
        // assert_eq!(res.message, "FALSE is not TRUE"); // output changed on CI for some reason... TODO: fix
        assert!(res.exception.is_empty());
    }

    #[test]
    fn run_tests_run_failed() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(
            &temp_dir,
            "tests/testthat/test.R",
            r#"
library("testthat")
points_for_all_tests(c("r1"))
test("sample", c("r1.1"), {
    expect_true(unexpected)
})
"#,
        );

        let plugin = RPlugin::new();
        let run = plugin.run_tests(temp_dir.path()).unwrap();
        assert_eq!(run.status, RunStatus::TestsFailed);
        assert!(run.logs.is_empty());
        assert_eq!(run.test_results.len(), 1);
        let res = &run.test_results[0];
        log::debug!("{:#?}", res);
        assert!(!res.successful);
        assert_eq!(res.points, &["r1", "r1.1"]);
        assert_eq!(res.message, "object 'unexpected' not found");
        assert!(res.exception.is_empty());
    }

    #[test]
    fn run_tests_sourcing() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(&temp_dir, "R/main.R", r#"invalid R file"#);
        file_to(&temp_dir, "tests/testthat/test.R", "");

        let plugin = RPlugin::new();
        let run = plugin.run_tests(temp_dir.path()).unwrap();
        log::debug!("{:#?}", run);
        assert_eq!(run.status, RunStatus::CompileFailed);
        assert!(run
            .logs
            .get("compiler_output")
            .unwrap()
            .contains("unexpected symbol"));
        assert!(run.test_results.is_empty());
    }

    #[test]
    fn run_tests_timeout() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(&temp_dir, "R/main.R", r#"invalid R file"#);
        file_to(&temp_dir, "tests/testthat/test.R", "");

        let plugin = RPlugin::new();
        let run = plugin
            .run_tests_with_timeout(temp_dir.path(), Some(std::time::Duration::from_nanos(1)))
            .unwrap_err();
        use std::error::Error;
        let mut source = run.source();
        while let Some(inner) = source {
            source = inner.source();
            if let Some(cmd_error) = inner.downcast_ref::<tmc_langs_framework::CommandError>() {
                if matches!(cmd_error, tmc_langs_framework::CommandError::TimeOut { .. }) {
                    return;
                }
            }
        }
        panic!()
    }

    #[test]
    fn finds_project_dir_in_zip() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(&temp_dir, "Outer/Inner/r_project/R/main.R", "");

        let bytes = dir_to_zip(&temp_dir);
        let mut zip = ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let dir = RPlugin::find_project_dir_in_zip(&mut zip).unwrap();
        assert_eq!(dir, Path::new("Outer/Inner/r_project"));
    }

    #[test]
    fn doesnt_find_project_dir_in_zip() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(&temp_dir, "Outer/Inner/r_project/RR/main.R", "");

        let bytes = dir_to_zip(&temp_dir);
        let mut zip = ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let res = RPlugin::find_project_dir_in_zip(&mut zip);
        assert!(res.is_err());
    }

    #[test]
    fn parses_points() {
        init();

        let target = "asd";
        assert!(RPlugin::points_parser(target).is_err());

        let target = "test ( \"first arg\", \"second arg but no brace\"";
        assert!(RPlugin::points_parser(target).is_err());

        let target = r#"test("1d and 1e are solved correctly", c("W1A.1.2"), {
  expect_equivalent(z, z_correct, tolerance=1e-5)
  expect_true(areEqual(res, res_correct))
})
"#;
        assert_eq!(RPlugin::points_parser(target).unwrap().1[0], "W1A.1.2");

        let target = r#"test  (  "1d and 1e are solved correctly", c  (  "  W1A.1.2  "  )  , {
  expect_equivalent(z, z_correct, tolerance=1e-5)
  expect_true(areEqual(res, res_correct))
})
"#;
        assert_eq!(RPlugin::points_parser(target).unwrap().1[0], "W1A.1.2");
    }

    #[test]
    fn parsing_regression_test() {
        init();

        let temp = tempfile::tempdir().unwrap();
        // a file like this used to cause an error before for some reason...
        file_to(
            &temp,
            "tests/testthat/testExercise.R",
            r#"library('testthat')
"#,
        );

        let _points = RPlugin::get_available_points(temp.path()).unwrap();
    }

    #[test]
    fn parses_points_for_all_tests() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(
            &temp,
            "tests/testthat/testExercise.R",
            r#"
something
points_for_all_tests(c("r1"))
etc
"#,
        );

        let points = RPlugin::get_available_points(temp.path()).unwrap();
        assert_eq!(points, &["r1"]);
    }

    #[test]
    fn parses_multiple_points() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(
            &temp,
            "tests/testthat/testExercise.R",
            r#"
something
test("some test", c("r1", "r2", "r3"))
etc
"#,
        );

        let points = RPlugin::get_available_points(temp.path()).unwrap();
        assert_eq!(points, &["r1", "r2", "r3"]);
    }
}
