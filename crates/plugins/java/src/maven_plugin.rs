//! Java Maven plugin.

use crate::{
    CompileResult, JvmWrapper, MavenStudentFilePolicy, SEPARATOR, TestRun, error::JavaError,
    java_plugin::JavaPlugin,
};
use flate2::read::GzDecoder;
use std::{
    ffi::{OsStr, OsString},
    io::{Cursor, Read, Seek},
    ops::ControlFlow::{Break, Continue},
    path::{Path, PathBuf},
    time::Duration,
};
use tar::Archive as Tar;
use tmc_langs_framework::{
    Archive, ExerciseDesc, Language, LanguagePlugin, RunResult, StyleValidationResult, TmcCommand,
    TmcError, nom::IResult, nom_language::error::VerboseError,
};
use tmc_langs_util::file_util;

const MVN_ARCHIVE: &[u8] = include_bytes!("../deps/apache-maven-3.8.1-bin.tar.gz");
const MVN_PATH_IN_ARCHIVE: &str = "apache-maven-3.8.1"; // the name of the base directory in the maven archive
const MVN_VERSION: &str = "3.8.1";

pub struct MavenPlugin {
    jvm: JvmWrapper,
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
    // the executable used from within the extracted maven differs per platform
    fn get_mvn_command() -> Result<OsString, JavaError> {
        // check if mvn is in PATH
        if let Ok(status) = TmcCommand::piped("mvn")
            .with(|e| e.arg("--batch-mode").arg("--version"))
            .status()
        {
            if status.success() {
                return Ok(OsString::from("mvn"));
            }
        }
        log::debug!("could not execute mvn, using bundled maven");
        let tmc_path = dirs::cache_dir().ok_or(JavaError::CacheDir)?.join("tmc");

        #[cfg(windows)]
        let mvn_exec = "mvn.cmd";
        #[cfg(not(windows))]
        let mvn_exec = "mvn";

        let mvn_path = tmc_path.join("apache-maven");
        let mvn_version_path = mvn_path.join("VERSION");

        let needs_update = if mvn_version_path.exists() {
            let version_contents = file_util::read_file_to_string(&mvn_version_path)?;
            MVN_VERSION != version_contents
        } else {
            true
        };

        if needs_update {
            if mvn_path.exists() {
                file_util::remove_dir_all(&mvn_path)?;
            }
            // TODO: remove this bit eventually, this is just to clean up the old maven cachce that had the version in the name
            let old_path = tmc_path.join("apache-maven-3.6.3");
            if old_path.exists() {
                file_util::remove_dir_all(old_path)?;
            }

            log::debug!("extracting bundled tar");
            let tar = GzDecoder::new(Cursor::new(MVN_ARCHIVE));
            let mut tar = Tar::new(tar);
            tar.unpack(&tmc_path)
                .map_err(|e| JavaError::JarWrite(tmc_path.clone(), e))?;

            log::debug!("renaming extracted archive to apache-maven");
            file_util::rename(tmc_path.join(MVN_PATH_IN_ARCHIVE), &mvn_path)?;

            log::debug!("writing bundle version data");
            file_util::write_to_file(MVN_VERSION.as_bytes(), &mvn_version_path)?;
        }

        let mvn_exec_path = mvn_path.join("bin").join(mvn_exec);
        Ok(mvn_exec_path.as_os_str().to_os_string())
    }
}

/// Project directory:
/// Contains pom.xml file
impl LanguagePlugin for MavenPlugin {
    const PLUGIN_NAME: &'static str = "apache-maven";
    const DEFAULT_SANDBOX_IMAGE: &'static str = "eu.gcr.io/moocfi-public/tmc-sandbox-java:latest";
    const LINE_COMMENT: &'static str = "//";
    const BLOCK_COMMENT: Option<(&'static str, &'static str)> = Some(("/*", "*/"));
    type StudentFilePolicy = MavenStudentFilePolicy;

    fn check_code_style(
        &self,
        path: &Path,
        locale: Language,
    ) -> Result<Option<StyleValidationResult>, TmcError> {
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
        timeout: Option<Duration>,
    ) -> Result<RunResult, TmcError> {
        Ok(self.run_java_tests(project_root_path, timeout)?)
    }

    fn find_project_dir_in_archive<R: Read + Seek>(
        archive: &mut Archive<R>,
    ) -> Result<PathBuf, TmcError> {
        let mut iter = archive.iter()?;

        let project_dir = loop {
            // try to find pom.xml
            let next = iter.with_next(|file| {
                let file_path = file.path()?;

                if file.is_file() && file_path.file_name() == Some(OsStr::new("pom.xml")) {
                    if let Some(pom_parent) = file_path.parent() {
                        return Ok(Break(Some(pom_parent.to_path_buf())));
                    }
                }
                Ok(Continue(()))
            })?;
            if let Some(Some(root)) = next.break_value() {
                return Ok(root);
            }

            // accept any dir with src/main/*.java
            let root = iter.with_next(|file| {
                let file_path = file.path()?;

                let mut components = file_path.iter();
                let mut in_src = false;
                let mut in_src_main = false;
                for next in components {
                    if in_src_main {
                        if Path::new(next).extension() == Some(OsStr::new("java")) {
                            let root = file_path
                                .iter()
                                .take_while(|c| c != &OsStr::new("main"))
                                .collect();
                            return Ok(Break(Some(root)));
                        }
                    } else {
                        break;
                    }

                    if in_src {
                        if next == "main" {
                            in_src_main = true;
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }

                    if next == "src" {
                        in_src = true;
                    } else {
                        break;
                    }
                }
                if file.is_file() && file_path.extension() == Some(OsStr::new("java")) {
                    if let Some(pom_parent) = file_path.parent() {
                        return Ok(Break(Some(pom_parent.to_path_buf())));
                    }
                }
                Ok(Continue(()))
            });
            match root? {
                Continue(_) => continue,
                Break(project_dir) => break project_dir,
            }
        };

        match project_dir {
            Some(project_dir) => Ok(project_dir),
            None => Err(TmcError::NoProjectDirInArchive),
        }
    }

    /// Checks if the directory has a pom.xml file.
    fn is_exercise_type_correct(path: &Path) -> bool {
        path.join("pom.xml").exists()
    }

    /// Runs the Maven clean plugin.
    fn clean(&self, path: &Path) -> Result<(), TmcError> {
        log::info!("Cleaning maven project at {}", path.display());

        let mvn_command = Self::get_mvn_command()?;
        let _output = TmcCommand::piped(mvn_command)
            .with(|e| e.cwd(path).arg("--batch-mode").arg("clean"))
            .output_checked()?;

        Ok(())
    }

    fn get_default_student_file_paths() -> Vec<PathBuf> {
        vec![PathBuf::from("src/main")]
    }

    fn get_default_exercise_file_paths() -> Vec<PathBuf> {
        vec![PathBuf::from("src/test")]
    }

    fn points_parser(i: &str) -> IResult<&str, Vec<&str>, VerboseError<&str>> {
        Self::java_points_parser(i)
    }
}

impl JavaPlugin for MavenPlugin {
    const TEST_DIR: &'static str = "src";

    fn jvm(&self) -> &JvmWrapper {
        &self.jvm
    }

    fn get_project_class_path(&self, path: &Path) -> Result<String, JavaError> {
        // canonicalize root path to avoid issues where the cwd and project root are different directories
        let path = file_util::canonicalize(path)?;
        log::info!("Building classpath for maven project at {}", path.display());

        let temp = tempfile::tempdir().map_err(JavaError::TempDir)?;
        let class_path_file = temp.path().join("cp.txt");

        let output_arg = format!("-Dmdep.outputFile={}", class_path_file.display());
        let mvn_path = Self::get_mvn_command()?;
        let _output = TmcCommand::piped(mvn_path)
            .with(|e| {
                e.cwd(&path)
                    .arg("--batch-mode")
                    .arg("dependency:build-classpath")
                    .arg(output_arg)
            })
            .output_checked()?;

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
        let output = TmcCommand::piped(mvn_path)
            .with(|e| {
                e.cwd(project_root_path)
                    .arg("--batch-mode")
                    .arg("clean")
                    .arg("compile")
                    .arg("test-compile")
            })
            .output()?;

        Ok(CompileResult {
            status_code: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    /// Runs the tmc-maven-plugin.
    fn create_run_result_file(
        &self,
        path: &Path,
        timeout: Option<Duration>,
        _compile_result: CompileResult,
    ) -> Result<TestRun, JavaError> {
        log::info!("Running tests for maven project at {}", path.display());

        let mvn_path = Self::get_mvn_command()?;
        let command = TmcCommand::piped(mvn_path).with(|e| {
            e.cwd(path)
                .arg("--batch-mode")
                .arg("fi.helsinki.cs.tmc:tmc-maven-plugin:1.12:test")
        });
        let output = if let Some(timeout) = timeout {
            command.output_with_timeout_checked(timeout)?
        } else {
            command.output_checked()?
        };

        Ok(TestRun {
            test_results: path.join("target/test_output.txt"),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

#[cfg(test)]
#[cfg(not(target_os = "macos"))] // issues with maven dependencies
#[allow(clippy::unwrap_used)]
mod test {

    use super::{
        super::{TestCase, TestCaseStatus},
        *,
    };
    use once_cell::sync::Lazy;
    use std::{
        fs,
        sync::{Mutex, MutexGuard},
    };
    use tmc_langs_framework::{Archive, StyleValidationStrategy};
    use tmc_langs_util::deserialize;
    use zip::write::SimpleFileOptions;

    static MAVEN_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new()
            .with_level(LevelFilter::Debug)
            .with_module_level("j4rs", LevelFilter::Warn)
            .init();
    }

    /// Maven doesn't like being run in parallel, at least on Windows.
    /// For now the tests access the MavenPlugin with a function that locks a mutex.
    fn get_maven() -> (MavenPlugin, MutexGuard<'static, ()>) {
        (MavenPlugin::new().unwrap(), MAVEN_LOCK.lock().unwrap())
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
                zip.add_directory(rela, SimpleFileOptions::default())
                    .unwrap();
            } else if entry.path().is_file() {
                zip.start_file(rela, SimpleFileOptions::default()).unwrap();
                let bytes = std::fs::read(entry.path()).unwrap();
                zip.write_all(&bytes).unwrap();
            }
        }

        zip.finish().unwrap();
        target
    }

    #[test]
    #[ignore = "changing PATH breaks other tests, figure out a better way to test this. or don't"]
    fn unpacks_bundled_mvn() {
        let cmd = MavenPlugin::get_mvn_command().unwrap();
        let expected = format!(
            "tmc{0}apache-maven-3.8.1{0}bin{0}mvn",
            std::path::MAIN_SEPARATOR
        );
        assert!(cmd.to_string_lossy().ends_with(&expected))
    }

    #[test]
    fn runs_checkstyle() {
        init();

        let temp_dir = dir_to_temp("tests/data/maven-exercise");
        let (plugin, _lock) = get_maven();
        let checkstyle_result = plugin
            .check_code_style(temp_dir.path(), Language::from_639_3("fin").unwrap())
            .unwrap()
            .unwrap();

        assert_eq!(checkstyle_result.strategy, StyleValidationStrategy::Fail);
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

    #[test]
    fn scans_exercise() {
        init();

        let temp_dir = dir_to_temp("tests/data/maven-exercise");
        let (plugin, _lock) = get_maven();
        let exercises = plugin
            .scan_exercise(temp_dir.path(), "test".to_string())
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
    fn runs_tests() {
        init();

        let temp_dir = dir_to_temp("tests/data/maven-exercise");
        let (plugin, _lock) = get_maven();
        let res = plugin.run_tests(temp_dir.path()).unwrap();
        log::debug!("{res:#?}");
        assert_eq!(res.status, tmc_langs_framework::RunStatus::TestsFailed);
    }

    #[test]
    fn runs_tests_timeout() {
        init();

        let temp_dir = dir_to_temp("tests/data/maven-exercise");
        let (plugin, _lock) = get_maven();
        let test_result_err = plugin
            .run_tests_with_timeout(temp_dir.path(), Some(std::time::Duration::from_nanos(1)))
            .unwrap_err();
        log::debug!("{test_result_err:#?}");

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
        panic!()
    }

    #[test]
    fn exercise_type_is_correct() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(temp_dir.path(), "pom.xml", "");
        assert!(MavenPlugin::is_exercise_type_correct(temp_dir.path()));
    }

    #[test]
    fn exercise_type_is_incorrect() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(temp_dir.path(), "pom", "");
        file_to(temp_dir.path(), "po.xml", "");
        file_to(temp_dir.path(), "dir/pom.xml", "");
        assert!(!MavenPlugin::is_exercise_type_correct(temp_dir.path()));
    }

    #[test]
    fn cleans() {
        init();

        let temp_dir = dir_to_temp("tests/data/maven-exercise");
        file_to(&temp_dir, "target/output file", "");

        assert!(temp_dir.path().join("target/output file").exists());
        assert!(temp_dir.path().join("src").exists());
        let (plugin, _lock) = get_maven();
        plugin.clean(temp_dir.path()).unwrap();
        assert!(!temp_dir.path().join("target/output file").exists());
        assert!(temp_dir.path().join("src").exists());
    }

    #[test]
    fn finds_project_dir_in_zip() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        file_to(&temp_dir, "Outer/Inner/maven-exercise/pom.xml", "pom!");

        let zip_contents = dir_to_zip(&temp_dir);
        let mut zip = Archive::zip(std::io::Cursor::new(zip_contents)).unwrap();
        let dir = MavenPlugin::find_project_dir_in_archive(&mut zip).unwrap();
        assert_eq!(dir, Path::new("Outer/Inner/maven-exercise"));
    }

    #[test]
    fn doesnt_find_project_dir_in_zip() {
        init();

        let temp_dir = tempfile::tempdir().unwrap();
        dir_to(&temp_dir, "Outer/Inner/maven-exercise/srcb");

        let zip_contents = dir_to_zip(&temp_dir);
        let mut zip = Archive::zip(std::io::Cursor::new(zip_contents)).unwrap();
        let dir = MavenPlugin::find_project_dir_in_archive(&mut zip);
        assert!(dir.is_err());
    }

    #[test]
    fn gets_project_class_path() {
        init();

        let temp_dir = dir_to_temp("tests/data/maven-exercise");
        let (plugin, _lock) = get_maven();
        let class_path = plugin.get_project_class_path(temp_dir.path()).unwrap();
        log::debug!("{class_path}");
        let expected = format!("{0}junit{0}", std::path::MAIN_SEPARATOR);
        assert!(class_path.contains(&expected));
    }

    #[test]
    fn builds() {
        init();

        use std::path::PathBuf;
        log::debug!("{}", PathBuf::from(".").canonicalize().unwrap().display());

        let temp_dir = dir_to_temp("tests/data/maven-exercise");
        let (plugin, _lock) = get_maven();
        let compile_result = plugin.build(temp_dir.path()).unwrap();
        assert!(compile_result.status_code.success());
    }

    #[test]
    fn creates_run_result_file() {
        init();

        let temp_dir = dir_to_temp("tests/data/maven-exercise");
        let test_path = temp_dir.path();
        let (plugin, _lock) = get_maven();
        let compile_result = plugin.build(test_path).unwrap();
        let test_run = plugin
            .create_run_result_file(test_path, None, compile_result)
            .unwrap();
        let test_result: Vec<TestCase> =
            deserialize::json_from_str(&fs::read_to_string(test_run.test_results).unwrap())
                .unwrap();
        let test_case = &test_result[0];

        assert_eq!(test_case.class_name, "fi.helsinki.cs.maventest.AppTest");
        assert_eq!(test_case.point_names, ["maven-exercise"]);
        assert_eq!(test_case.status, TestCaseStatus::Failed);
        let message = test_case.message.as_ref().unwrap();
        assert!(message.starts_with("ComparisonFailure"));

        let exception = test_case.exception.as_ref().unwrap();
        // assert_eq!(exception.class_name, "org.junit.ComparisonFailure");
        assert!(exception.message.as_ref().unwrap().starts_with("expected"));
        let stack_trace = &exception.stack_trace[0];
        assert_eq!(stack_trace.declaring_class, "org.junit.Assert");
        assert_eq!(stack_trace.file_name.as_ref().unwrap(), "Assert.java");
        assert_eq!(stack_trace.line_number, 115);
        assert_eq!(stack_trace.method_name, "assertEquals");
    }
}
