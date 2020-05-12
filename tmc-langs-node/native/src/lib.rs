use isolang::Language;
use neon::prelude::*;
use neon_serde::export;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tmc_langs_util::{
    task_executor, Error as TmcError, ExerciseDesc, ExercisePackagingConfiguration, RunResult,
    ValidationResult,
};

#[derive(Debug, Deserialize, Serialize)]
enum Error {
    PluginNotFound,
    FileProcessing,
    YamlDeserialization,
    ZipError,
    NoProjectDirInZip,
    Other,
}

type Result<T> = std::result::Result<T, Error>;

impl From<TmcError> for Error {
    fn from(other: TmcError) -> Self {
        match other {
            TmcError::PluginNotFound => Error::PluginNotFound,
            TmcError::FileProcessing(_) => Error::FileProcessing,
            TmcError::YamlDeserialization(_) => Error::YamlDeserialization,
            TmcError::ZipError(_) => Error::ZipError,
            TmcError::NoProjectDirInZip => Error::NoProjectDirInZip,
            TmcError::Other(_) => Error::Other,
        }
    }
}

export! {
    fn prepareSolutions(exercise_paths: Vec<PathBuf>, dest_root: PathBuf) -> Result<()> {
        Ok(task_executor::prepare_solutions(
            &exercise_paths,
            &dest_root,
        )?)
    }

    fn prepareStubs(exercise_paths: Vec<PathBuf>, repo_path: PathBuf, dest_path: PathBuf) -> Result<()> {
        Ok(task_executor::prepare_stubs(exercise_paths, &repo_path, &dest_path)?)
    }

    fn runCheckCodeStyle(path: PathBuf, locale: Language) -> Result<Option<ValidationResult>> {
        Ok(task_executor::run_check_code_style(&path, locale)?)
    }

    fn runTests(path: PathBuf) -> Result<RunResult> {
        Ok(task_executor::run_tests(&path)?)
    }

    fn scanExercise(path: PathBuf, exercise_name: String) -> Result<ExerciseDesc> {
        Ok(task_executor::scan_exercise(&path, exercise_name)?)
    }

    fn isExerciseRootDirectory(path: PathBuf) -> bool {
        task_executor::is_exercise_root_directory(&path)
    }

    fn extractProject(compressed_project: PathBuf, target_location: PathBuf) -> Result<()> {
        Ok(task_executor::extract_project(&compressed_project, &target_location)?)
    }

    fn extractProjectOverwrite(compressed_project: PathBuf, target_location: PathBuf) -> Result<()> {
        Ok(task_executor::extract_project_overwrite(&compressed_project, &target_location)?)
    }

    fn compressProject(path: PathBuf) -> Result<Vec<u8>> {
        Ok(task_executor::compress_project(&path)?)
    }

    fn getExercisePackagingConfiguration(path: PathBuf) -> Result<ExercisePackagingConfiguration> {
        Ok(task_executor::get_exercise_packaging_configuration(&path)?)
    }

    fn compressTarForSubmitting(project_dir: PathBuf, tmc_langs: PathBuf, tmcrun: PathBuf, target_location: PathBuf) -> Result<()> {
        Ok(task_executor::compress_tar_for_submitting(&project_dir, &tmc_langs, &tmcrun, &target_location)?)
    }

    fn clean(path: PathBuf) -> Result<()> {
        Ok(task_executor::clean(&path)?)
    }
}
