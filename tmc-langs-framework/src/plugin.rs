//! Contains LanguagePlugin.

pub use isolang::Language;

use crate::domain::{
    ExerciseDesc, ExercisePackagingConfiguration, RunResult, RunStatus, StyleValidationResult,
    TestResult,
};
use crate::error::TmcError;
use crate::file_util;
use crate::policy::StudentFilePolicy;
use crate::TmcProjectYml;
use nom::{branch, bytes, character, combinator, error::VerboseError, multi, sequence, IResult};
use std::collections::HashSet;
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
    const LINE_COMMENT: &'static str;
    const BLOCK_COMMENT: Option<(&'static str, &'static str)>;
    type StudentFilePolicy: StudentFilePolicy;

    /// Produces an exercise description of an exercise directory.
    ///
    /// This involves finding the test cases and the points offered by the
    /// exercise.
    ///
    /// Must return `Err` if the given path is not a valid exercise directory for
    /// this language.
    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc, TmcError>;

    /// Runs the tests for the exercise.
    fn run_tests(&self, path: &Path) -> Result<RunResult, TmcError> {
        let timeout = Self::StudentFilePolicy::new(path)?
            .get_project_config()
            .tests_timeout_ms
            .map(Duration::from_millis);
        let result = self.run_tests_with_timeout(path, timeout)?;

        // override success on no test cases
        if result.status == RunStatus::Passed && result.test_results.is_empty() {
            Ok(RunResult {
                status: RunStatus::TestsFailed,
                test_results: vec![TestResult {
                    name: "Tests found test".to_string(),
                    successful: false,
                    points: vec![],
                    message: "No tests found. Did you terminate your program with an exit() command?\nYou can also try submitting the exercise to the server."
                        .to_string(),
                    exception: vec![],
                }],
                logs: result.logs,
            })
        } else {
            Ok(result)
        }
    }

    /// Runs the tests for the exercise with the given timeout.
    /// Used by run_tests with the timeout from the project config.
    fn run_tests_with_timeout(
        &self,
        path: &Path,
        timeout: Option<Duration>,
    ) -> Result<RunResult, TmcError>;

    /// Run checkstyle or similar plugin to project if applicable, no-op by default
    fn check_code_style(
        &self,
        _path: &Path,
        _locale: Language,
    ) -> Result<Option<StyleValidationResult>, TmcError> {
        Ok(None)
    }

    /// Extract a given archive file containing a compressed project to a target location.
    ///
    /// This will overwrite any existing files as long as they are not specified as student files
    /// by the language dependent student file policy.
    // TODO: look at removing or relocating
    fn extract_project(
        compressed_project: impl std::io::Read + std::io::Seek,
        target_location: &Path,
        clean: bool,
    ) -> Result<(), TmcError> {
        log::debug!("Unzipping to {}", target_location.display());

        let mut zip_archive = ZipArchive::new(compressed_project)?;

        // find the exercise root directory inside the archive
        let project_dir = Self::find_project_dir_in_zip(&mut zip_archive)?;
        log::debug!("Project dir in zip: {}", project_dir.display());

        // extract config file if any
        let tmc_project_yml_path = project_dir.join(".tmcproject.yml");
        let tmc_project_yml_path_s = tmc_project_yml_path
            .to_str()
            .ok_or_else(|| TmcError::ProjectDirInvalidUtf8(project_dir.clone()))?;
        if let Ok(mut file) = zip_archive.by_name(tmc_project_yml_path_s) {
            let target_path = target_location.join(".tmcproject.yml");
            file_util::read_to_file(&mut file, target_path)?;
        }
        let policy = Self::StudentFilePolicy::new(target_location)?;

        // used to clean non-student files not in the zip later
        let mut unzip_paths = HashSet::new();

        for i in 0..zip_archive.len() {
            let mut file = zip_archive.by_index(i)?;
            let file_path = PathBuf::from(file.name());
            if file_path == tmc_project_yml_path {
                // already extracted
                continue;
            }

            let relative = match file_path.strip_prefix(&project_dir) {
                Ok(relative) => relative,
                _ => {
                    log::trace!("skip {}, not in project dir", file.name());
                    continue;
                }
            };
            let path_in_target = target_location.join(&relative);
            log::trace!("processing {:?} -> {:?}", file_path, path_in_target);
            unzip_paths.insert(path_in_target.clone());

            // not student file, or forced update
            let overwrite = {
                if !path_in_target.exists() {
                    // currently policies do not consider nonexistent files to ever be student files
                    // this is a not-so-nice hack around that, TODO: fix properly
                    let mut created_dirs = vec![];
                    for dir in path_in_target
                        .ancestors()
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                    {
                        if !dir.exists() {
                            file_util::create_dir(dir)?;
                            created_dirs.push(dir.to_path_buf());
                        }
                    }
                    let write = !policy.is_student_file(&path_in_target, &target_location)?
                        || policy.is_updating_forced(&relative)?;
                    for dir in created_dirs.into_iter().rev() {
                        file_util::remove_dir_empty(&dir)?;
                    }
                    write
                } else {
                    !policy.is_student_file(&path_in_target, &target_location)?
                        || policy.is_updating_forced(&relative)?
                }
            };

            if !path_in_target.exists() {
                // just extract
                if file.is_dir() {
                    file_util::create_dir_all(path_in_target)?;
                } else {
                    let mut overwrite_target = file_util::create_file(&path_in_target)?;
                    let mut file_contents = vec![];
                    file.read_to_end(&mut file_contents)
                        .map_err(|e| TmcError::ZipRead(file_path.clone(), e))?;
                    overwrite_target
                        .write_all(&file_contents)
                        .map_err(|e| TmcError::ZipWrite(path_in_target.clone(), e))?;
                }
            } else if overwrite {
                // overwrite existing
                if file.is_dir() {
                    // remove old and overwrite
                    if path_in_target.is_dir() {
                        file_util::remove_dir_all(&path_in_target)?;
                    } else if path_in_target.is_file() {
                        file_util::remove_file(&path_in_target)?;
                    }
                    log::trace!("creating {:?}", path_in_target);
                    file_util::create_dir_all(&path_in_target)?;
                } else {
                    // remove old if dir
                    if path_in_target.is_dir() {
                        file_util::remove_dir_all(&path_in_target)?;
                    }
                    log::trace!("writing to {}", path_in_target.display());
                    if let Some(parent) = path_in_target.parent() {
                        file_util::create_dir_all(parent)?;
                    }
                    let mut overwrite_target = file_util::create_file(&path_in_target)?;
                    let mut file_contents = vec![];
                    file.read_to_end(&mut file_contents)
                        .map_err(|e| TmcError::ZipRead(file_path.clone(), e))?;
                    overwrite_target
                        .write_all(&file_contents)
                        .map_err(|e| TmcError::ZipWrite(path_in_target.clone(), e))?;
                }
            }
        }

        if clean {
            // delete non-student files that were not in zip
            log::debug!("deleting non-student files not in zip");
            log::debug!("{:?}", unzip_paths);
            for entry in WalkDir::new(target_location)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if !unzip_paths.contains(entry.path())
                    && (policy.is_updating_forced(entry.path())?
                        || !policy.is_student_file(entry.path(), &target_location)?)
                {
                    log::debug!(
                        "rm {} {}",
                        entry.path().display(),
                        target_location.display()
                    );
                    if entry.path().is_dir() {
                        // delete if empty
                        if WalkDir::new(entry.path()).max_depth(1).into_iter().count() == 1 {
                            log::debug!("deleting empty directory {}", entry.path().display());
                            file_util::remove_dir_empty(entry.path())?;
                        }
                    } else {
                        log::debug!("removing file {}", entry.path().display());
                        file_util::remove_file(entry.path())?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Extracts student files from the compressed project.
    /// It finds the project dir from the zip and extracts the student files from there.
    /// Overwrites all files/directories.
    // todo: DRY, very similar to extract_project
    fn extract_student_files(
        compressed_project: impl Read + Seek,
        target_location: &Path,
    ) -> Result<(), TmcError> {
        log::debug!("Unzipping student files to {}", target_location.display());

        let mut zip_archive = ZipArchive::new(compressed_project)?;

        // find the exercise root directory inside the archive
        let project_dir = Self::find_project_dir_in_zip(&mut zip_archive)?;
        log::debug!("Project dir in zip: {}", project_dir.display());

        // extract config file if any
        let tmc_project_yml_path = project_dir.join(".tmcproject.yml");
        let tmc_project_yml_path = tmc_project_yml_path
            .to_str()
            .ok_or_else(|| TmcError::ProjectDirInvalidUtf8(project_dir.clone()))?;
        if let Ok(mut file) = zip_archive.by_name(tmc_project_yml_path) {
            let target_path = target_location.join(".tmcproject.yml");
            file_util::read_to_file(&mut file, target_path)?;
        }
        let policy = Self::StudentFilePolicy::new(target_location)?;

        for i in 0..zip_archive.len() {
            let mut file = zip_archive.by_index(i)?;
            let file_path = PathBuf::from(file.name());
            let relative = match file_path.strip_prefix(&project_dir) {
                Ok(relative) => relative,
                _ => {
                    log::trace!("skip {}, not in project dir", file.name());
                    continue;
                }
            };
            let path_in_target = target_location.join(&relative);
            log::trace!("processing {:?} -> {:?}", file_path, path_in_target);

            let write = {
                if !path_in_target.exists() {
                    // currently policies do not consider nonexistent files to ever be student files
                    // this is a not-so-nice hack around that, TODO: fix properly
                    let mut created_dirs = vec![];
                    for dir in path_in_target
                        .ancestors()
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                    {
                        if !dir.exists() {
                            file_util::create_dir(dir)?;
                            created_dirs.push(dir.to_path_buf());
                        }
                    }
                    let is_student_file =
                        policy.is_student_file(&path_in_target, &target_location)?;
                    for dir in created_dirs.into_iter().rev() {
                        file_util::remove_dir_empty(&dir)?;
                    }
                    is_student_file
                } else {
                    policy.is_student_file(&path_in_target, &target_location)?
                }
            };

            if write {
                if file.is_dir() {
                    if path_in_target.exists() {
                        // remove old and overwrite
                        if path_in_target.is_dir() {
                            file_util::remove_dir_all(&path_in_target)?;
                        } else {
                            file_util::remove_file(&path_in_target)?;
                        }
                    }
                    log::trace!("creating {:?}", path_in_target);
                    file_util::create_dir_all(&path_in_target)?;
                } else if file.is_file() {
                    let mut file_contents = vec![];
                    file.read_to_end(&mut file_contents)
                        .map_err(|e| TmcError::ZipRead(file_path.clone(), e))?;
                    if let Some(parent) = path_in_target.parent() {
                        file_util::create_dir_all(parent)?;
                    }
                    let mut overwrite_target = file_util::create_file(&path_in_target)?;
                    overwrite_target
                        .write_all(&file_contents)
                        .map_err(|e| TmcError::ZipWrite(path_in_target.clone(), e))?;
                }
            }
        }

        Ok(())
    }

    /// Searches the zip for a valid project directory.
    /// Note that the returned path may not actually have an entry in the zip.
    /// The default implementation tries to find a directory that contains a "src" directory,
    /// which may be sufficient for some languages.
    /// Ignores everything in a __MACOSX directory.
    fn find_project_dir_in_zip<R: Read + Seek>(
        zip_archive: &mut ZipArchive<R>,
    ) -> Result<PathBuf, TmcError> {
        for i in 0..zip_archive.len() {
            // zips don't necessarily contain entries for intermediate directories,
            // so we need to check every path for src
            let file = zip_archive.by_index(i)?;
            let file_path = Path::new(file.name());

            // todo: do in one pass somehow
            if file_path.components().any(|c| c.as_os_str() == "src") {
                let path: PathBuf = file_path
                    .components()
                    .take_while(|c| c.as_os_str() != "src")
                    .collect();

                if path.components().any(|p| p.as_os_str() == "__MACOSX") {
                    continue;
                }
                return Ok(path);
            }
        }
        Err(TmcError::NoProjectDirInZip)
    }

    /// Tells if there's a valid exercise in this path.
    fn is_exercise_type_correct(path: &Path) -> bool;

    /// Returns configuration which is used to package submission on tmc-server.
    // TODO: rethink
    fn get_exercise_packaging_configuration(
        configuration: TmcProjectYml,
    ) -> Result<ExercisePackagingConfiguration, TmcError> {
        let extra_student_files = configuration.extra_student_files;
        let extra_test_files = configuration.extra_exercise_files;

        let student_files = Self::get_default_student_file_paths()
            .into_iter()
            .chain(extra_student_files)
            .collect::<HashSet<_>>();
        let exercise_files_without_student_files = Self::get_default_exercise_file_paths()
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
    fn clean(&self, path: &Path) -> Result<(), TmcError>;

    fn get_default_student_file_paths() -> Vec<PathBuf>;

    fn get_default_exercise_file_paths() -> Vec<PathBuf>;

    /// Parses exercise files using Self::LINE_COMMENT and Self::BLOCK_COMMENt to filter out comments and Self::points_parser to parse points from the actual code.
    fn get_available_points(exercise_path: &Path) -> Result<Vec<String>, TmcError> {
        let config = TmcProjectYml::from(exercise_path)?;
        let config = Self::get_exercise_packaging_configuration(config)?;

        let mut points = Vec::new();
        for exercise_file_path in config.exercise_file_paths {
            let exercise_file_path = exercise_path.join(exercise_file_path);
            if !exercise_file_path.exists() {
                continue;
            }

            // file path may point to a directory of file, walkdir takes care of both
            for entry in WalkDir::new(exercise_file_path) {
                let entry = entry?;
                if entry.path().is_file() {
                    log::trace!("parsing points from {}", entry.path().display());
                    let file_contents = file_util::read_file_to_string(entry.path())?;

                    // reads any character
                    let etc_parser = combinator::value(Parse::Other, character::complete::anychar);

                    // reads a single line comment
                    let line_comment_parser = combinator::value(
                        Parse::LineComment,
                        sequence::delimited(
                            bytes::complete::tag(Self::LINE_COMMENT),
                            bytes::complete::take_until("\n"),
                            character::complete::newline,
                        ),
                    );

                    // reads a single block comment
                    let block_comment_parser: Box<dyn FnMut(_) -> _> =
                        if let Some((block_start, block_end)) = Self::BLOCK_COMMENT {
                            Box::new(combinator::value(
                                Parse::BlockComment,
                                sequence::delimited(
                                    bytes::complete::tag(block_start),
                                    bytes::complete::take_until(block_end),
                                    bytes::complete::tag(block_end),
                                ),
                            ))
                        } else {
                            Box::new(combinator::value(
                                Parse::BlockComment,
                                character::complete::one_of(""),
                            ))
                        };

                    // reads a points annotation
                    let points_parser =
                        combinator::map(Self::points_parser, |p| Parse::Points(p.to_string()));

                    // try to apply the interesting parsers, else read a character with the etc parser. repeat until the input ends
                    let mut parser = multi::many0(branch::alt((
                        line_comment_parser,
                        block_comment_parser,
                        points_parser,
                        etc_parser,
                    )));

                    let res: IResult<_, _, _> = parser(&file_contents);
                    match res {
                        Ok((_, parsed)) => {
                            for parse in parsed {
                                if let Parse::Points(parsed) = parse {
                                    // a single points annotation can contain multiple whitespace separated points
                                    let split_points =
                                        parsed.split_whitespace().map(str::to_string);
                                    points.extend(split_points);
                                }
                            }
                        }
                        Err(nom::Err::Incomplete(_)) => unreachable!("this should never happen"),
                        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => {
                            return Err(TmcError::PointParse(
                                entry.path().to_path_buf(),
                                VerboseError {
                                    errors: e
                                        .errors
                                        .into_iter()
                                        .map(|(s, k)| (s.to_string(), k))
                                        .collect(),
                                },
                            ));
                        }
                    }
                }
            }
        }
        Ok(points)
    }

    /// A nom parser that recognizes a points annotation and returns the inner points value.
    ///
    /// For example implementations, see the existing language plugins.
    fn points_parser(i: &str) -> IResult<&str, &str, nom::error::VerboseError<&str>>;
}

#[derive(Debug, Clone)]
enum Parse {
    LineComment,
    BlockComment,
    Points(String),
    Other,
}

#[cfg(test)]
mod test {
    use super::*;
    use nom::character;

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new().with_level(LevelFilter::Trace).init();
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

    fn dir_to_zip(source_dir: impl AsRef<std::path::Path>) -> Vec<u8> {
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

    struct MockPlugin {}

    struct MockPolicy {
        project_config: TmcProjectYml,
    }

    impl StudentFilePolicy for MockPolicy {
        fn new_with_project_config(project_config: TmcProjectYml) -> Self
        where
            Self: Sized,
        {
            Self { project_config }
        }
        fn get_project_config(&self) -> &TmcProjectYml {
            &self.project_config
        }
        fn is_student_source_file(path: &Path) -> bool {
            path.starts_with("src")
        }
    }

    impl LanguagePlugin for MockPlugin {
        const PLUGIN_NAME: &'static str = "mock_plugin";
        const LINE_COMMENT: &'static str = "//";
        const BLOCK_COMMENT: Option<(&'static str, &'static str)> = Some(("/*", "*/"));
        type StudentFilePolicy = MockPolicy;

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
            Ok(RunResult {
                status: RunStatus::Passed,
                test_results: vec![],
                logs: std::collections::HashMap::new(),
            })
        }

        fn is_exercise_type_correct(_path: &Path) -> bool {
            unimplemented!()
        }

        fn clean(&self, _path: &Path) -> Result<(), TmcError> {
            Ok(())
        }

        fn points_parser(i: &str) -> IResult<&str, &str, nom::error::VerboseError<&str>> {
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
                    branch::alt((
                        sequence::delimited(
                            character::complete::char('"'),
                            bytes::complete::is_not("\""),
                            character::complete::char('"'),
                        ),
                        sequence::delimited(
                            character::complete::char('\''),
                            bytes::complete::is_not("'"),
                            character::complete::char('\''),
                        ),
                    )),
                    sequence::tuple((
                        character::complete::multispace0,
                        character::complete::char(')'),
                    )),
                ),
                str::trim,
            )(i)
        }

        fn get_default_student_file_paths() -> Vec<PathBuf> {
            vec![PathBuf::from("src")]
        }

        fn get_default_exercise_file_paths() -> Vec<PathBuf> {
            vec![PathBuf::from("test")]
        }
    }

    #[test]
    fn gets_exercise_packaging_configuration() {
        init();

        let config = TmcProjectYml {
            extra_student_files: vec!["test/StudentTest.java".into(), "test/OtherTest.java".into()],
            extra_exercise_files: vec!["test/SomeFile.java".into(), "OtherTest.java".into()],
            ..Default::default()
        };
        let conf = MockPlugin::get_exercise_packaging_configuration(config).unwrap();
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
        init();
        let plugin = MockPlugin {};
        let res = plugin.run_tests(Path::new("")).unwrap();
        assert_eq!(res.status, RunStatus::TestsFailed);
        assert_eq!(res.test_results[0].name, "Tests found test")
    }

    #[test]
    fn gets_available_points() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(
            &temp,
            "non_test_dir/file.py",
            r#"
@Points("1.1")
"#,
        );
        let points = MockPlugin::get_available_points(&temp.path()).unwrap();
        assert!(points.is_empty());

        let temp = tempfile::tempdir().unwrap();
        file_to(
            &temp,
            "test/file.py",
            r#"
@Points("1")
def a():
    pass

@ points ( '2' )
def b():
    pass
    @    Points    (    "3"    )
def c():
    pass

@pOiNtS("4")
def d():
    pass
"#,
        );
        let points = MockPlugin::get_available_points(&temp.path()).unwrap();
        assert_eq!(points, &["1", "2", "3", "4"]);

        let temp = tempfile::tempdir().unwrap();
        file_to(
            &temp,
            "test/file.py",
            r#"
@Points("1")
def a():
    pass

// @Points("2")
def b():
    pass

@Points("3") // comment
def c():
    pass

/* @Points("4") */
def d():
    pass

/*
@Points("5")
def e():
    pass
*/

@Test // @Points("6")
def f():
    pass
"#,
        );
        let points = MockPlugin::get_available_points(&temp.path()).unwrap();
        assert_eq!(points, &["1", "3"]);
    }

    #[test]
    fn finds_project_dir_in_zip() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "dir1/dir2/dir3/src/file", "");
        let zip = dir_to_zip(&temp);

        let mut zip = ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
        let dir = MockPlugin::find_project_dir_in_zip(&mut zip).unwrap();
        assert_eq!(dir, Path::new("dir1").join("dir2").join("dir3"));
    }

    #[test]
    fn doesnt_find_project_dir_in_macos() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "dir1/dir2/dir3/__MACOSX/src/file", "");
        file_to(&temp, "dir1/__MACOSX/dir2/dir3/src/file", "");
        let zip = dir_to_zip(&temp);

        let mut zip = ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
        let dir = MockPlugin::find_project_dir_in_zip(&mut zip);
        assert!(dir.is_err());
    }

    #[test]
    fn extracts_student_files() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "dir/src/more/dirs/student file", "");
        file_to(&temp, "dir/test/exercise file", "");
        file_to(&temp, "not in project dir", "");
        let zip = dir_to_zip(&temp);

        MockPlugin::extract_student_files(
            std::io::Cursor::new(zip),
            &temp.path().join("extracted"),
        )
        .unwrap();

        for entry in WalkDir::new(temp.path().join("extracted")) {
            if let Ok(entry) = entry {
                log::debug!("{}", entry.path().display());
            }
        }

        assert!(temp
            .path()
            .join("extracted/src/more/dirs/student file")
            .exists());
        assert!(!temp.path().join("extracted/test/exercise file").exists());
    }

    #[test]
    fn extracts_student_dirs() {
        init();

        let temp = tempfile::tempdir().unwrap();
        dir_to(&temp, "dir/src");
        dir_to(&temp, "dir/test");
        dir_to(&temp, "not in project dir");
        let zip = dir_to_zip(&temp);

        MockPlugin::extract_student_files(
            std::io::Cursor::new(zip),
            &temp.path().join("extracted"),
        )
        .unwrap();

        for entry in WalkDir::new(temp.path().join("extracted")) {
            if let Ok(entry) = entry {
                log::debug!("{}", entry.path().display());
            }
        }

        assert!(temp.path().join("extracted/src").exists());
        assert!(!temp.path().join("extracted/test").exists());
        assert!(!temp.path().join("extracted/not in project dir").exists());
    }

    #[test]
    fn extract_student_files_overwrites() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "dir/src/file overwrites file", "new");
        file_to(&temp, "dir/src/file overwrites dir", "data");
        dir_to(&temp, "dir/src/dir overwrites file");
        dir_to(&temp, "dir/src/dir overwrites dir");
        let zip = dir_to_zip(&temp);
        file_to(&temp, "extracted/src/file overwrites file", "old");
        file_to(
            &temp,
            "extracted/src/file overwrites dir/some dir/some file",
            "",
        );
        file_to(&temp, "extracted/src/dir overwrites file", "old");
        file_to(
            &temp,
            "extracted/src/dir overwrites dir/another dir/another file",
            "",
        );

        MockPlugin::extract_student_files(
            std::io::Cursor::new(zip),
            &temp.path().join("extracted"),
        )
        .unwrap();

        for entry in WalkDir::new(temp.path().join("extracted")) {
            if let Ok(entry) = entry {
                log::debug!("{}", entry.path().display());
            }
        }

        let path = temp.path().join("extracted/src/file overwrites file");
        assert!(path.is_file());
        let s = std::fs::read_to_string(path).unwrap();
        assert_eq!(s, "new");

        let path = temp.path().join("extracted/src/file overwrites dir");
        assert!(path.is_file());
        let s = std::fs::read_to_string(path).unwrap();
        assert_eq!(s, "data");

        let path = temp.path().join("extracted/src/dir overwrites file");
        assert!(path.is_dir());

        let path = temp.path().join("extracted/src/dir overwrites dir");
        assert!(path.is_dir());
        assert!(!path.join("another dir").exists())
    }

    #[test]
    fn extracts_project() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "dir/src/more/dirs/student file", "");
        file_to(&temp, "dir/test/exercise file", "");
        file_to(&temp, "not in project dir", "");
        let zip = dir_to_zip(&temp);

        MockPlugin::extract_project(
            std::io::Cursor::new(zip),
            &temp.path().join("extracted"),
            false,
        )
        .unwrap();

        for entry in WalkDir::new(temp.path().join("extracted")) {
            if let Ok(entry) = entry {
                log::debug!("{}", entry.path().display());
            }
        }

        assert!(temp
            .path()
            .join("extracted/src/more/dirs/student file")
            .exists());
        assert!(temp.path().join("extracted/test/exercise file").exists());
        assert!(!temp.path().join("extracted/not in project dir").exists());
    }

    #[test]
    fn extract_project_overwrites_default() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "dir/src/student file", "new");
        file_to(&temp, "dir/test/exercise file", "new");
        let zip = dir_to_zip(&temp);
        file_to(&temp, "extracted/src/student file", "old");
        file_to(&temp, "extracted/test/exercise file", "old");

        MockPlugin::extract_project(
            std::io::Cursor::new(zip),
            &temp.path().join("extracted"),
            false,
        )
        .unwrap();

        for entry in WalkDir::new(temp.path().join("extracted")) {
            if let Ok(entry) = entry {
                log::debug!("{}", entry.path().display());
            }
        }

        let s = std::fs::read_to_string(temp.path().join("extracted/src/student file")).unwrap();
        assert_eq!(s, "old");
        let s = std::fs::read_to_string(temp.path().join("extracted/test/exercise file")).unwrap();
        assert_eq!(s, "new");
    }

    #[test]
    fn extract_project_overwrites_with_config_file() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "dir/src/forced update", "new");
        file_to(&temp, "dir/extra student file", "new");
        file_to(
            &temp,
            "dir/.tmcproject.yml",
            r#"
extra_student_files:
  - "extra student file"
force_update:
  - "src/forced update"
"#,
        );
        let zip = dir_to_zip(&temp);
        file_to(&temp, "extracted/src/forced update", "old");
        file_to(&temp, "extracted/extra student file", "old");

        MockPlugin::extract_project(
            std::io::Cursor::new(zip),
            &temp.path().join("extracted"),
            false,
        )
        .unwrap();

        for entry in WalkDir::new(temp.path().join("extracted")) {
            if let Ok(entry) = entry {
                log::debug!("{}", entry.path().display());
            }
        }

        let s = std::fs::read_to_string(temp.path().join("extracted/src/forced update")).unwrap();
        assert_eq!(s, "new");
        let s = std::fs::read_to_string(temp.path().join("extracted/extra student file")).unwrap();
        assert_eq!(s, "old");
    }

    #[test]
    fn extract_project_doesnt_clean() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "dir/src/some file", "");
        let zip = dir_to_zip(&temp);
        file_to(&temp, "extracted/test/some existing non-student file", "");

        MockPlugin::extract_project(
            std::io::Cursor::new(zip),
            &temp.path().join("extracted"),
            false,
        )
        .unwrap();

        for entry in WalkDir::new(temp.path().join("extracted")) {
            if let Ok(entry) = entry {
                log::debug!("{}", entry.path().display());
            }
        }

        assert!(temp
            .path()
            .join("extracted/test/some existing non-student file")
            .exists())
    }

    #[test]
    fn extract_project_cleans() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "dir/src/some file", "");
        let zip = dir_to_zip(&temp);
        file_to(&temp, "extracted/test/some existing non-student file", "");

        MockPlugin::extract_project(
            std::io::Cursor::new(zip),
            &temp.path().join("extracted"),
            true,
        )
        .unwrap();

        for entry in WalkDir::new(temp.path().join("extracted")) {
            if let Ok(entry) = entry {
                log::debug!("{}", entry.path().display());
            }
        }

        assert!(!temp
            .path()
            .join("extracted/test/some existing non-student file")
            .exists())
    }

    #[test]
    fn splits_points_by_whitespace() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(
            &temp,
            "test/file",
            r#"
@points("1 2 3 4")
@points("  5  6  7  8  ")
"#,
        );

        let points = MockPlugin::get_available_points(temp.path()).unwrap();
        assert_eq!(points, &["1", "2", "3", "4", "5", "6", "7", "8"]);
    }

    #[test]
    fn parses_empty() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "test/file", r#""#);

        let points = MockPlugin::get_available_points(temp.path()).unwrap();
        assert!(points.is_empty());

        let temp = tempfile::tempdir().unwrap();
        file_to(
            &temp,
            "test/file",
            r#"
"#,
        );

        let points = MockPlugin::get_available_points(temp.path()).unwrap();
        assert!(points.is_empty());
    }
}
