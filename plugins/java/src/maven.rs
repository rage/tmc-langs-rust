//! Java maven plugin

pub mod policy;

use crate::{error::JavaError, plugin::JavaPlugin, CompileResult, TestRun, SEPARATOR};
use flate2::read::GzDecoder;
use j4rs::Jvm;
use policy::MavenStudentFilePolicy;
use std::ffi::OsString;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tar::Archive;
use tmc_langs_framework::{
    command::TmcCommand,
    domain::{ExerciseDesc, RunResult, ValidationResult},
    io::file_util,
    plugin::{Language, LanguagePlugin},
    TmcError,
};

const MVN_ARCHIVE: &[u8] = include_bytes!("../apache-maven-3.6.3-bin.tar.gz");

pub struct MavenPlugin {
    jvm: Jvm,
}

impl MavenPlugin {
    pub fn new() -> Result<Self, JavaError> {
        let jvm = crate::instantiate_jvm()?;
        Ok(Self { jvm })
    }

    // check if mvn is in PATH, if yes return mvn
    // if not, check if the bundled maven has been extracted already,
    // if not, extract
    // finally, return the path to the extracted executable
    fn get_mvn_command() -> Result<OsString, JavaError> {
        // check if mvn is in PATH
        if let Ok(status) = TmcCommand::new("mvn".to_string())
            .arg("--version")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .status()
        {
            if status.success() {
                return Ok(OsString::from("mvn"));
            }
        }
        log::debug!("could not execute mvn");
        let tmc_path = dirs::cache_dir().ok_or(JavaError::CacheDir)?.join("tmc");

        #[cfg(windows)]
        let mvn_exec = "mvn.cmd";
        #[cfg(not(windows))]
        let mvn_exec = "mvn";

        let mvn_exec_path = tmc_path
            .join("apache-maven-3.6.3")
            .join("bin")
            .join(mvn_exec);
        if !mvn_exec_path.exists() {
            log::debug!("extracting bundled tar");
            let tar = GzDecoder::new(Cursor::new(MVN_ARCHIVE));
            let mut tar = Archive::new(tar);
            tar.unpack(&tmc_path)
                .map_err(|e| JavaError::JarWrite(tmc_path, e))?;
        }
        Ok(mvn_exec_path.as_os_str().to_os_string())
    }
}

impl LanguagePlugin for MavenPlugin {
    const PLUGIN_NAME: &'static str = "apache-maven";
    type StudentFilePolicy = MavenStudentFilePolicy;

    fn check_code_style(
        &self,
        path: &Path,
        locale: Language,
    ) -> Result<Option<ValidationResult>, TmcError> {
        Ok(Some(self.run_checkstyle(&locale, path)?))
    }

    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc, TmcError> {
        if !Self::is_exercise_type_correct(path) {
            return JavaError::InvalidExercise(path.to_path_buf()).into();
        }

        let compile_result = self.build(path)?;
        Ok(self.scan_exercise_with_compile_result(path, exercise_name, compile_result)?)
    }

    fn run_tests_with_timeout(
        &self,
        project_root_path: &Path,
        _timeout: Option<Duration>,
    ) -> Result<RunResult, TmcError> {
        Ok(self.run_java_tests(project_root_path)?)
    }

    /// Checks if the directory has a pom.xml file.
    fn is_exercise_type_correct(path: &Path) -> bool {
        path.join("pom.xml").exists()
    }

    fn get_student_file_policy(project_path: &Path) -> Self::StudentFilePolicy {
        MavenStudentFilePolicy::new(project_path.to_path_buf())
    }

    fn clean(&self, path: &Path) -> Result<(), TmcError> {
        log::info!("Cleaning maven project at {}", path.display());

        let mvn_command = Self::get_mvn_command()?;
        let mut command = TmcCommand::named("maven", mvn_command);
        command.current_dir(path).arg("clean");
        command.output_checked()?;

        Ok(())
    }

    fn get_default_student_file_paths(&self) -> Vec<PathBuf> {
        vec![PathBuf::from("src/main")]
    }

    fn get_default_exercise_file_paths(&self) -> Vec<PathBuf> {
        vec![PathBuf::from("src/test")]
    }
}

impl JavaPlugin for MavenPlugin {
    const TEST_DIR: &'static str = "src";

    fn jvm(&self) -> &Jvm {
        &self.jvm
    }

    fn get_project_class_path(&self, path: &Path) -> Result<String, JavaError> {
        log::info!("Building classpath for maven project at {}", path.display());

        let temp = tempfile::tempdir().map_err(JavaError::TempDir)?;
        let class_path_file = temp.path().join("cp.txt");

        let output_arg = format!("-Dmdep.outputFile={}", class_path_file.display());
        let mvn_path = Self::get_mvn_command()?;
        let mut command = TmcCommand::named("maven", &mvn_path);
        command
            .current_dir(path)
            .arg("dependency:build-classpath")
            .arg(output_arg);
        command.output_checked()?;

        let class_path = file_util::read_file_to_string(&class_path_file)?;
        if class_path.is_empty() {
            return Err(JavaError::NoMvnClassPath);
        }

        let mut class_path: Vec<String> = vec![class_path];
        class_path.push(path.join("target/classes").to_string_lossy().into_owned());
        class_path.push(
            path.join("target/test-classes")
                .to_string_lossy()
                .into_owned(),
        );

        Ok(class_path.join(SEPARATOR))
    }

    fn build(&self, project_root_path: &Path) -> Result<CompileResult, JavaError> {
        log::info!("Building maven project at {}", project_root_path.display());

        let mvn_path = Self::get_mvn_command()?;
        let mut command = TmcCommand::named("maven", &mvn_path);
        command
            .current_dir(project_root_path)
            .arg("clean")
            .arg("compile")
            .arg("test-compile");
        let output = command.output_checked()?;

        Ok(CompileResult {
            status_code: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    fn create_run_result_file(
        &self,
        path: &Path,
        _compile_result: CompileResult,
    ) -> Result<TestRun, JavaError> {
        log::info!("Running tests for maven project at {}", path.display());

        let mvn_path = Self::get_mvn_command()?;
        let mut command = TmcCommand::named("maven", &mvn_path);
        command
            .current_dir(path)
            .arg("fi.helsinki.cs.tmc:tmc-maven-plugin:1.12:test");
        let output = command.output_checked()?;

        Ok(TestRun {
            test_results: path.join("target/test_output.txt"),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

#[cfg(test)]
#[cfg(not(target_os = "macos"))] // issues with maven dependencies
mod test {
    use super::super::{TestCase, TestCaseStatus};
    use super::*;
    use std::fs::{self, File};
    use tempfile::{tempdir, TempDir};
    use tmc_langs_framework::domain::Strategy;
    use tmc_langs_framework::zip::ZipArchive;
    use walkdir::WalkDir;

    #[cfg(windows)]
    use std::sync::Once;
    #[cfg(windows)]
    static INIT_MAVEN: Once = Once::new();

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();

        // initializes maven in a synchronized manner for all tests
        #[cfg(windows)]
        INIT_MAVEN.call_once(|| {
            MavenPlugin::new().expect("failed to instantiate maven");
        });
    }

    fn copy_test_dir(path: &str) -> TempDir {
        let path = Path::new(path);

        let temp = tempdir().unwrap();
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
    fn gets_project_class_path() {
        init();

        let temp_dir = copy_test_dir("tests/data/maven_exercise");
        let test_path = temp_dir.path();
        let plugin = MavenPlugin::new().unwrap();
        let class_path = plugin.get_project_class_path(test_path).unwrap();
        log::debug!("{}", class_path);
        let expected = format!("{0}junit{0}", std::path::MAIN_SEPARATOR);
        assert!(class_path.contains(&expected));
    }

    #[test]
    fn builds() {
        init();

        use std::path::PathBuf;
        log::debug!("{}", PathBuf::from(".").canonicalize().unwrap().display());

        let temp_dir = copy_test_dir("tests/data/maven_exercise");
        let test_path = temp_dir.path();
        let plugin = MavenPlugin::new().unwrap();
        let compile_result = plugin.build(test_path).unwrap();
        assert!(compile_result.status_code.success());
    }

    #[test]
    fn creates_run_result_file() {
        init();

        let temp_dir = copy_test_dir("tests/data/maven_exercise");
        let test_path = temp_dir.path();
        let plugin = MavenPlugin::new().unwrap();
        let compile_result = plugin.build(test_path).unwrap();
        let test_run = plugin
            .create_run_result_file(test_path, compile_result)
            .unwrap();
        let test_result: Vec<TestCase> =
            serde_json::from_str(&fs::read_to_string(test_run.test_results).unwrap()).unwrap();
        let test_case = &test_result[0];

        assert_eq!(test_case.class_name, "fi.helsinki.cs.maventest.AppTest");
        assert_eq!(test_case.point_names, ["maven-exercise"]);
        assert_eq!(test_case.status, TestCaseStatus::Failed);
        let message = test_case.message.as_ref().unwrap();
        assert!(message.starts_with("ComparisonFailure"));

        let exception = test_case.exception.as_ref().unwrap();
        assert_eq!(exception.class_name, "org.junit.ComparisonFailure");
        assert!(exception.message.as_ref().unwrap().starts_with("expected"));
        let stack_trace = &exception.stack_trace[0];
        assert_eq!(stack_trace.declaring_class, "org.junit.Assert");
        assert_eq!(stack_trace.file_name.as_ref().unwrap(), "Assert.java");
        assert_eq!(stack_trace.line_number, 115);
        assert_eq!(stack_trace.method_name, "assertEquals");
    }

    #[test]
    fn scans_exercise() {
        init();

        let temp_dir = copy_test_dir("tests/data/maven_exercise");
        let test_path = temp_dir.path();
        let plugin = MavenPlugin::new().unwrap();
        let exercises = plugin
            .scan_exercise(&test_path, "test".to_string())
            .unwrap();
        assert_eq!(exercises.name, "test");
        assert_eq!(exercises.tests.len(), 1);
        assert_eq!(
            exercises.tests[0].name,
            "fi.helsinki.cs.maventest.AppTest trol"
        );
        assert_eq!(exercises.tests[0].points, ["maven-exercise"]);
    }

    #[test]
    fn runs_checkstyle() {
        init();

        let temp_dir = copy_test_dir("tests/data/maven_exercise");
        let test_path = temp_dir.path();
        let plugin = MavenPlugin::new().unwrap();
        let checkstyle_result = plugin
            .check_code_style(test_path, Language::from_639_3("fin").unwrap())
            .unwrap()
            .unwrap();

        assert_eq!(checkstyle_result.strategy, Strategy::Fail);
        let validation_errors = checkstyle_result.validation_errors.unwrap();
        let errors = validation_errors
            .get(Path::new("fi/helsinki/cs/maventest/App.java"))
            .unwrap();
        assert_eq!(errors.len(), 1);
        let error = &errors[0];
        assert_eq!(error.column, 0);
        assert_eq!(error.line, 4);
        assert!(error.message.starts_with("Sisennys väärin"));
        assert_eq!(
            error.source_name,
            "com.puppycrawl.tools.checkstyle.checks.indentation.IndentationCheck"
        );
    }

    // TODO: currently will extract maven to your cache directory
    // #[test] TODO: changing PATH breaks other tests
    fn _unpack_bundled_mvn() {
        std::env::set_var("PATH", "");
        let cmd = MavenPlugin::get_mvn_command().unwrap();
        let expected = format!(
            "tmc{0}apache-maven-3.6.3{0}bin{0}mvn",
            std::path::MAIN_SEPARATOR
        );
        assert!(cmd.to_string_lossy().ends_with(&expected))
    }

    #[test]
    fn finds_project_dir_in_zip() {
        let file = File::open("tests/data/MavenProject.zip").unwrap();
        let mut zip = ZipArchive::new(file).unwrap();
        let dir = MavenPlugin::find_project_dir_in_zip(&mut zip).unwrap();
        assert_eq!(dir, Path::new("Outer/Inner/maven_exercise"));
    }

    #[test]
    fn doesnt_find_project_dir_in_zip() {
        let file = File::open("tests/data/MavenWithoutSrc.zip").unwrap();
        let mut zip = ZipArchive::new(file).unwrap();
        let dir = MavenPlugin::find_project_dir_in_zip(&mut zip);
        assert!(dir.is_err());
    }
}
