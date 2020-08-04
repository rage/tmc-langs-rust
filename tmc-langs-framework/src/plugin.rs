//! Contains LanguagePlugin.

pub use isolang::Language;

use super::domain::{
    ExerciseDesc, ExercisePackagingConfiguration, RunResult, RunStatus, TestResult, TmcProjectYml,
    ValidationResult,
};
use super::io::{submission_processing, tmc_zip};
use super::policy::StudentFilePolicy;
use super::Result;
use crate::TmcError;

use log::debug;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use walkdir::WalkDir;
use zip::ZipArchive;

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
pub trait LanguagePlugin {
    const PLUGIN_NAME: &'static str;
    type StudentFilePolicy: StudentFilePolicy + 'static;

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
                if Self::is_exercise_type_correct(entry.path()) {
                    debug!("found exercise {}", entry.path().display());
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
    fn run_tests(&self, path: &Path) -> Result<RunResult> {
        let timeout = Self::get_student_file_policy(path)
            .get_tmc_project_yml()
            .ok()
            .and_then(|t| t.tests_timeout_ms.map(Duration::from_millis));
        let result = self.run_tests_with_timeout(path, timeout)?;

        // override success on no test cases
        if result.status == RunStatus::Passed && result.test_results.is_empty() {
            Ok(RunResult {
                status: RunStatus::TestsFailed,
                test_results: vec![TestResult {
                    name: "Tests found test".to_string(),
                    successful: true,
                    points: vec![],
                    message: "No tests found.\nTry submitting the exercise to the server."
                        .to_string(),
                    exception: vec![],
                }],
                logs: HashMap::new(),
            })
        } else {
            Ok(result)
        }
    }

    /// Runs the tests for the exercise with the given timeout.
    fn run_tests_with_timeout(&self, path: &Path, timeout: Option<Duration>) -> Result<RunResult>;

    /// Prepares a submission for processing in the sandbox.
    ///
    /// The destination path is initialised with the original exercise as it
    /// appears in the course repository. The implementation should copy over a
    /// selection of files from the submission so that the student cannot e.g.
    /// easily replace the tests.
    fn prepare_submission(
        &self,
        policy: Self::StudentFilePolicy,
        submission_path: &Path,
        dest_path: &Path,
    ) -> Result<()> {
        Ok(submission_processing::move_files(
            policy,
            submission_path,
            dest_path,
        )?)
    }

    /// Prepares a stub exercise from the original.
    ///
    /// The stub is a copy of the original where the model solution and special
    /// comments have been stripped and stubs like ('return 0') have been added.
    fn prepare_stub(&self, exercise_path: &Path, repo_path: &Path, dest_path: &Path) -> Result<()> {
        submission_processing::prepare_stub(exercise_path, dest_path)?;

        let relative_path = exercise_path
            .strip_prefix(repo_path)
            .unwrap_or(exercise_path);
        self.maybe_copy_shared_stuff(&dest_path.join(relative_path))?;
        Ok(())
    }

    /// Prepares a presentable solution from the original.
    ///
    /// The solution usually has stubs and special comments stripped.
    fn prepare_solution(&self, exercise_paths: Vec<PathBuf>, dest_path: &Path) -> Result<()> {
        Ok(submission_processing::prepare_solutions(
            &exercise_paths,
            dest_path,
        )?)
    }

    /// Run checkstyle or similar plugin to project if applicable, empty by default
    fn check_code_style(&self, _path: &Path, _locale: Language) -> Option<ValidationResult> {
        None
    }

    /// Compress a given project so that it can be sent to the TestMyCode server.
    fn compress_project(&self, path: &Path) -> Result<Vec<u8>> {
        let policy = Self::get_student_file_policy(path);
        Ok(tmc_zip::zip(policy, path)?)
    }

    fn get_student_file_policy(project_path: &Path) -> Self::StudentFilePolicy;

    /// Extract a given archive file containing a compressed project to a target location.
    ///
    /// This will overwrite any existing files as long as they are not specified as student files
    /// by the language dependent student file policy.
    fn extract_project(&self, compressed_project: &Path, target_location: &Path) -> Result<()> {
        let policy = Self::get_student_file_policy(target_location);
        let zip = compressed_project;
        let target = target_location;

        log::debug!("Unzipping {} to {}", zip.display(), target.display());

        let file = File::open(zip).map_err(|e| TmcError::OpenFile(zip.to_path_buf(), e))?;
        let mut zip_archive = ZipArchive::new(file)?;

        let project_dir = Self::find_project_dir_in_zip(&mut zip_archive)?;
        log::debug!("Project dir in zip: {}", project_dir.display());

        let tmc_project_yml = policy.get_tmc_project_yml()?;

        // for each file in the zip, contains its path if unzipped
        // used to clean non-student files not in the zip later
        let mut unzip_paths = HashSet::new();

        for i in 0..zip_archive.len() {
            let mut file = zip_archive.by_index(i)?;
            let file_path = file.sanitized_name();
            let relative = match file_path.strip_prefix(&project_dir) {
                Ok(relative) => relative,
                _ => {
                    log::trace!("skip {}, not in project dir", file.name());
                    continue;
                }
            };
            let path_in_target = target.join(&relative);
            log::trace!("processing {:?} -> {:?}", file_path, path_in_target);

            if file.is_dir() {
                log::trace!("creating {:?}", path_in_target);
                fs::create_dir_all(&path_in_target)
                    .map_err(|e| TmcError::CreateDir(path_in_target.clone(), e))?;
                unzip_paths.insert(
                    path_in_target
                        .canonicalize()
                        .map_err(|e| TmcError::Canonicalize(path_in_target.clone(), e))?,
                );
            } else {
                let mut write = true;
                let mut file_contents = vec![];
                file.read_to_end(&mut file_contents)
                    .map_err(|e| TmcError::FileRead(file_path.clone(), e))?;
                // always overwrite .tmcproject.yml
                if path_in_target.exists()
                    && !path_in_target
                        .file_name()
                        .map(|o| o == ".tmcproject.yml")
                        .unwrap_or_default()
                {
                    let mut target_file = File::open(&path_in_target)
                        .map_err(|e| TmcError::OpenFile(path_in_target.clone(), e))?;
                    let mut target_file_contents = vec![];
                    target_file
                        .read_to_end(&mut target_file_contents)
                        .map_err(|e| TmcError::FileRead(path_in_target.clone(), e))?;
                    if file_contents == target_file_contents
                        || (policy.is_student_file(&path_in_target, &target, &tmc_project_yml)?
                            && !policy.is_updating_forced(&relative, &tmc_project_yml)?)
                    {
                        write = false;
                    }
                }
                if write {
                    log::trace!("writing to {}", path_in_target.display());
                    if let Some(parent) = path_in_target.parent() {
                        fs::create_dir_all(parent)
                            .map_err(|e| TmcError::CreateDir(parent.to_path_buf(), e))?;
                    }
                    let mut overwrite_target = File::create(&path_in_target)
                        .map_err(|e| TmcError::CreateFile(path_in_target.clone(), e))?;
                    overwrite_target
                        .write_all(&file_contents)
                        .map_err(|e| TmcError::Write(path_in_target.clone(), e))?;
                }
            }
            unzip_paths.insert(
                path_in_target
                    .canonicalize()
                    .map_err(|e| TmcError::Canonicalize(path_in_target.clone(), e))?,
            );
        }

        // delete non-student files that were not in zip
        log::debug!("deleting non-student files not in zip");
        log::debug!("{:?}", unzip_paths);
        for entry in WalkDir::new(target).into_iter().filter_map(|e| e.ok()) {
            if !unzip_paths.contains(
                &entry
                    .path()
                    .canonicalize()
                    .map_err(|e| TmcError::Canonicalize(entry.path().to_path_buf(), e))?,
            ) && (policy.is_updating_forced(entry.path(), &tmc_project_yml)?
                || !policy.is_student_file(entry.path(), &target, &tmc_project_yml)?)
            {
                log::debug!("rm {} {}", entry.path().display(), target.display());
                if entry.path().is_dir() {
                    // delete if empty
                    if WalkDir::new(entry.path()).max_depth(1).into_iter().count() == 1 {
                        log::debug!("deleting empty directory {}", entry.path().display());
                        fs::remove_dir(entry.path())
                            .map_err(|e| TmcError::RemoveDir(entry.path().to_path_buf(), e))?;
                    }
                } else {
                    log::debug!("removing file {}", entry.path().display());
                    fs::remove_file(entry.path())
                        .map_err(|e| TmcError::RemoveFile(entry.path().to_path_buf(), e))?;
                }
            }
        }

        Ok(())
    }

    /// Searches the zip for a valid project directory.
    /// Note that the returned path may not actually have an entry in the zip.
    fn find_project_dir_in_zip<R: Read + Seek>(zip_archive: &mut ZipArchive<R>) -> Result<PathBuf>;

    /// Tells if there's a valid exercise in this path.
    fn is_exercise_type_correct(path: &Path) -> bool;

    /// Copy shared stuff to stub or solution used for example for copying tmc-junit-runner.
    #[allow(unused_variables)]
    fn maybe_copy_shared_stuff(&self, dest_path: &Path) -> Result<()> {
        // no op by default
        Ok(())
    }

    /// Returns configuration which is used to package submission on tmc-server.
    fn get_exercise_packaging_configuration(
        &self,
        path: &Path,
    ) -> Result<ExercisePackagingConfiguration> {
        let configuration = TmcProjectYml::from(path)?;
        let extra_student_files = configuration.extra_student_files;
        let extra_test_files = configuration.extra_exercise_files;

        let student_files = self
            .get_default_student_file_paths()
            .into_iter()
            .chain(extra_student_files)
            .collect::<HashSet<_>>();
        let exercise_files_without_student_files = self
            .get_default_exercise_file_paths()
            .into_iter()
            .chain(extra_test_files)
            .filter(|e| !student_files.contains(e))
            .collect();
        Ok(ExercisePackagingConfiguration::new(
            student_files,
            exercise_files_without_student_files,
        ))
    }

    /// Runs clean command e.g `make clean` for make or `mvn clean` for maven.
    fn clean(&self, path: &Path) -> Result<()>;

    fn get_default_student_file_paths(&self) -> Vec<PathBuf> {
        vec![PathBuf::from("src")]
    }

    fn get_default_exercise_file_paths(&self) -> Vec<PathBuf> {
        vec![PathBuf::from("test")]
    }
}

#[cfg(test)]
mod test {
    use super::*;

    struct MockPlugin {}

    struct MockPolicy {}

    impl StudentFilePolicy for MockPolicy {
        fn get_config_file_parent_path(&self) -> &Path {
            Path::new("")
        }
        fn is_student_source_file(&self, _path: &Path) -> bool {
            unimplemented!()
        }
    }

    impl LanguagePlugin for MockPlugin {
        const PLUGIN_NAME: &'static str = "mock_plugin";
        type StudentFilePolicy = MockPolicy;

        fn get_student_file_policy(_project_path: &Path) -> Self::StudentFilePolicy {
            Self::StudentFilePolicy {}
        }

        fn find_project_dir_in_zip<R: Read + Seek>(
            _zip_archive: &mut ZipArchive<R>,
        ) -> Result<PathBuf> {
            todo!()
        }

        fn scan_exercise(&self, _path: &Path, _exercise_name: String) -> Result<ExerciseDesc> {
            unimplemented!()
        }

        fn run_tests_with_timeout(
            &self,
            _path: &Path,
            _timeout: Option<Duration>,
        ) -> Result<RunResult> {
            Ok(RunResult {
                status: RunStatus::Passed,
                test_results: vec![],
                logs: HashMap::new(),
            })
        }

        fn is_exercise_type_correct(path: &Path) -> bool {
            !path.to_str().unwrap().contains("ignored")
        }

        fn clean(&self, _path: &Path) -> Result<()> {
            unimplemented!()
        }
    }

    #[test]
    fn finds_exercises() {
        let plugin = MockPlugin {};
        let exercises = plugin.find_exercises(&PathBuf::from("tests/data"));
        assert!(
            exercises.contains(&PathBuf::from("tests/data/dir")),
            "{:?} did not contain testdata/dir",
            exercises
        );
        assert!(
            exercises.contains(&PathBuf::from("tests/data/dir/inner")),
            "{:?} did not contain testdata/dir/inner",
            exercises
        );
        assert!(
            !exercises.contains(&PathBuf::from("tests/data/ignored")),
            "{:?} contained testdata/ignored",
            exercises
        );
        assert!(
            !exercises.contains(&PathBuf::from("tests/data/dir/nonbinary.java")),
            "{:?} contained testdata/dir/nonbinary.java",
            exercises
        );
    }

    #[test]
    fn gets_exercise_packaging_configuration() {
        use std::fs::File;
        use std::io::Write;

        let plugin = MockPlugin {};
        let temp = tempfile::tempdir().unwrap();
        let mut path = temp.path().to_owned();
        path.push(".tmcproject.yml");
        let mut file = File::create(&path).unwrap();
        file.write_all(
            r#"
extra_student_files:
  - test/StudentTest.java
  - test/OtherTest.java
extra_exercise_files:
  - test/SomeFile.java
  - test/OtherTest.java
"#
            .as_bytes(),
        )
        .unwrap();
        let conf = plugin
            .get_exercise_packaging_configuration(&temp.path())
            .unwrap();
        assert!(conf.student_file_paths.contains(&PathBuf::from("src")));
        assert!(conf
            .student_file_paths
            .contains(&PathBuf::from("test/StudentTest.java")));
        assert!(conf
            .student_file_paths
            .contains(&PathBuf::from("test/OtherTest.java")));
        assert!(conf.exercise_file_paths.contains(&PathBuf::from("test")));
        assert!(conf
            .exercise_file_paths
            .contains(&PathBuf::from("test/SomeFile.java")));
        assert!(!conf
            .exercise_file_paths
            .contains(&PathBuf::from("test/OtherTest.java")));
    }

    #[test]
    fn empty_run_result_is_err() {
        let plugin = MockPlugin {};
        let res = plugin.run_tests(Path::new("")).unwrap();
        assert_eq!(res.status, RunStatus::TestsFailed);
        assert_eq!(res.test_results[0].name, "Tests found test")
    }
}
