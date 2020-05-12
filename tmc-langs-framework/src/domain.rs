pub mod meta_syntax;

use super::Result;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct TestDesc {
    pub name: String,
    pub points: Vec<String>,
}

impl TestDesc {
    pub fn new(name: String, points: Vec<String>) -> Self {
        Self { name, points }
    }
}

#[derive(Debug, Deserialize)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub points: Vec<String>,
    pub message: String,
    #[serde(default)]
    pub exception: Vec<String>,
}

#[derive(Debug)]
pub struct ExerciseDesc {
    pub name: String,
    pub tests: Vec<TestDesc>,
}

impl ExerciseDesc {
    pub fn new(name: String, tests: Vec<TestDesc>) -> Self {
        Self { name, tests }
    }
}

#[derive(Debug)]
pub struct RunResult {
    pub status: RunStatus,
    pub test_results: Vec<TestResult>,
    pub logs: HashMap<String, Vec<u8>>,
}

impl RunResult {
    pub fn new(
        status: RunStatus,
        test_results: Vec<TestResult>,
        logs: HashMap<String, Vec<u8>>,
    ) -> Self {
        Self {
            status,
            test_results,
            logs,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RunStatus {
    Passed,
    TestsFailed,
    CompileFailed,
    TestrunInterrupted,
    GenericError,
}

#[derive(Debug)]
pub struct ExercisePackagingConfiguration {
    pub student_file_paths: HashSet<PathBuf>,
    pub exercise_file_paths: HashSet<PathBuf>,
}

impl ExercisePackagingConfiguration {
    pub fn new(
        student_file_paths: HashSet<PathBuf>,
        exercise_file_paths: HashSet<PathBuf>,
    ) -> Self {
        Self {
            student_file_paths,
            exercise_file_paths,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct TmcProjectYml {
    #[serde(default)]
    pub extra_student_files: Vec<PathBuf>,
    #[serde(default)]
    pub extra_exercise_files: Vec<PathBuf>,
    #[serde(default)]
    pub force_update: Vec<PathBuf>,
}

impl TmcProjectYml {
    pub fn from(project_dir: &Path) -> Result<Self> {
        let mut config_path = project_dir.to_owned();
        config_path.push(".tmcproject.yml");
        let file = File::open(config_path)?;
        Ok(serde_yaml::from_reader(file)?)
    }
}
