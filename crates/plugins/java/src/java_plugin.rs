//! Common functionality for all Java plugins

use crate::{
    error::JavaError, CompileResult, JvmWrapper, TestCase, TestCaseStatus, TestMethod, TestRun,
};
use j4rs::InvocationArg;
use std::{
    collections::HashMap,
    convert::TryFrom,
    ffi::OsStr,
    path::{Path, PathBuf},
    time::Duration,
};
use tmc_langs_framework::{
    nom::{bytes, character, combinator, error::VerboseError, sequence, IResult},
    ExerciseDesc, Language, LanguagePlugin, RunResult, RunStatus, StyleValidationResult, TestDesc,
    TestResult, TmcCommand,
};
use tmc_langs_util::{deserialize, file_util, parse_util};
use walkdir::WalkDir;

pub(crate) trait JavaPlugin: LanguagePlugin {
    const TEST_DIR: &'static str;

    /// Returns a reference to the inner Jvm.
    fn jvm(&self) -> &JvmWrapper;

    /// Constructs a CLASSPATH for the given path (see https://docs.oracle.com/javase/tutorial/essential/environment/paths.html).
    fn get_project_class_path(&self, path: &Path) -> Result<String, JavaError>;

    /// Builds the Java project.
    fn build(&self, project_root_path: &Path) -> Result<CompileResult, JavaError>;

    /// Runs the tests for the given project.
    fn run_java_tests(
        &self,
        project_root_path: &Path,
        timeout: Option<Duration>,
    ) -> Result<RunResult, JavaError> {
        log::info!(
            "running tests for project at {}",
            project_root_path.display()
        );

        let compile_result = self.build(project_root_path)?;
        if !compile_result.status_code.success() {
            return Ok(self.run_result_from_failed_compilation(compile_result));
        }

        let test_result =
            self.create_run_result_file(project_root_path, timeout, compile_result)?;
        let result = self.parse_test_result(&test_result);
        if let Err(err) = file_util::remove_file(&test_result.test_results) {
            log::warn!("Failed to remove test results file: {err}");
        }
        result
    }

    /// Parses test results.
    fn parse_test_result(&self, results: &TestRun) -> Result<RunResult, JavaError> {
        let result_file = file_util::open_file(&results.test_results)?;
        let test_case_records: Vec<TestCase> = deserialize::json_from_reader(&result_file)?;

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
        let output = TmcCommand::piped("java")
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
        timeout: Option<Duration>,
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

        log::info!("class path: {}", class_path);
        log::info!("source files: {:?}", source_files);

        let scan_results = self.jvm().with(|jvm| {
            let test_scanner =
                jvm.create_instance("fi.helsinki.cs.tmc.testscanner.TestScanner", &[])?;

            jvm.invoke(
                &test_scanner,
                "setClassPath",
                &[InvocationArg::try_from(class_path)?],
            )?;

            for source_file in source_files {
                let file = jvm.create_instance(
                    "java.io.File",
                    &[InvocationArg::try_from(&*source_file.to_string_lossy())?],
                )?;
                jvm.invoke(&test_scanner, "addSource", &[InvocationArg::from(file)])?;
            }
            let scan_results = jvm.invoke(&test_scanner, "findTests", &[])?;
            jvm.invoke(&test_scanner, "clearSources", &[])?;

            let scan_results: Vec<TestMethod> = jvm.to_rust(scan_results)?;
            Ok(scan_results)
        })?;

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
    ) -> Result<StyleValidationResult, JavaError> {
        let path = path.to_string_lossy();
        let result = self.jvm().with(|jvm| {
            let file =
                jvm.create_instance("java.io.File", &[InvocationArg::try_from(path.as_ref())?])?;
            let locale_code = locale.to_639_1().unwrap_or_else(|| locale.to_639_3()); // Java requires 639-1 if one exists
            let locale =
                jvm.create_instance("java.util.Locale", &[InvocationArg::try_from(locale_code)?])?;
            let checkstyle_runner = jvm.create_instance(
                "fi.helsinki.cs.tmc.stylerunner.CheckstyleRunner",
                &[InvocationArg::from(file), InvocationArg::from(locale)],
            )?;
            let result = jvm.invoke(&checkstyle_runner, "run", &[])?;
            let result: StyleValidationResult = jvm.to_rust(result)?;
            Ok(result)
        })?;

        log::debug!("Validation result: {:?}", result);
        Ok(result)
    }

    /// Parses @Points("1.1") point annotations.
    fn java_points_parser(i: &str) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
        combinator::map(
            sequence::delimited(
                sequence::tuple((
                    character::complete::char('@'),
                    character::complete::multispace0,
                    bytes::complete::tag_no_case("points"),
                    character::complete::multispace0,
                    character::complete::char('('),
                    character::complete::multispace0,
                )),
                parse_util::comma_separated_strings,
                sequence::tuple((
                    character::complete::multispace0,
                    character::complete::char(')'),
                )),
            ),
            // splits each point by whitespace
            |points| {
                points
                    .into_iter()
                    .flat_map(|p| p.split_whitespace())
                    .collect()
            },
        )(i)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use crate::SEPARATOR;
    use std::io::{Read, Seek};
    use tmc_langs_framework::{nom, Archive, TmcError};

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new()
            .with_level(LevelFilter::Debug)
            .with_module_level("j4rs", LevelFilter::Warn)
            .init();
    }

    fn dir_to_temp(source_dir: impl AsRef<std::path::Path>) -> tempfile::TempDir {
        let temp = tempfile::TempDir::new().unwrap();
        for entry in walkdir::WalkDir::new(&source_dir).min_depth(1) {
            let entry = entry.unwrap();
            let rela = entry.path().strip_prefix(&source_dir).unwrap();
            let target = temp.path().join(rela);
            if entry.path().is_dir() {
                std::fs::create_dir(target).unwrap();
            } else if entry.path().is_file() {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
        temp
    }

    struct Stub {
        jvm: JvmWrapper,
    }

    impl Stub {
        fn new() -> Self {
            Self {
                jvm: crate::instantiate_jvm().unwrap(),
            }
        }
    }

    impl LanguagePlugin for Stub {
        const PLUGIN_NAME: &'static str = "stub";
        const DEFAULT_SANDBOX_IMAGE: &'static str = "stub-image";
        const LINE_COMMENT: &'static str = "//";
        const BLOCK_COMMENT: Option<(&'static str, &'static str)> = Some(("/*", "*/"));
        type StudentFilePolicy = tmc_langs_framework::EverythingIsStudentFilePolicy;

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
            unimplemented!()
        }

        fn find_project_dir_in_archive<R: Read + Seek>(
            _archive: &mut Archive<R>,
        ) -> Result<PathBuf, TmcError> {
            unimplemented!()
        }

        fn is_exercise_type_correct(_path: &Path) -> bool {
            true
        }

        fn clean(&self, _path: &Path) -> Result<(), TmcError> {
            unimplemented!()
        }

        fn get_default_student_file_paths() -> Vec<PathBuf> {
            unimplemented!()
        }

        fn get_default_exercise_file_paths() -> Vec<PathBuf> {
            unimplemented!()
        }

        fn points_parser(i: &str) -> IResult<&str, Vec<&str>, nom::error::VerboseError<&str>> {
            Self::java_points_parser(i)
        }
    }

    impl JavaPlugin for Stub {
        const TEST_DIR: &'static str = "test";

        fn jvm(&self) -> &JvmWrapper {
            &self.jvm
        }
        fn get_project_class_path(&self, path: &Path) -> Result<String, JavaError> {
            let path = path.to_str().unwrap();
            let cp =
                format!("{path}/lib/edu-test-utils-0.4.1.jar{SEPARATOR}{path}/lib/junit-4.10.jar");
            Ok(cp)
        }

        fn build(&self, _project_root_path: &Path) -> Result<CompileResult, JavaError> {
            Ok(CompileResult {
                status_code: tmc_langs_framework::ExitStatus::Exited(0),
                stdout: vec![],
                stderr: vec![],
            })
        }

        fn create_run_result_file(
            &self,
            path: &Path,
            _timeout: Option<Duration>,
            _compile_result: CompileResult,
        ) -> Result<TestRun, JavaError> {
            let path = path.join("runresult");
            std::fs::write(
                &path,
                r#"[{
                "className": "cls1",
                "methodName": "mtd1",
                "pointNames": [],
                "status": "PASSED",
                "message": null,
                "exception": null
            },{
                "className": "cls2",
                "methodName": "mtd2",
                "pointNames": [],
                "status": "FAILED",
                "message": null,
                "exception": null
            }]"#,
            )
            .unwrap();
            Ok(TestRun {
                test_results: path,
                stdout: vec![],
                stderr: vec![],
            })
        }
    }

    #[test]
    fn runs_java_tests() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        let plugin = Stub::new();
        let result = plugin.run_java_tests(temp_dir.path(), None).unwrap();
        assert_eq!(result.status, RunStatus::TestsFailed);
    }

    #[test]
    fn parses_test_results() {
        init();

        use std::io::Write;
        let mut temp_file = tempfile::NamedTempFile::new().unwrap();
        temp_file
            .write_all(
                br#"[{
                "className": "cls1",
                "methodName": "mtd1",
                "pointNames": [],
                "status": "PASSED",
                "message": null,
                "exception": null
            },{
                "className": "cls2",
                "methodName": "mtd2",
                "pointNames": [],
                "status": "FAILED",
                "message": null,
                "exception": null
            }]"#,
            )
            .unwrap();

        let plugin = Stub::new();
        let test_run = TestRun {
            test_results: temp_file.path().to_path_buf(),
            stdout: vec![],
            stderr: vec![],
        };
        let run_result = plugin.parse_test_result(&test_run).unwrap();
        assert_eq!(run_result.status, RunStatus::TestsFailed);
    }

    #[test]
    fn converts_test_case_result() {
        init();

        let plugin = Stub::new();
        let test_case = TestCase {
            class_name: "cls".to_string(),
            exception: None,
            message: None,
            method_name: "mtd".to_string(),
            point_names: vec!["1".to_string(), "2".to_string()],
            status: TestCaseStatus::Failed,
        };
        let test_result = plugin.convert_test_case_result(test_case);
        assert_eq!(test_result.points, &["1", "2"]);
    }

    #[test]
    fn parses_java_home() {
        init();

        let properties = r#"Property settings:
    awt.toolkit = sun.awt.X11.XToolkit
    java.ext.dirs = /usr/lib/jvm/java-8-openjdk-amd64/jre/lib/ext
        /usr/java/packages/lib/ext
    java.home = /usr/lib/jvm/java-8-openjdk-amd64/jre
    user.timezone = 

openjdk version "1.8.0_252"S
"#;

        let parsed = Stub::parse_java_home(properties);
        assert_eq!(
            Some(PathBuf::from("/usr/lib/jvm/java-8-openjdk-amd64/jre")),
            parsed,
        );
    }

    #[test]
    fn gets_java_home() {
        init();

        let _java_home = Stub::get_java_home().unwrap();
    }

    #[test]
    fn scans_exercise_with_compile_result() {
        init();

        let temp_dir = dir_to_temp("tests/data/ant-exercise");

        let plugin = Stub::new();
        let compile_result = CompileResult {
            stdout: vec![],
            stderr: vec![],
            status_code: tmc_langs_framework::ExitStatus::Exited(0),
        };
        let desc = plugin
            .scan_exercise_with_compile_result(temp_dir.path(), "ex".to_string(), compile_result)
            .unwrap();
        assert_eq!(desc.tests[0].points[0], "arith-funcs");
    }

    #[test]
    fn creates_run_result_from_failed_compilation() {
        init();

        let plugin = Stub::new();
        let compile_result = CompileResult {
            status_code: tmc_langs_framework::ExitStatus::Exited(0),
            stdout: "hello, 世界".as_bytes().to_vec(),
            stderr: "エラー".as_bytes().to_vec(),
        };
        let run_result = plugin.run_result_from_failed_compilation(compile_result);
        assert_eq!(run_result.logs.get("stdout").unwrap(), "hello, 世界");
        assert_eq!(run_result.logs.get("stderr").unwrap(), "エラー");
    }

    #[test]
    fn runs_checkstyle() {
        init();

        let temp_dir = dir_to_temp("tests/data/ant-exercise");

        let plugin = Stub::new();
        let validation_result = plugin
            .run_checkstyle(&Language::from_639_3("fin").unwrap(), temp_dir.path())
            .unwrap();
        log::debug!("{:#?}", validation_result);
        let validation_errors = validation_result.validation_errors.unwrap();
        let validation_error = validation_errors.values().next().unwrap().get(0).unwrap();
        assert!(validation_error.message.contains("Sisennys väärin"));
    }

    #[test]
    fn parses_points() {
        assert!(Stub::java_points_parser("asd").is_err());
        assert!(Stub::java_points_parser(r#"@points("help""#).is_err());

        assert_eq!(
            Stub::java_points_parser(r#"@points("point")"#).unwrap().1,
            &["point"]
        );
        assert_eq!(
            Stub::java_points_parser(r#"@  PoInTs  (  "  another point  "  )  "#)
                .unwrap()
                .1,
            &["another", "point"]
        );
        assert_eq!(
            Stub::java_points_parser(r#"@points("point", "another point"  ,  "asd")"#)
                .unwrap()
                .1,
            &["point", "another", "point", "asd"]
        );
    }
}
