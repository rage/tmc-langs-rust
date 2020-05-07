pub mod domain;
pub mod io;

use domain::{Configuration, ExerciseDesc, ExercisePackagingConfiguration, RunResult};
use io::{sandbox, zip};
use isolang::Language;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tmc_langs_abstraction::ValidationResult;
use walkdir::WalkDir;

/// The trait that each language plug-in must implement.
///
/// These implement the operations needed by the TMC server to support a
/// programming language. These are provided as a library to IDE plug-ins as a
/// convenience. IDE plug-ins often need additional integration work to support a
/// language properly. This interface does NOT attempt to provide everything that
/// an IDE plug-in might need to fully support a language.
///
/// Parts of this interface may be called in a TMC sandbox.
///
/// Implementations must be thread-safe and preferably fully stateless. Users of
/// this interface are free to cache results if needed.
#[cfg_attr(test, mockall::automock)]
pub trait LanguagePlugin {
    /// Returns the name of the plug-in.
    fn get_plugin_name(&self) -> &str;

    /// Returns a list of all directories inside that contain an exercise in this
    /// language.
    ///
    /// These directories might overlap with directories returned by some other
    /// language plug-in.
    // TODO: rewrite using the exercise finder used by find exercises of the tmc-langs-cli?
    fn find_exercises(&self, base_path: &Path) -> Vec<PathBuf> {
        let mut exercises = vec![];
        if base_path.is_dir() {
            for entry in WalkDir::new(base_path)
                .into_iter()
                .filter_entry(|e| e.path().is_dir())
                .filter_map(|e| e.ok())
            {
                if self.is_exercise_type_correct(entry.path()) {
                    exercises.push(entry.into_path());
                }
            }
        }
        exercises
    }

    /// Produces an exercise description of an exercise directory.
    ///
    /// This involves finding the test cases and the points offered by the
    /// exercise.
    ///
    /// Must return `Err` if the given path is not a valid exercise directory for
    /// this language.
    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc>;

    /// Runs the tests for the exercise.
    fn run_tests(&self, path: &Path) -> RunResult;

    /// Prepares a submission for processing in the sandbox.
    ///
    /// The destination path is initialised with the original exercise as it
    /// appears in the course repository. The implementation should copy over a
    /// selection of files from the submission so that the student cannot e.g.
    /// easily replace the tests.
    fn prepare_submission(&self, submission_path: &Path, dest_path: &Path) {
        sandbox::move_files((), submission_path, dest_path);
    }

    /// Prepares a stub exercise from the original.
    ///
    /// The stub is a copy of the original where the model solution and special
    /// comments have been stripped and stubs like ('return 0') have been added.
    fn prepare_stubs(
        &self,
        exercise_map: HashMap<PathBuf, Box<dyn LanguagePlugin>>,
        repo_path: &Path,
        dest_path: &Path,
    ) -> Result<()> {
        Ok(domain::prepare_stubs(exercise_map, repo_path, dest_path)?)
    }

    /// Prepares a presentable solution from the original.
    ///
    /// The solution usually has stubs and special comments stripped.
    fn prepare_solutions(
        &self,
        exercise_map: HashMap<PathBuf, Box<dyn LanguagePlugin>>,
        repo_path: &Path,
        dest_path: &Path,
    ) -> Result<()> {
        Ok(domain::prepare_solutions(exercise_map.keys(), dest_path)?)
    }

    /// Run checkstyle or similar plugin to project if applicable
    fn check_code_style(&self, path: &Path, locale: Language) -> Option<ValidationResult>;

    /// Compress a given project so that it can be sent to the TestMyCode server.
    fn compress_project(&self, path: &Path) -> Vec<u8> {
        zip::student_file_aware_zip((), path)
    }

    /// Extract a given archive file containing a compressed project to a target location.
    ///
    /// This will overwrite any existing files as long as they are not specified as student files
    /// by the language dependent student file policy.
    fn extract_project(&self, compressed_project: &Path, target_location: &Path) {
        zip::student_file_aware_unzip((), compressed_project, target_location);
    }

    /// Tells if there's a valid exercise in this path.
    fn is_exercise_type_correct(&self, path: &Path) -> bool;

    /// Copy shared stuff to stub or solution used for example for copying tmc-junit-runner.
    fn maybe_copy_shared_stuff(&self, dest_path: &Path) {
        // no op by default
    }

    /// Returns configuration which is used to package submission on tmc-server.
    fn get_exercise_packaging_configuration(&self, path: &Path) -> ExercisePackagingConfiguration {
        let configuration = Configuration::from(path);
        let extra_student_files = configuration.get_extra_student_files();
        let extra_test_files = configuration.get_extra_exercise_files();

        let student_files = self
            .get_default_student_file_paths()
            .into_iter()
            .chain(extra_student_files)
            .collect::<HashSet<_>>();
        let exercise_files_without_student_files = self
            .get_default_exercise_file_paths()
            .into_iter()
            .chain(extra_test_files)
            .filter(|e| student_files.contains(e))
            .collect();
        ExercisePackagingConfiguration::new(student_files, exercise_files_without_student_files)
    }

    /// Runs clean command e.g `make clean` for make or `mvn clean` for maven.
    fn clean(&self, path: &Path);

    fn get_default_student_file_paths(&self) -> Vec<PathBuf> {
        vec![PathBuf::from("src")]
    }

    fn get_default_exercise_file_paths(&self) -> Vec<PathBuf> {
        vec![PathBuf::from("test")]
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("No matching plugin found")]
    PluginNotFound,
    #[error("Error processing files")]
    FileProcessing(#[from] std::io::Error),
    #[error(transparent)]
    Other(Box<dyn std::error::Error>),
}

pub type Result<T> = std::result::Result<T, Error>;
