//! Java Ant plugin.

use crate::{
    error::JavaError, java_plugin::JavaPlugin, AntStudentFilePolicy, CompileResult, TestRun,
    SEPARATOR,
};
use j4rs::Jvm;
use std::{
    env,
    ffi::OsStr,
    io::{Read, Seek},
    ops::ControlFlow::{Break, Continue},
    path::{Path, PathBuf},
    time::Duration,
};
use tmc_langs_framework::{
    nom::{error::VerboseError, IResult},
    Archive, ExerciseDesc, Language, LanguagePlugin, RunResult, StyleValidationResult, TmcCommand,
    TmcError,
};
use tmc_langs_util::{file_util, path_util};
use walkdir::WalkDir;

pub struct AntPlugin {
    jvm: Jvm,
}

impl AntPlugin {
    pub fn new() -> Result<Self, JavaError> {
        let jvm = crate::instantiate_jvm()?;
        Ok(Self { jvm })
    }

    fn get_ant_executable(&self) -> &'static str {
        if cfg!(windows) {
            let command = TmcCommand::piped("ant");
            if let Ok(status) = command.with(|e| e.arg("-version")).status() {
                if status.success() {
                    return "ant";
                }
            }
            // if ant not found on windows, try ant.bat
            "ant.bat"
        } else {
            "ant"
        }
    }

    /// Writes the bundled tmc-junit-runner into dest_path/lib/testrunner/tmc-junit-runner.jar
    // TODO: check for updates
    pub fn copy_tmc_junit_runner(dest_path: &Path) -> Result<(), JavaError> {
        log::debug!("copying TMC Junit runner");

        let runner_dir = dest_path.join("lib").join("testrunner");
        let runner_path = runner_dir.join("tmc-junit-runner.jar");

        // TODO: don't traverse symlinks
        if !runner_path.exists() {
            log::debug!("writing tmc-junit-runner to {}", runner_path.display());
            file_util::write_to_file(super::TMC_JUNIT_RUNNER_BYTES, &runner_path)?;
        } else {
            log::debug!("already exists");
        }
        Ok(())
    }
}

/// Project directory:
/// Contains build.xml file.
/// OR
/// Contains src and test directories.
impl LanguagePlugin for AntPlugin {
    const PLUGIN_NAME: &'static str = "apache-ant";
    const LINE_COMMENT: &'static str = "//";
    const BLOCK_COMMENT: Option<(&'static str, &'static str)> = Some(("/*", "*/"));
    type StudentFilePolicy = AntStudentFilePolicy;

    fn check_code_style(
        &self,
        path: &Path,
        locale: Language,
    ) -> Result<Option<StyleValidationResult>, TmcError> {
        Ok(Some(self.run_checkstyle(&locale, path)?))
    }

    /// Scans the exercise at the given path. Immediately exits if the target directory is not a valid exercise.
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
        timeout: Option<Duration>,
    ) -> Result<RunResult, TmcError> {
        Ok(self.run_java_tests(project_root_path, timeout)?)
    }

    fn find_project_dir_in_archive<R: Read + Seek>(
        archive: &mut Archive<R>,
    ) -> Result<PathBuf, TmcError> {
        let mut iter = archive.iter()?;
        let mut src_parents = vec![];
        let mut test_parents = vec![];
        let project_dir = loop {
            let next = iter.with_next(|file| {
                let file_path = file.path()?;

                if file.is_file() {
                    // check for build.xml
                    if let Some(parent) = path_util::get_parent_of(&file_path, "build.xml") {
                        return Ok(Break(Some(parent)));
                    }
                } else if file.is_dir() {
                    // check for src
                    if let Some(src_parent) = path_util::get_parent_of_dir(&file_path, "src") {
                        if test_parents.contains(&src_parent) {
                            // found a test in the same directory before, return
                            return Ok(Break(Some(src_parent)));
                        } else {
                            src_parents.push(src_parent)
                        }
                    }

                    // check for test
                    if let Some(test_parent) = path_util::get_parent_of_dir(&file_path, "test") {
                        if src_parents.contains(&test_parent) {
                            // found a test in the same directory before, return
                            return Ok(Break(Some(test_parent)));
                        } else {
                            test_parents.push(test_parent)
                        }
                    }
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

    /// Checks if the directory contains a build.xml file, or a src and a test directory.
    fn is_exercise_type_correct(path: &Path) -> bool {
        path.join("build.xml").is_file() || path.join("test").is_dir() && path.join("src").is_dir()
    }

    fn clean(&self, path: &Path) -> Result<(), TmcError> {
        log::debug!("cleaning project at {}", path.display());

        // TODO: is writing stdout and stderr to file really necessary?
        let stdout_path = path.join("build_log.txt");
        let stdout = file_util::create_file(&stdout_path)?;
        let stderr_path = path.join("build_errors.txt");
        let stderr = file_util::create_file(&stderr_path)?;

        let ant_exec = self.get_ant_executable();
        let _output = TmcCommand::new(ant_exec)
            .with(|e| e.arg("clean").stdout(stdout).stderr(stderr).cwd(path))
            .output_checked()?;
        file_util::remove_file(&stdout_path)?;
        file_util::remove_file(&stderr_path)?;
        Ok(())
    }

    fn points_parser(i: &str) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
        Self::java_points_parser(i)
    }

    fn get_default_student_file_paths() -> Vec<PathBuf> {
        vec![PathBuf::from("src")]
    }

    fn get_default_exercise_file_paths() -> Vec<PathBuf> {
        vec![PathBuf::from("test")]
    }
}

impl JavaPlugin for AntPlugin {
    const TEST_DIR: &'static str = "test";

    fn jvm(&self) -> &Jvm {
        &self.jvm
    }

    /// Constructs the class path for the given path.
    fn get_project_class_path(&self, path: &Path) -> Result<String, JavaError> {
        let mut paths = vec![];

        // add all .jar files in lib
        let lib_dir = path.join("lib");
        for entry in WalkDir::new(&lib_dir) {
            let entry = entry?;

            if entry.path().is_file() && entry.path().extension() == Some(OsStr::new("jar")) {
                paths.push(entry.path().to_path_buf());
            }
        }
        paths.push(lib_dir);
        paths.push(path.join("build").join("test").join("classes"));
        paths.push(path.join("build").join("classes"));

        let java_home = Self::get_java_home()?;
        // TODO: what's tools.jar?
        let tools_jar_path = java_home.join("..").join("lib").join("tools.jar");
        if tools_jar_path.exists() {
            paths.push(tools_jar_path);
        } else {
            log::warn!("no tools.jar found; skip adding to class path");
        }

        // ignore non-UTF8 paths
        let paths = paths
            .into_iter()
            .filter_map(|p| p.to_str().map(str::to_string))
            .collect::<Vec<_>>();

        // TODO: is it OK to not include the runner in the classpath?
        Self::copy_tmc_junit_runner(path)?;
        Ok(paths.join(SEPARATOR))
    }

    fn build(&self, project_root_path: &Path) -> Result<CompileResult, JavaError> {
        log::info!("building project at {}", project_root_path.display());

        let ant_exec = self.get_ant_executable();
        let output = TmcCommand::piped(ant_exec)
            .with(|e| e.arg("compile-test").cwd(project_root_path))
            .output()?;

        // TODO: is it really necessary to write the logs in files?
        log::debug!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        log::debug!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        let stdout_path = project_root_path.join("build_log.txt");
        let stderr_path = project_root_path.join("build_errors.txt");
        file_util::write_to_file(&mut output.stdout.as_slice(), stdout_path)?;
        file_util::write_to_file(&mut output.stderr.as_slice(), stderr_path)?;

        Ok(CompileResult {
            status_code: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    fn create_run_result_file(
        &self,
        path: &Path,
        timeout: Option<Duration>,
        compile_result: CompileResult,
    ) -> Result<TestRun, JavaError> {
        log::info!("running tests for project at {}", path.display());

        // build java args
        let mut arguments = vec![];
        // JVM args
        if let Ok(jvm_options) = env::var("JVM_OPTIONS") {
            arguments.extend(
                jvm_options
                    .split(" +")
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string()),
            )
        }
        // TMC args
        let test_dir = path.join("test");
        let result_file = path.join("results.txt");
        arguments.push(format!("-Dtmc.test_class_dir={}", test_dir.display()));
        arguments.push(format!("-Dtmc.results_file={}", result_file.display()));
        // TODO: endorsed libs?
        let endorsed_libs_path = path.join("lib/endorsed");
        if endorsed_libs_path.exists() {
            arguments.push(format!(
                "-Djava.endorsed.dirs={}",
                endorsed_libs_path.display()
            ));
        }
        // scan needs to be before getting class path
        let exercise = self.scan_exercise_with_compile_result(
            path,
            format!("{}{}", path.display(), "/test"), // TODO: ?
            compile_result,
        )?;
        // classpath
        arguments.push("-cp".to_string());
        let class_path = self.get_project_class_path(path)?;
        arguments.push(class_path);
        // main
        arguments.push("fi.helsinki.cs.tmc.testrunner.Main".to_string());
        // ?
        for desc in exercise.tests {
            let mut s = String::new();
            s.push_str(&desc.name.replace(' ', "."));
            s.push('{');
            s.push_str(&desc.points.join(","));
            s.push('}');
            arguments.push(s);
        }

        log::debug!("java args '{}' in {}", arguments.join(" "), path.display());
        let command = TmcCommand::piped("java").with(|e| e.cwd(path).args(&arguments));
        let output = if let Some(timeout) = timeout {
            command.output_with_timeout(timeout)?
        } else {
            command.output()?
        };

        Ok(TestRun {
            test_results: result_file,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use std::fs;
    use tmc_langs_framework::{Archive, StyleValidationStrategy};
    use tmc_langs_util::deserialize;

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new()
            .with_level(LevelFilter::Debug)
            // j4rs does a lot of logging
            .with_module_level("j4rs", LevelFilter::Warn)
            .init();
    }

    fn file_to(
        target_dir: impl AsRef<std::path::Path>,
        target_relative: impl AsRef<std::path::Path>,
        contents: impl AsRef<[u8]>,
    ) -> PathBuf {
        let target = target_dir.as_ref().join(target_relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&target, contents.as_ref()).unwrap();
        target
    }

    fn dir_to(
        target_dir: impl AsRef<std::path::Path>,
        target_relative: impl AsRef<std::path::Path>,
    ) -> PathBuf {
        let target = target_dir.as_ref().join(target_relative);
        std::fs::create_dir_all(&target).unwrap();
        target
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

    fn dir_to_zip(source_dir: impl AsRef<std::path::Path>) -> Vec<u8> {
        use std::io::Write;

        let mut target = vec![];
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut target));

        for entry in walkdir::WalkDir::new(&source_dir)
            .min_depth(1)
            .sort_by(|a, b| a.path().cmp(b.path()))
        {
            let entry = entry.unwrap();
            let rela = entry
                .path()
                .strip_prefix(&source_dir)
                .unwrap()
                .to_str()
                .unwrap();
            if entry.path().is_dir() {
                zip.add_directory(rela, zip::write::FileOptions::default())
                    .unwrap();
            } else if entry.path().is_file() {
                zip.start_file(rela, zip::write::FileOptions::default())
                    .unwrap();
                let bytes = std::fs::read(entry.path()).unwrap();
                zip.write_all(&bytes).unwrap();
            }
        }

        zip.finish().unwrap();
        drop(zip);
        target
    }

    #[test]
    fn copies_tmc_junit_runner() {
        init();

        let temp = tempfile::TempDir::new().unwrap();
        let jar_dir = temp.path().join("dir");
        let jar_path = jar_dir.join("lib/testrunner/tmc-junit-runner.jar");
        assert!(!jar_path.exists());
        AntPlugin::copy_tmc_junit_runner(&jar_dir).unwrap();
        assert!(jar_path.exists());
    }

    #[test]
    fn gets_project_class_path() {
        init();

        let temp = tempfile::TempDir::new().unwrap();
        let test_path = temp.path().join("dir");
        file_to(&test_path, "lib/junit-4.10.jar", "");
        file_to(&test_path, "lib/edu-test-utils-0.4.1.jar", "");

        let plugin = AntPlugin::new().unwrap();
        let cp = plugin.get_project_class_path(&test_path).unwrap();

        let sep = std::path::MAIN_SEPARATOR;
        assert!(
            cp.contains(&format!(
                "{0}{1}lib{1}junit-4.10.jar",
                test_path.display(),
                sep
            )),
            "Classpath {} did not contain junit",
            cp
        );
        assert!(
            cp.contains(&format!(
                "{0}{1}lib{1}edu-test-utils-0.4.1.jar",
                test_path.display(),
                sep
            )),
            "Classpath {} did not contain edu-test-utils",
            cp
        );
        assert!(
            cp.contains(&format!("{0}{1}build{1}classes", test_path.display(), sep)),
            "Classpath {} did not contain build{}classes",
            cp,
            sep
        );
        assert!(
            cp.contains(&format!(
                "{0}{1}build{1}test{1}classes",
                test_path.display(),
                sep
            )),
            "Classpath {} did not contain build/test/classes",
            cp
        );
        // tools.jar is in java home, tricky to test
        /*
        assert!(
            cp.ends_with(&format!("{0}..{0}lib{0}tools.jar", sep)),
            "Classpath was {}",
            cp
        );
        */
    }

    #[test]
    fn builds() {
        init();

        let temp_dir = dir_to_temp("tests/data/ant-exercise");
        let plugin = AntPlugin::new().unwrap();
        let compile_result = plugin.build(temp_dir.path()).unwrap();
        assert!(compile_result.status_code.success());
        // may contain unexpected output depending on machine config
        // assert!(!compile_result.stdout.is_empty());
        // assert!(compile_result.stderr.is_empty());
    }

    #[test]
    fn creates_run_result_file() {
        init();

        let temp_dir = dir_to_temp("tests/data/ant-exercise");
        let plugin = AntPlugin::new().unwrap();
        let compile_result = plugin.build(temp_dir.path()).unwrap();
        let test_run = plugin
            .create_run_result_file(temp_dir.path(), None, compile_result)
            .unwrap();
        log::trace!("stdout: {}", String::from_utf8_lossy(&test_run.stdout));
        log::debug!("stderr: {}", String::from_utf8_lossy(&test_run.stderr));
        // may contain unexpected output depending on machine config
        // assert!(test_run.stdout.is_empty());
        // assert!(test_run.stderr.is_empty());
        let res = fs::read_to_string(test_run.test_results).unwrap();
        let test_cases: Vec<super::super::TestCase> = deserialize::json_from_str(&res).unwrap();

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
        // assert_eq!(exception.class_name, "java.lang.AssertionError");
        assert!(exception.message.as_ref().unwrap().starts_with("expected:"));
        // assert!(exception.cause.is_none());

        let stack_trace = &exception.stack_trace[0];
        assert_eq!(stack_trace.declaring_class, "org.junit.Assert");
        assert_eq!(stack_trace.file_name.as_ref().unwrap(), "Assert.java");
        assert_eq!(stack_trace.method_name, "fail");
    }

    #[test]
    fn scans_exercise() {
        init();

        let temp_dir = dir_to_temp("tests/data/ant-exercise");
        let plugin = AntPlugin::new().unwrap();
        let exercises = plugin
            .scan_exercise(&temp_dir.path(), "test".to_string())
            .unwrap();
        assert_eq!(exercises.name, "test");
        assert_eq!(exercises.tests.len(), 4);
        assert_eq!(exercises.tests[0].name, "ArithTest testAdd");
        assert_eq!(exercises.tests[0].points, ["arith-funcs"]);
    }

    #[test]
    fn runs_checkstyle() {
        init();

        let temp_dir = dir_to_temp("tests/data/ant-exercise");
        let plugin = AntPlugin::new().unwrap();
        let checkstyle_result = plugin
            .check_code_style(temp_dir.path(), Language::from_639_3("fin").unwrap())
            .unwrap()
            .unwrap();

        assert_eq!(checkstyle_result.strategy, StyleValidationStrategy::Fail);
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
    fn runs_tests() {
        init();

        let temp_dir = dir_to_temp("tests/data/ant-exercise");
        let plugin = AntPlugin::new().unwrap();
        let test_result = plugin
            .run_tests_with_timeout(Path::new(temp_dir.path()), None)
            .unwrap();
        log::debug!("{:?}", test_result);
        assert_eq!(
            test_result.status,
            tmc_langs_framework::RunStatus::TestsFailed
        );
    }

    #[test]
    fn runs_tests_with_timeout() {
        init();

        let temp_dir = dir_to_temp("tests/data/ant-exercise");
        let plugin = AntPlugin::new().unwrap();
        let test_result_err = plugin
            .run_tests_with_timeout(Path::new(temp_dir.path()), Some(Duration::from_nanos(1)))
            .unwrap_err();
        log::debug!("{:?}", test_result_err);

        // verify that there's a timeout error in the source chain
        use std::error::Error;
        let mut source = test_result_err.source();
        while let Some(inner) = source {
            source = inner.source();
            if let Some(cmd_error) = inner.downcast_ref::<tmc_langs_framework::CommandError>() {
                if matches!(cmd_error, tmc_langs_framework::CommandError::TimeOut { .. }) {
                    return;
                }
            }
        }
        panic!("no timeout error found");
    }

    #[test]
    fn exercise_type_is_correct() {
        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "build.xml", "");
        assert!(AntPlugin::is_exercise_type_correct(temp.path()));

        let temp = tempfile::tempdir().unwrap();
        dir_to(&temp, "test");
        dir_to(&temp, "src");
        assert!(AntPlugin::is_exercise_type_correct(temp.path()));
    }

    #[test]
    fn exercise_type_is_not_correct() {
        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "buid.xml", "");
        file_to(&temp, "dir/build.xml", "");
        file_to(&temp, "test", "");
        dir_to(&temp, "src");
        assert!(!AntPlugin::is_exercise_type_correct(temp.path()));
    }

    #[test]
    fn cleans() {
        init();

        let temp_dir = dir_to_temp("tests/data/ant-exercise");
        let test_path = temp_dir.path();
        let plugin = AntPlugin::new().unwrap();
        plugin.clean(test_path).unwrap();
    }

    #[test]
    fn finds_project_dir_in_zip() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        dir_to(&temp_dir, "Outer/Inner/ant-exercise/src");
        dir_to(&temp_dir, "Outer/Inner/ant-exercise/test");

        let zip_contents = dir_to_zip(&temp_dir);
        let mut zip = Archive::zip(std::io::Cursor::new(zip_contents)).unwrap();
        let dir = AntPlugin::find_project_dir_in_archive(&mut zip).unwrap();
        assert_eq!(dir, Path::new("Outer/Inner/ant-exercise"));
    }

    #[test]
    fn finds_project_dir_in_zip_build() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(&temp_dir, "Outer/Inner/ant-exercise/build.xml", "build!");

        let zip_contents = dir_to_zip(&temp_dir);
        let mut zip = Archive::zip(std::io::Cursor::new(zip_contents)).unwrap();
        let dir = AntPlugin::find_project_dir_in_archive(&mut zip).unwrap();
        assert_eq!(dir, Path::new("Outer/Inner/ant-exercise"));
    }

    #[test]
    fn doesnt_find_project_dir_in_zip() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        dir_to(&temp_dir, "Outer/Inner/ant-exercise/srcb");

        let zip_contents = dir_to_zip(&temp_dir);
        let mut zip = Archive::zip(std::io::Cursor::new(zip_contents)).unwrap();
        let dir = AntPlugin::find_project_dir_in_archive(&mut zip);
        assert!(dir.is_err());
    }
}
