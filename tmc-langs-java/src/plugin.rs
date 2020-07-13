//! Common functionality for all Java plugins

use super::{error::JavaError, CompileResult, TestCase, TestCaseStatus, TestMethod, TestRun};

use j4rs::{InvocationArg, Jvm};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tmc_langs_framework::{
    domain::{ExerciseDesc, RunResult, RunStatus, TestDesc, TestResult},
    plugin::{Language, LanguagePlugin, ValidationResult},
};
use walkdir::WalkDir;

pub(crate) trait JavaPlugin: LanguagePlugin {
    const TEST_DIR: &'static str;

    /// Returns a reference to the inner Jvm.
    fn jvm(&self) -> &Jvm;

    /// Constructs a CLASSPATH for the given path (see https://docs.oracle.com/javase/tutorial/essential/environment/paths.html).
    fn get_project_class_path(&self, path: &Path) -> Result<String, JavaError>;

    /// Builds the Java project.
    fn build(&self, project_root_path: &Path) -> Result<CompileResult, JavaError>;

    /// Runs the tests for the given project.
    fn run_java_tests(&self, project_root_path: &Path) -> Result<RunResult, JavaError> {
        log::info!(
            "Running tests for project at {}",
            project_root_path.display()
        );

        let compile_result = self.build(project_root_path)?;
        if !compile_result.status_code.success() {
            return Ok(self.run_result_from_failed_compilation(compile_result));
        }

        let test_result = self.create_run_result_file(project_root_path, compile_result)?;
        let result = self.parse_test_result(&test_result);
        fs::remove_file(&test_result.test_results)
            .map_err(|e| JavaError::File(test_result.test_results, e))?;
        Ok(result?)
    }

    /// Parses test results.
    fn parse_test_result(&self, results: &TestRun) -> Result<RunResult, JavaError> {
        let json = fs::read_to_string(&results.test_results)
            .map_err(|e| JavaError::File(results.test_results.to_owned(), e))?;

        let mut test_results: Vec<TestResult> = vec![];
        let test_case_records: Vec<TestCase> = serde_json::from_str(&json)?;

        let mut status = RunStatus::Passed;
        for test_case in test_case_records {
            if test_case.status == TestCaseStatus::Failed {
                status = RunStatus::TestsFailed;
            }
            test_results.push(self.convert_test_case_result(test_case));
        }

        let mut logs = HashMap::new();
        logs.insert("stdout".to_string(), results.stdout.clone());
        logs.insert("stderr".to_string(), results.stderr.clone());
        Ok(RunResult {
            status,
            test_results,
            logs,
        })
    }

    /// Converts a Java test case into a tmc-langs test result.
    fn convert_test_case_result(&self, test_case: TestCase) -> TestResult {
        let mut exceptions = vec![];
        let mut points = vec![];

        if let Some(exception) = test_case.exception {
            if let Some(message) = exception.message {
                exceptions.push(message);
            }
            for stack_trace in exception.stack_trace {
                exceptions.push(stack_trace.to_string())
            }
        }

        points.extend(test_case.point_names);

        let name = format!("{} {}", test_case.class_name, test_case.method_name);
        let successful = test_case.status == TestCaseStatus::Passed;
        let message = test_case.message.unwrap_or_default();

        TestResult {
            name,
            successful,
            points,
            message,
            exception: exceptions,
        }
    }

    /// Tries to parse the java.home property.
    fn parse_java_home(properties: &str) -> Option<PathBuf> {
        for line in properties.lines() {
            if line.contains("java.home") {
                return line.split('=').nth(1).map(|s| PathBuf::from(s.trim()));
            }
        }

        log::warn!("No java.home found in {}", properties);
        None
    }

    /// Tries to find the java.home property.
    fn get_java_home() -> Result<PathBuf, JavaError> {
        let output = Command::new("java")
            .arg("-XshowSettings:properties")
            .arg("-version")
            .output()
            .map_err(|e| JavaError::FailedToRun("java".to_string(), e))?;

        if !output.status.success() {
            return Err(JavaError::FailedCommand(
                "java".to_string(),
                output.stdout,
                output.stderr,
            ));
        }

        // information is printed to stderr
        let stderr = String::from_utf8_lossy(&output.stderr);

        match Self::parse_java_home(&stderr) {
            Some(java_home) => Ok(java_home),
            None => Err(JavaError::NoJavaHome),
        }
    }

    /// Runs tests and writes the results into a file.
    fn create_run_result_file(
        &self,
        path: &Path,
        compile_result: CompileResult,
    ) -> Result<TestRun, JavaError>;

    /// Checks the compile result and scans an exercise.
    fn scan_exercise_with_compile_result(
        &self,
        path: &Path,
        exercise_name: String,
        compile_result: CompileResult,
    ) -> Result<ExerciseDesc, JavaError> {
        if !Self::is_exercise_type_correct(path) || !compile_result.status_code.success() {
            return Err(JavaError::InvalidExercise);
        }

        let mut source_files = vec![];
        for entry in WalkDir::new(path.join(Self::TEST_DIR))
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let ext = entry.path().extension();
            if ext.map_or(false, |o| o == "java" || o == "jar") {
                source_files.push(entry.into_path());
            }
        }
        let class_path = self.get_project_class_path(path)?;

        log::info!("Class path: {}", class_path);
        log::info!("Source files: {:?}", source_files);

        let test_scanner = self
            .jvm()
            .create_instance("fi.helsinki.cs.tmc.testscanner.TestScanner", &[])
            .expect("failed to instantiate");

        self.jvm()
            .invoke(
                &test_scanner,
                "setClassPath",
                &[InvocationArg::try_from(class_path).expect("failed to convert")],
            )
            .expect("failed to invoke");

        for source_file in source_files {
            let file = self
                .jvm()
                .create_instance(
                    "java.io.File",
                    &[InvocationArg::try_from(&*source_file.to_string_lossy())
                        .expect("failed to convert")],
                )
                .expect("failed to instantiate");
            self.jvm()
                .invoke(
                    &test_scanner,
                    "addSource",
                    &[InvocationArg::try_from(file).expect("failed to convert")],
                )
                .expect("failed to invoke");
        }
        let scan_results = self
            .jvm()
            .invoke(&test_scanner, "findTests", &[])
            .expect("failed to invoke");
        self.jvm()
            .invoke(&test_scanner, "clearSources", &[])
            .expect("failed to invoke");

        let scan_results: Vec<TestMethod> =
            self.jvm().to_rust(scan_results).expect("failed to convert");

        let tests = scan_results
            .into_iter()
            .map(|s| TestDesc {
                name: format!("{} {}", s.class_name, s.method_name),
                points: s.points,
            })
            .collect();

        Ok(ExerciseDesc {
            name: exercise_name,
            tests,
        })
    }

    /// Creates a run result from a failed compilation.
    fn run_result_from_failed_compilation(&self, compile_result: CompileResult) -> RunResult {
        let mut logs = HashMap::new();
        logs.insert("stdout".to_string(), compile_result.stdout);
        logs.insert("stderr".to_string(), compile_result.stderr);
        RunResult {
            status: RunStatus::CompileFailed,
            test_results: vec![],
            logs,
        }
    }

    /// Runs checkstyle.
    fn run_checkstyle(&self, locale: &Language, path: &Path) -> Option<ValidationResult> {
        let file = self
            .jvm()
            .create_instance(
                "java.io.File",
                &[InvocationArg::try_from(&*path.to_string_lossy()).expect("failed to convert")],
            )
            .expect("failed to instantiate");
        let locale_code = locale.to_639_1().unwrap_or_else(|| locale.to_639_3()); // Java requires 639-1 if one exists
        let locale = self
            .jvm()
            .create_instance(
                "java.util.Locale",
                &[InvocationArg::try_from(locale_code).expect("failed to convert")],
            )
            .expect("failed to instantiate");
        let checkstyle_runner = self
            .jvm()
            .create_instance(
                "fi.helsinki.cs.tmc.stylerunner.CheckstyleRunner",
                &[InvocationArg::from(file), InvocationArg::from(locale)],
            )
            .expect("failed to instantiate");
        let result = self
            .jvm()
            .invoke(&checkstyle_runner, "run", &[])
            .expect("failed to invoke");
        let result = self.jvm().to_rust(result).expect("failed to convert");

        log::debug!("Validation result: {:?}", result);
        result
    }
}

#[cfg(test)]
mod test {
    // TODO: look into not having to use AntPlugin
    use super::super::ant::AntPlugin;
    use super::*;

    #[test]
    fn parses_java_home() {
        let properties = r#"Property settings:
    awt.toolkit = sun.awt.X11.XToolkit
    java.ext.dirs = /usr/lib/jvm/java-8-openjdk-amd64/jre/lib/ext
        /usr/java/packages/lib/ext
    java.home = /usr/lib/jvm/java-8-openjdk-amd64/jre
    user.timezone = 

openjdk version "1.8.0_252"
"#;

        let parsed = AntPlugin::parse_java_home(properties);
        assert_eq!(
            Some(PathBuf::from("/usr/lib/jvm/java-8-openjdk-amd64/jre")),
            parsed,
        );
    }
}
