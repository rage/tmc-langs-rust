//! Java ant plugin

pub mod policy;

use super::{error::JavaError, plugin::JavaPlugin, CompileResult, TestRun, SEPARATOR};

use isolang::Language;
use j4rs::Jvm;
use policy::AntStudentFilePolicy;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use tmc_langs_framework::{
    domain::{ExerciseDesc, RunResult},
    plugin::{LanguagePlugin, ValidationResult},
    policy::StudentFilePolicy,
    Error,
};
use walkdir::WalkDir;

const BUILD_FILE_NAME: &str = "build.xml";

pub struct AntPlugin {
    jvm: Jvm,
}

impl AntPlugin {
    pub fn new() -> Result<Self, JavaError> {
        let jvm = crate::instantiate_jvm()?;
        Ok(Self { jvm })
    }

    fn copy_tmc_junit_runner(&self, path: &Path) -> Result<(), JavaError> {
        log::debug!("Copying TMC Junit runner");

        let local_tmc_junit_runner = Path::new("./jars/tmc-junit-runner-0.2.8.jar");
        let runner_dir = path.join("lib/testrunner");
        let runner_path = runner_dir.join("tmc-junit-runner.jar");

        // TODO: don't traverse symlinks
        if !runner_path.exists() {
            fs::create_dir_all(&runner_dir).map_err(|e| JavaError::Dir(runner_dir, e))?;
            log::debug!(
                "copying from {} to {}",
                local_tmc_junit_runner.display(),
                runner_path.display()
            );
            fs::copy(local_tmc_junit_runner, &runner_path).map_err(|e| {
                JavaError::FileCopy(local_tmc_junit_runner.to_path_buf(), runner_path, e)
            })?;
        } else {
            log::debug!("already exists");
        }
        Ok(())
    }
}

impl LanguagePlugin for AntPlugin {
    fn get_plugin_name(&self) -> &str {
        "apache-ant"
    }

    fn check_code_style(&self, path: &Path, locale: Language) -> Option<ValidationResult> {
        self.run_checkstyle(&locale, path)
    }

    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc, Error> {
        if !self.is_exercise_type_correct(path) {
            return JavaError::InvalidExercise.into();
        }

        let compile_result = self.build(path)?;
        Ok(self.scan_exercise_with_compile_result(path, exercise_name, compile_result)?)
    }

    fn run_tests_with_timeout(
        &self,
        project_root_path: &Path,
        _timeout: Option<Duration>,
    ) -> Result<RunResult, Error> {
        Ok(self.run_java_tests(project_root_path)?)
    }

    fn is_exercise_type_correct(&self, path: &Path) -> bool {
        path.join(BUILD_FILE_NAME).exists()
            || path.join("test").exists() && path.join("src").exists()
    }

    fn get_student_file_policy(&self, project_path: &Path) -> Box<dyn StudentFilePolicy> {
        Box::new(AntStudentFilePolicy::new(project_path.to_path_buf()))
    }

    fn maybe_copy_shared_stuff(&self, dest_path: &Path) -> Result<(), Error> {
        Ok(self.copy_tmc_junit_runner(dest_path)?)
    }

    fn clean(&self, path: &Path) -> Result<(), Error> {
        log::debug!("Cleaning project at {}", path.display());

        let stdout_path = path.join("build_log.txt");
        let stdout =
            File::create(&stdout_path).map_err(|e| JavaError::File(stdout_path.clone(), e))?;
        let stderr_path = path.join("build_errors.txt");
        let stderr =
            File::create(&stderr_path).map_err(|e| JavaError::File(stderr_path.clone(), e))?;

        let output = Command::new("ant")
            .arg("clean")
            .stdout(stdout)
            .stderr(stderr)
            .current_dir(path)
            .output()
            .map_err(|e| JavaError::FailedToRun("ant", e))?;

        if output.status.success() {
            fs::remove_file(stdout_path)?;
            fs::remove_file(stderr_path)?;
            Ok(())
        } else {
            Err(Error::CommandFailed("ant clean"))
        }
    }
}

impl JavaPlugin for AntPlugin {
    const TEST_DIR: &'static str = "test";

    fn jvm(&self) -> &Jvm {
        &self.jvm
    }

    fn get_project_class_path(&self, path: &Path) -> Result<String, JavaError> {
        let mut paths = vec![];

        // add all .jar files in lib
        let lib_dir = path.join("lib");
        for entry in WalkDir::new(&lib_dir).into_iter().filter_map(|e| e.ok()) {
            if entry.path().is_file() && entry.path().extension().unwrap_or_default() == "jar" {
                paths.push(entry.path().to_path_buf());
            }
        }
        paths.push(lib_dir);

        paths.push(path.join("build/test/classes"));
        paths.push(path.join("build/classes"));
        let java_home = Self::get_java_home()?;
        paths.push(java_home.join("../lib/tools.jar"));
        let paths = paths
            .into_iter()
            .map(|p| p.into_os_string().to_str().map(|s| s.to_string()))
            .filter_map(|p| p)
            .collect::<Vec<_>>();

        self.copy_tmc_junit_runner(path)?; // ?
        Ok(paths.join(SEPARATOR))
    }

    fn build(&self, project_root_path: &Path) -> Result<CompileResult, JavaError> {
        log::info!("Building project at {}", project_root_path.display());

        let stdout_path = project_root_path.join("build_log.txt");
        let mut stdout =
            File::create(&stdout_path).map_err(|e| JavaError::File(stdout_path.clone(), e))?;
        let stderr_path = project_root_path.join("build_errors.txt");
        let mut stderr =
            File::create(&stderr_path).map_err(|e| JavaError::File(stderr_path.clone(), e))?;

        // TODO: don't require ant in path?
        let output = Command::new("ant")
            .arg("compile-test")
            .current_dir(project_root_path)
            .output()
            .map_err(|e| JavaError::FailedToRun("ant", e))?;

        log::debug!(
            "Writing stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        log::debug!(
            "Writing stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        stdout
            .write_all(&output.stdout)
            .map_err(|e| JavaError::File(stdout_path, e))?;
        stderr
            .write_all(&output.stderr)
            .map_err(|e| JavaError::File(stderr_path, e))?;

        Ok(CompileResult {
            status_code: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    fn create_run_result_file(
        &self,
        path: &Path,
        compile_result: CompileResult,
    ) -> Result<TestRun, JavaError> {
        log::info!("Running tests for project at {}", path.display());

        let exercise = self.scan_exercise_with_compile_result(
            path,
            format!("{}{}", path.display().to_string(), "/test"), // ?
            compile_result,
        )?;

        let test_dir = path.join("test");
        let result_file = path.join("results.txt");
        let class_path = self.get_project_class_path(path)?;

        let mut arguments = vec![];
        if let Ok(jvm_options) = env::var("JVM_OPTIONS") {
            arguments.extend(
                jvm_options
                    .split(" +")
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string()),
            )
        }
        arguments.push(format!("-Dtmc.test_class_dir={}", test_dir.display()));
        arguments.push(format!("-Dtmc.results_file={}", result_file.display()));
        let endorsed_libs_path = path.join("lib/endorsed");
        if endorsed_libs_path.exists() {
            arguments.push(format!(
                "-Djava.endorsed.dirs={}",
                endorsed_libs_path.display()
            ));
        }
        arguments.push("-cp".to_string());
        arguments.push(class_path);
        arguments.push("fi.helsinki.cs.tmc.testrunner.Main".to_string());
        for desc in exercise.tests {
            let mut s = String::new();
            s.push_str(&desc.name.replace(' ', "."));
            s.push('{');
            s.push_str(&desc.points.join(","));
            s.push('}');
            arguments.push(s);
        }

        log::debug!("java args {} in {}", arguments.join(" "), path.display());
        let command = Command::new("java")
            .current_dir(path)
            .args(arguments.join(" ").split(' ').collect::<Vec<&str>>())
            .output()
            .map_err(|e| JavaError::FailedToRun("java", e))?;

        Ok(TestRun {
            test_results: result_file,
            stdout: command.stdout,
            stderr: command.stderr,
        })
    }
}

#[cfg(test)]
#[cfg(not(target_os = "macos"))] // ant is not installed on github's macos-latest image
mod test {
    use super::*;
    use tempfile::{tempdir, TempDir};
    use tmc_langs_framework::plugin::Strategy;

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    fn copy_test_dir(path: &str) -> TempDir {
        let path = Path::new(path);

        let temp = tempdir().unwrap();
        for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
            let target = temp.path().join(entry.path().strip_prefix(path).unwrap());
            if entry.path().is_dir() {
                log::debug!("creating dirs {}", entry.path().display());
                fs::create_dir_all(target).unwrap();
            } else {
                log::debug!(
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

        let temp_dir = copy_test_dir("tests/data/ant_project");
        let test_path = temp_dir.path();
        let plugin = AntPlugin::new().unwrap();
        let cp = plugin.get_project_class_path(test_path).unwrap();
        assert!(
            cp.contains(&format!("{0}/lib/junit-4.10.jar", test_path.display())),
            "Classpath {} did not contain junit",
            cp
        );
        assert!(
            cp.contains(&format!(
                "{0}/lib/edu-test-utils-0.4.1.jar",
                test_path.display()
            )),
            "Classpath {} did not contain edu-test-utils",
            cp
        );
        assert!(
            cp.contains(&format!("{0}/build/classes", test_path.display())),
            "Classpath {} did not contain build/classes",
            cp
        );
        assert!(
            cp.contains(&format!("{0}/build/test/classes", test_path.display())),
            "Classpath {} did not contain build/test/classes",
            cp
        );
        assert!(cp.ends_with("/../lib/tools.jar",), "Classpath was {}", cp);
    }

    #[test]
    fn builds() {
        init();

        let temp_dir = copy_test_dir("tests/data/ant_project");
        let test_path = temp_dir.path();
        let plugin = AntPlugin::new().unwrap();
        let compile_result = plugin.build(test_path).unwrap();
        assert!(compile_result.status_code.success());
        assert!(!compile_result.stdout.is_empty());
        assert!(compile_result.stderr.is_empty());
    }

    #[test]
    fn creates_run_result_file() {
        init();

        let temp_dir = copy_test_dir("tests/data/ant_project");
        let test_path = temp_dir.path();
        let plugin = AntPlugin::new().unwrap();
        let compile_result = plugin.build(test_path).unwrap();
        let test_run = plugin
            .create_run_result_file(test_path, compile_result)
            .unwrap();
        log::debug!("stdout: {}", String::from_utf8_lossy(&test_run.stdout));
        log::debug!("stderr: {}", String::from_utf8_lossy(&test_run.stderr));
        assert!(test_run.stdout.is_empty());
        assert!(test_run.stderr.is_empty());
        let res = fs::read_to_string(test_run.test_results).unwrap();
        let test_cases: Vec<super::super::TestCase> = serde_json::from_str(&res).unwrap();

        let test_case = &test_cases[0];
        assert_eq!(test_case.class_name, "ArithTest");
        assert_eq!(test_case.method_name, "testAdd");
        assert_eq!(test_case.status, super::super::TestCaseStatus::Passed);
        assert_eq!(test_case.point_names[0], "arith-funcs");
        assert!(test_case.message.is_none());
        assert!(test_case.exception.is_none());

        let test_case = &test_cases[1];
        assert_eq!(test_case.class_name, "ArithTest");
        assert_eq!(test_case.method_name, "testSub");
        assert_eq!(test_case.status, super::super::TestCaseStatus::Failed);
        assert_eq!(test_case.point_names[0], "arith-funcs");
        assert!(test_case.message.as_ref().unwrap().starts_with("expected:"));

        let exception = test_case.exception.as_ref().unwrap();
        assert_eq!(exception.class_name, "java.lang.AssertionError");
        assert!(exception.message.starts_with("expected:"));
        assert!(exception.cause.is_none());

        let stack_trace = &exception.stack_trace[0];
        assert_eq!(stack_trace.declaring_class, "org.junit.Assert");
        assert_eq!(stack_trace.file_name, "Assert.java");
        assert_eq!(stack_trace.method_name, "fail");
    }

    #[test]
    fn scans_exercise() {
        init();

        let temp_dir = copy_test_dir("tests/data/ant_project");
        let test_path = temp_dir.path();
        let plugin = AntPlugin::new().unwrap();
        let exercises = plugin
            .scan_exercise(&test_path, "test".to_string())
            .unwrap();
        assert_eq!(exercises.name, "test");
        assert_eq!(exercises.tests.len(), 4);
        assert_eq!(exercises.tests[0].name, "ArithTest testAdd");
        assert_eq!(exercises.tests[0].points, ["arith-funcs"]);
    }

    #[test]
    fn runs_checkstyle() {
        init();

        let temp_dir = copy_test_dir("tests/data/ant_project");
        let test_path = temp_dir.path();
        let plugin = AntPlugin::new().unwrap();
        let checkstyle_result = plugin
            .check_code_style(test_path, Language::from_639_3("fin").unwrap())
            .unwrap();

        assert_eq!(checkstyle_result.strategy, Strategy::Fail);
        let validation_errors = checkstyle_result.validation_errors.unwrap();
        let errors = validation_errors.get(Path::new("Arith.java")).unwrap();
        assert_eq!(errors.len(), 1);
        let error = &errors[0];
        assert_eq!(error.column, 0);
        assert_eq!(error.line, 7);
        assert!(error.message.starts_with("Sisennys väärin"));
        assert_eq!(
            error.source_name,
            "com.puppycrawl.tools.checkstyle.checks.indentation.IndentationCheck"
        );
    }

    #[test]
    fn cleans() {
        init();

        let temp_dir = copy_test_dir("tests/data/ant_project");
        let test_path = temp_dir.path();
        let plugin = AntPlugin::new().unwrap();
        plugin.clean(test_path).unwrap();
    }
}
