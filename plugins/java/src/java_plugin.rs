//! Common functionality for all Java plugins

use crate::{error::JavaError, CompileResult, TestCase, TestCaseStatus, TestMethod, TestRun};
use j4rs::{InvocationArg, Jvm};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use tmc_langs_framework::{
    command::TmcCommand,
    domain::{ExerciseDesc, RunResult, RunStatus, TestDesc, TestResult, ValidationResult},
    io::file_util,
    nom::{bytes, character, combinator, sequence, IResult},
    plugin::{Language, LanguagePlugin},
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
        file_util::remove_file(&test_result.test_results)?;
        Ok(result?)
    }

    /// Parses test results.
    fn parse_test_result(&self, results: &TestRun) -> Result<RunResult, JavaError> {
        let result_file = file_util::open_file(&results.test_results)?;
        let test_case_records: Vec<TestCase> = serde_json::from_reader(&result_file)?;

        let mut test_results: Vec<TestResult> = vec![];
        let mut status = RunStatus::Passed;
        for test_case in test_case_records {
            if test_case.status == TestCaseStatus::Failed {
                status = RunStatus::TestsFailed;
            }
            test_results.push(self.convert_test_case_result(test_case));
        }

        let mut logs = HashMap::new();
        logs.insert(
            "stdout".to_string(),
            String::from_utf8_lossy(&results.stdout).into_owned(),
        );
        logs.insert(
            "stderr".to_string(),
            String::from_utf8_lossy(&results.stderr).into_owned(),
        );
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
        let output = TmcCommand::new_with_file_io("java")?
            .with(|e| e.arg("-XshowSettings:properties").arg("-version"))
            .output()?;

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
        if !Self::is_exercise_type_correct(path) {
            return Err(JavaError::InvalidExercise(path.to_path_buf()));
        } else if !compile_result.status_code.success() {
            return Err(JavaError::Compilation {
                stdout: String::from_utf8_lossy(&compile_result.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&compile_result.stderr).into_owned(),
            });
        }

        let mut source_files = vec![];
        for entry in WalkDir::new(path.join(Self::TEST_DIR)) {
            let entry = entry?;
            let ext = entry.path().extension();
            if ext == Some(OsStr::new("java")) || ext == Some(OsStr::new("jar")) {
                source_files.push(entry.into_path());
            }
        }
        let class_path = self.get_project_class_path(path)?;

        log::info!("Class path: {}", class_path);
        log::info!("Source files: {:?}", source_files);

        let test_scanner = self
            .jvm()
            .create_instance("fi.helsinki.cs.tmc.testscanner.TestScanner", &[])?;

        self.jvm().invoke(
            &test_scanner,
            "setClassPath",
            &[InvocationArg::try_from(class_path)?],
        )?;

        for source_file in source_files {
            let file = self.jvm().create_instance(
                "java.io.File",
                &[InvocationArg::try_from(&*source_file.to_string_lossy())?],
            )?;
            self.jvm()
                .invoke(&test_scanner, "addSource", &[InvocationArg::from(file)])?;
        }
        let scan_results = self.jvm().invoke(&test_scanner, "findTests", &[])?;
        self.jvm().invoke(&test_scanner, "clearSources", &[])?;

        let scan_results: Vec<TestMethod> = self.jvm().to_rust(scan_results)?;

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
        logs.insert(
            "stdout".to_string(),
            String::from_utf8_lossy(&compile_result.stdout).into_owned(),
        );
        logs.insert(
            "stderr".to_string(),
            String::from_utf8_lossy(&compile_result.stderr).into_owned(),
        );
        RunResult {
            status: RunStatus::CompileFailed,
            test_results: vec![],
            logs,
        }
    }

    /// Runs checkstyle.
    fn run_checkstyle(
        &self,
        locale: &Language,
        path: &Path,
    ) -> Result<ValidationResult, JavaError> {
        let path = path.to_string_lossy();
        let file = self
            .jvm()
            .create_instance("java.io.File", &[InvocationArg::try_from(path.as_ref())?])?;
        let locale_code = locale.to_639_1().unwrap_or_else(|| locale.to_639_3()); // Java requires 639-1 if one exists
        let locale = self
            .jvm()
            .create_instance("java.util.Locale", &[InvocationArg::try_from(locale_code)?])?;
        let checkstyle_runner = self.jvm().create_instance(
            "fi.helsinki.cs.tmc.stylerunner.CheckstyleRunner",
            &[InvocationArg::from(file), InvocationArg::from(locale)],
        )?;
        let result = self.jvm().invoke(&checkstyle_runner, "run", &[])?;
        let result: ValidationResult = self.jvm().to_rust(result)?;

        log::debug!("Validation result: {:?}", result);
        Ok(result)
    }

    fn java_points_parser<'a>(i: &'a str) -> IResult<&'a str, &'a str> {
        combinator::map(
            sequence::delimited(
                sequence::tuple((
                    bytes::complete::tag("@"),
                    character::complete::multispace0,
                    bytes::complete::tag_no_case("points"),
                    character::complete::multispace0,
                    character::complete::char('('),
                    character::complete::multispace0,
                )),
                sequence::delimited(
                    character::complete::char('"'),
                    bytes::complete::is_not("\""),
                    character::complete::char('"'),
                ),
                sequence::tuple((
                    character::complete::multispace0,
                    character::complete::char(')'),
                )),
            ),
            str::trim,
        )(i)
    }
}

#[cfg(test)]
mod test {
    // TODO: look into not having to use AntPlugin
    use super::super::ant_plugin::AntPlugin;
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
