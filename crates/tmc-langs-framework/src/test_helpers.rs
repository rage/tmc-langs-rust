//! Test helpers for plugin tests

use crate::{
    Archive, TmcProjectYml,
    domain::{ExerciseDesc, RunResult, RunStatus},
    error::TmcError,
    plugin::LanguagePlugin,
    policy::StudentFilePolicy,
};
use nom::{IResult, Parser, branch, bytes, character, combinator, sequence};
use nom_language::error::VerboseError;
use std::{
    io::{Read, Seek},
    ops::ControlFlow::{Break, Continue},
    path::{Path, PathBuf},
    time::Duration,
};

pub struct MockPolicy {
    project_config: TmcProjectYml,
}

impl StudentFilePolicy for MockPolicy {
    fn new_with_project_config(project_config: TmcProjectYml) -> Self
    where
        Self: Sized,
    {
        Self { project_config }
    }
    fn get_project_config(&self) -> &TmcProjectYml {
        &self.project_config
    }
    fn is_non_extra_student_file(&self, path: &Path) -> bool {
        path.starts_with("src")
    }
}

pub struct SimpleMockPolicy {
    project_config: TmcProjectYml,
}

impl StudentFilePolicy for SimpleMockPolicy {
    fn new_with_project_config(project_config: TmcProjectYml) -> Self
    where
        Self: Sized,
    {
        Self { project_config }
    }
    fn get_project_config(&self) -> &TmcProjectYml {
        &self.project_config
    }
    fn is_non_extra_student_file(&self, path: &Path) -> bool {
        // Consider files under "src" as student files, even if they're in subdirectories
        path.to_string_lossy().contains("/src/") || path.starts_with("src")
    }
}

pub struct MockPlugin {}

impl LanguagePlugin for MockPlugin {
    const PLUGIN_NAME: &'static str = "mock_plugin";
    const DEFAULT_SANDBOX_IMAGE: &'static str = "mock_image";
    const LINE_COMMENT: &'static str = "//";
    const BLOCK_COMMENT: Option<(&'static str, &'static str)> = Some(("/*", "*/"));
    type StudentFilePolicy = MockPolicy;

    fn scan_exercise(
        &self,
        _path: &Path,
        _exercise_name: String,
    ) -> Result<ExerciseDesc, TmcError> {
        unimplemented!()
    }

    fn run_tests_with_timeout(
        &self,
        _path: &Path,
        _timeout: Option<Duration>,
    ) -> Result<RunResult, TmcError> {
        Ok(RunResult {
            status: RunStatus::Passed,
            test_results: vec![],
            logs: std::collections::HashMap::new(),
        })
    }

    fn find_project_dir_in_archive<R: Read + Seek>(
        archive: &mut Archive<R>,
    ) -> Result<PathBuf, TmcError> {
        let mut iter = archive.iter()?;
        let project_dir = loop {
            let next = iter.with_next(|file| {
                let file_path = file.path()?;

                if let Some(parent) =
                    tmc_langs_util::path_util::get_parent_of_component_in_path(&file_path, "src")
                {
                    return Ok(Break(Some(parent)));
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

    fn is_exercise_type_correct(_path: &Path) -> bool {
        unimplemented!()
    }

    fn clean(&self, _path: &Path) -> Result<(), TmcError> {
        Ok(())
    }

    fn points_parser(i: &str) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
        combinator::map(
            sequence::delimited(
                (
                    bytes::complete::tag("@"),
                    character::complete::multispace0,
                    bytes::complete::tag_no_case("points"),
                    character::complete::multispace0,
                    character::complete::char('('),
                    character::complete::multispace0,
                ),
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
                (
                    character::complete::multispace0,
                    character::complete::char(')'),
                ),
            ),
            |s: &str| vec![s.trim()],
        )
        .parse(i)
    }

    fn get_default_student_file_paths() -> Vec<PathBuf> {
        vec![PathBuf::from("src")]
    }

    fn get_default_exercise_file_paths() -> Vec<PathBuf> {
        vec![PathBuf::from("test")]
    }
}

pub struct SimpleMockPlugin {}

impl LanguagePlugin for SimpleMockPlugin {
    const PLUGIN_NAME: &'static str = "simple_mock_plugin";
    const DEFAULT_SANDBOX_IMAGE: &'static str = "mock_image";
    const LINE_COMMENT: &'static str = "//";
    const BLOCK_COMMENT: Option<(&'static str, &'static str)> = Some(("/*", "*/"));
    type StudentFilePolicy = SimpleMockPolicy;

    fn scan_exercise(
        &self,
        _path: &Path,
        _exercise_name: String,
    ) -> Result<ExerciseDesc, TmcError> {
        unimplemented!()
    }

    fn run_tests_with_timeout(
        &self,
        _path: &Path,
        _timeout: Option<Duration>,
    ) -> Result<RunResult, TmcError> {
        Ok(RunResult {
            status: RunStatus::Passed,
            test_results: vec![],
            logs: std::collections::HashMap::new(),
        })
    }

    fn find_project_dir_in_archive<R: Read + Seek>(
        _archive: &mut Archive<R>,
    ) -> Result<PathBuf, TmcError> {
        // Always fail to find project directory to test fallback logic
        Err(TmcError::NoProjectDirInArchive)
    }

    fn is_exercise_type_correct(_path: &Path) -> bool {
        unimplemented!()
    }

    fn clean(&self, _path: &Path) -> Result<(), TmcError> {
        Ok(())
    }

    fn points_parser(i: &str) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
        combinator::map(
            sequence::delimited(
                (
                    bytes::complete::tag("@"),
                    character::complete::multispace0,
                    bytes::complete::tag_no_case("points"),
                    character::complete::multispace0,
                    character::complete::char('('),
                    character::complete::multispace0,
                ),
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
                (
                    character::complete::multispace0,
                    character::complete::char(')'),
                ),
            ),
            |s: &str| vec![s.trim()],
        )
        .parse(i)
    }

    fn get_default_student_file_paths() -> Vec<PathBuf> {
        vec![PathBuf::from("src")]
    }

    fn get_default_exercise_file_paths() -> Vec<PathBuf> {
        vec![PathBuf::from("test")]
    }
}
