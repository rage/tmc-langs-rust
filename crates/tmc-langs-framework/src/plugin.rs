//! Contains LanguagePlugin.

use crate::{
    Archive, Compression,
    archive::ArchiveIterator,
    domain::{
        ExerciseDesc, ExercisePackagingConfiguration, RunResult, RunStatus, StyleValidationResult,
        TestResult,
    },
    error::TmcError,
    policy::StudentFilePolicy,
};
pub use isolang::Language;
use nom::{IResult, Parser, branch, bytes, character, combinator, multi, sequence};
use nom_language::error::VerboseError;
use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    io::{Read, Seek},
    ops::ControlFlow::{Break, Continue},
    path::{Path, PathBuf},
    time::Duration,
};
use tmc_langs_util::file_util;
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
pub trait LanguagePlugin {
    const PLUGIN_NAME: &'static str;
    const DEFAULT_SANDBOX_IMAGE: &'static str;
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
            .map(Into::into)
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
    fn extract_project<R: Read + Seek>(
        archive: &mut Archive<R>,
        target_location: &Path,
        clean: bool,
    ) -> Result<(), TmcError> {
        log::debug!(
            "Extracting to {} ({})",
            target_location.display(),
            archive.compression()
        );

        // find the exercise root directory inside the archive
        let project_dir = Self::find_project_dir_in_archive(archive)?;
        log::debug!("Project dir in zip: {}", project_dir.display());

        // extract config file if any
        let tmc_project_yml_path = project_dir.join(".tmcproject.yml");
        let tmc_project_yml_path_s = tmc_project_yml_path
            .to_str()
            .ok_or_else(|| TmcError::ProjectDirInvalidUtf8(project_dir.clone()))?;
        if let Ok(mut file) = archive.by_path(tmc_project_yml_path_s) {
            let target_path = target_location.join(".tmcproject.yml");
            file_util::read_to_file(&mut file, target_path)?;
        }
        let policy = Self::StudentFilePolicy::new(target_location)?;

        // used to clean non-student files not in the zip later
        let mut files_from_archive = HashSet::new();
        files_from_archive.insert(target_location.join(".tmcproject.yml")); // prevent cleaning .tmcproject.yml

        let mut iter = archive.iter()?;
        loop {
            let next = iter.with_next::<(), _>(|mut file| {
                let file_path = file.path()?;
                if file_path == Path::new(tmc_project_yml_path_s) {
                    // already extracted
                    return Ok(Continue(()));
                }

                let relative = match file_path.strip_prefix(&project_dir) {
                    Ok(relative) => relative,
                    _ => {
                        log::trace!("skip {}, not in project dir", file_path.display());
                        return Ok(Continue(()));
                    }
                };
                let path_in_target = target_location.join(relative);
                log::trace!("processing {file_path:?} -> {path_in_target:?}");

                files_from_archive.insert(path_in_target.clone());

                if !path_in_target.exists() {
                    // just extract
                    if file.is_dir() {
                        file_util::create_dir_all(path_in_target)?;
                    } else {
                        file_util::read_to_file(&mut file, path_in_target)?;
                    }
                } else if !policy.is_student_file(relative)
                    || policy.is_updating_forced(relative)?
                {
                    // not student file, or forced update
                    if file.is_file() {
                        // remove old if dir
                        if path_in_target.is_dir() {
                            file_util::remove_dir_all(&path_in_target)?;
                        }
                        file_util::read_to_file(&mut file, path_in_target)?;
                    }
                }
                Ok(Continue(()))
            });
            match next? {
                Continue(_) => continue,
                Break(_) => break,
            }
        }

        if clean {
            // delete non-student files that were not in archive
            log::debug!("deleting non-student files not in archive");
            for entry in WalkDir::new(target_location)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let relative = entry
                    .path()
                    .strip_prefix(target_location)
                    .expect("all entries are inside target");
                if !files_from_archive.contains(entry.path())
                    && (policy.is_updating_forced(entry.path())?
                        || !policy.is_student_file(relative))
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
    /// Overwrites all files.
    /// Important: does not extract .tmcproject.yml from the students' submission as they control that file and they could use it to modify the test files.
    fn extract_student_files(
        compressed_project: impl Read + Seek,
        compression: Compression,
        target_location: &Path,
    ) -> Result<(), TmcError> {
        log::debug!("Extracting student files to {}", target_location.display());

        let mut archive = Archive::new(compressed_project, compression)?;

        // find the exercise root directory inside the archive
        let project_dir = Self::safe_find_project_dir_in_archive(&mut archive);
        log::debug!("Project directory in archive: {}", project_dir.display());

        let policy = Self::StudentFilePolicy::new(target_location)?;

        let mut iter: ArchiveIterator<_> = archive.iter()?;
        loop {
            let next = iter.with_next::<(), _>(|mut file| {
                // get the path where the file should be extracted
                let file_path = file.path()?;
                let relative = match file_path.strip_prefix(&project_dir) {
                    Ok(relative) => relative,
                    _ => {
                        log::trace!("skip {}, not in project dir", file_path.display());
                        return Ok(Continue(()));
                    }
                };
                let path_in_target = target_location.join(relative);
                log::trace!("processing {file_path:?} -> {path_in_target:?}");

                if policy.is_student_file(relative) {
                    if file.is_file() {
                        // for files, everything should be removed out of the way
                        file_util::remove_all(&path_in_target)?;
                        file_util::read_to_file(&mut file, &path_in_target)?;
                    } else {
                        // for directories, we should keep existing directories but delete files at the same path
                        if path_in_target.is_file() {
                            file_util::remove_file(&path_in_target)?;
                        }
                        file_util::create_dir_all(&path_in_target)?;
                    }
                }
                Ok(Continue(()))
            });
            match next? {
                Continue(_) => continue,
                Break(_) => break,
            }
        }

        Ok(())
    }

    /// Searches the zip for a valid project directory.
    /// This function is used to detect the language plugin for the archive, so
    /// simply finding "src" is not sufficient, e.g. the Python plugin should check
    /// that there is an actual "src/*.py" file inside src.
    /// Note that the returned path may not actually have an entry in the zip.
    fn find_project_dir_in_archive<R: Read + Seek>(
        archive: &mut Archive<R>,
    ) -> Result<PathBuf, TmcError>;

    /// A safer variant of `find_project_dir_in_archive` used by default extraction helpers.
    ///
    /// Fallback order:
    /// 1) Delegate to `find_project_dir_in_archive` implemented by the language plugin
    /// 2) First directory containing a `.tmcproject.yml`
    /// 3) If archive root has only one folder, use that folder
    /// 4) Archive root (empty path)
    fn safe_find_project_dir_in_archive<R: Read + Seek>(archive: &mut Archive<R>) -> PathBuf {
        // 1) Try plugin-specific project dir detection first
        if let Ok(dir) = Self::find_project_dir_in_archive(archive) {
            return dir;
        }

        // 2) Try to find the first directory that contains a .tmcproject.yml
        if let Ok(mut iter) = archive.iter() {
            loop {
                let next = iter.with_next(|file| {
                    let file_path = file.path()?;
                    if file.is_file()
                        && file_path
                            .file_name()
                            .map(|name| name == OsStr::new(".tmcproject.yml"))
                            .unwrap_or(false)
                    {
                        let parent = file_path
                            .parent()
                            .map(PathBuf::from)
                            .unwrap_or_else(|| PathBuf::from(""));
                        return Ok(Break(Some(parent)));
                    }
                    Ok(Continue(()))
                });
                match next {
                    Ok(Continue(_)) => continue,
                    Ok(Break(Some(dir))) => return dir,
                    Ok(Break(None)) => break,
                    Err(_) => break,
                }
            }
        }

        // 3) Check if archive root has only one folder. This is the format tmc-langs-cli sends submissions, so all official clients should use this format.
        if let Ok(mut iter) = archive.iter() {
            let mut root_entries = HashSet::<OsString>::new();
            loop {
                let next = iter.with_next::<(), _>(|file| {
                    let file_path = file.path()?;
                    if let Some(first_component) = file_path.iter().next() {
                        root_entries.insert(first_component.to_os_string());
                    }
                    Ok(Continue(()))
                });
                match next {
                    Ok(Continue(_)) => continue,
                    Ok(Break(_)) => break,
                    Err(_) => break,
                }
            }

            // If there's exactly one folder at the root and no files, skip over it
            // Special case: don't skip over certain folders
            let excluded_folders = [OsStr::new("src")];

            if root_entries.len() == 1 {
                let only = root_entries.iter().next().expect("len is 1");
                if !excluded_folders.contains(&only.as_os_str()) {
                    return PathBuf::from(only);
                }
            }
        }

        // 4) Default to archive root
        PathBuf::from("")
    }

    /// Tells if there's a valid exercise in this archive.
    /// Unlike `is_exercise_type_correct`, searches the entire archive.
    fn is_archive_type_correct<R: Read + Seek>(archive: &mut Archive<R>) -> bool {
        Self::find_project_dir_in_archive(archive).is_ok()
    }

    /// Tells if there's a valid exercise in this path. Delegates to `find_project_dir_in_archive` by default.
    /// Unlike `is_archive_type_correct`, only checks the root directory.
    fn is_exercise_type_correct(path: &Path) -> bool;

    /// Returns configuration which is used to package submission on tmc-server.
    fn get_exercise_packaging_configuration(
        path: &Path,
    ) -> Result<ExercisePackagingConfiguration, TmcError> {
        let policy = Self::StudentFilePolicy::new(path)?;
        let mut config = ExercisePackagingConfiguration {
            student_file_paths: HashSet::new(),
            exercise_file_paths: HashSet::new(),
        };
        for entry in WalkDir::new(path).min_depth(1) {
            let entry = entry?;
            if entry.metadata()?.is_dir() {
                continue;
            }

            let path = entry
                .path()
                .strip_prefix(path)
                .expect("All entries are within path")
                .to_path_buf();
            if policy.is_student_file(&path) {
                config.student_file_paths.insert(path);
            } else {
                config.exercise_file_paths.insert(path);
            }
        }

        Ok(config)
    }

    /// Runs clean command e.g `make clean` for make or `mvn clean` for maven.
    fn clean(&self, path: &Path) -> Result<(), TmcError>;

    fn get_default_student_file_paths() -> Vec<PathBuf>;

    fn get_default_exercise_file_paths() -> Vec<PathBuf>;

    /// Parses exercise files using Self::LINE_COMMENT and Self::BLOCK_COMMENT to filter out comments and Self::points_parser to parse points from the actual code.
    fn get_available_points(exercise_path: &Path) -> Result<Vec<String>, TmcError> {
        let config = Self::get_exercise_packaging_configuration(exercise_path)?;

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
                    let file_contents = file_util::read_file_to_string_lossy(entry.path())?;

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
                    let block_comment_parser = |i| {
                        if let Some((block_start, block_end)) = Self::BLOCK_COMMENT {
                            combinator::value(
                                Parse::BlockComment,
                                sequence::delimited(
                                    bytes::complete::tag(block_start),
                                    bytes::complete::take_until(block_end),
                                    bytes::complete::tag(block_end),
                                ),
                            )
                            .parse(i)
                        } else {
                            combinator::value(Parse::BlockComment, character::complete::one_of(""))
                                .parse(i)
                        }
                    };

                    // reads a points annotation
                    let points_parser = combinator::map(Self::points_parser, |p| {
                        Parse::Points(p.into_iter().map(|s| s.to_string()).collect())
                    });

                    // try to apply the interesting parsers, else read a character with the etc parser. repeat until the input ends
                    let mut parser = multi::many0(branch::alt((
                        line_comment_parser,
                        block_comment_parser,
                        points_parser,
                        etc_parser,
                    )));

                    let res: IResult<_, _, _> = parser.parse(&file_contents);
                    match res {
                        Ok((_, parsed)) => {
                            for parse in parsed {
                                if let Parse::Points(parsed) = parse {
                                    for point in parsed {
                                        // a single points annotation can contain multiple whitespace separated points
                                        let split_points =
                                            point.split_whitespace().map(str::to_string);
                                        points.extend(split_points);
                                    }
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

    /// A nom parser that recognizes a points annotation and returns the inner points value(s).
    ///
    /// For example implementations, see the existing language plugins.
    fn points_parser(i: &str) -> IResult<&str, Vec<&str>, VerboseError<&str>>;
}

#[derive(Debug, Clone)]
enum Parse {
    LineComment,
    BlockComment,
    Points(Vec<String>),
    Other,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use crate::test_helpers::{MockPlugin, SimpleMockPlugin};
    use std::io::Write;
    use zip::{ZipWriter, write::SimpleFileOptions};

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
    fn gets_exercise_packaging_configuration() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(
            &temp,
            ".tmcproject.yml",
            r#"
extra_student_files:
  - "test/StudentTest.java"
  - "test/OtherTest.java"
  - "InBothLists.java"
extra_exercise_files:
  - "src/SomeFile.java"
  - "src/OtherTest.java"
  - "InBothLists.java"
"#,
        );
        file_to(&temp, "test/StudentTest.java", "");
        file_to(&temp, "test/OtherTest.java", "");
        file_to(&temp, "src/SomeFile.java", "");
        file_to(&temp, "src/OtherTest.java", "");
        file_to(&temp, "InBothLists.java", "");
        let conf = MockPlugin::get_exercise_packaging_configuration(temp.path()).unwrap();
        assert!(
            conf.student_file_paths
                .contains(Path::new("test/StudentTest.java"))
        );
        assert!(
            conf.student_file_paths
                .contains(Path::new("test/OtherTest.java"))
        );
        assert!(
            conf.exercise_file_paths
                .contains(Path::new("src/SomeFile.java"))
        );
        assert!(
            !conf
                .exercise_file_paths
                .contains(Path::new("test/OtherTest.java"))
        );

        assert!(
            conf.student_file_paths
                .contains(Path::new("InBothLists.java"))
        );
        assert!(
            !conf
                .exercise_file_paths
                .contains(Path::new("InBothLists.java"))
        );
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
            "src/student_file.py",
            r#"
@Points("1.1")
"#,
        );
        let points = MockPlugin::get_available_points(temp.path()).unwrap();
        assert!(points.is_empty());

        let temp = tempfile::tempdir().unwrap();
        file_to(
            &temp,
            "test/exercise_file.py",
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
        let points = MockPlugin::get_available_points(temp.path()).unwrap();
        assert_eq!(points, &["1", "2", "3", "4"]);

        let temp = tempfile::tempdir().unwrap();
        file_to(
            &temp,
            "test/exercise_file.py",
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
        let points = MockPlugin::get_available_points(temp.path()).unwrap();
        assert_eq!(points, &["1", "3"]);
    }

    #[test]
    fn finds_project_dir_in_zip() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "dir1/dir2/dir3/src/file", "");
        let zip = dir_to_zip(&temp);

        let mut zip = Archive::zip(std::io::Cursor::new(zip)).unwrap();
        let dir = MockPlugin::find_project_dir_in_archive(&mut zip).unwrap();
        assert_eq!(dir, Path::new("dir1").join("dir2").join("dir3"));
    }

    #[test]
    fn doesnt_find_project_dir_in_macos() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "dir1/dir2/dir3/__MACOSX/src/file", "");
        file_to(&temp, "dir1/__MACOSX/dir2/dir3/src/file", "");
        let zip = dir_to_zip(&temp);

        let mut zip = Archive::zip(std::io::Cursor::new(zip)).unwrap();
        let dir = MockPlugin::find_project_dir_in_archive(&mut zip);
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
            Compression::Zip,
            &temp.path().join("extracted"),
        )
        .unwrap();

        assert!(
            temp.path()
                .join("extracted/src/more/dirs/student file")
                .exists()
        );
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
            Compression::Zip,
            &temp.path().join("extracted"),
        )
        .unwrap();

        for entry in WalkDir::new(temp.path().join("extracted"))
            .into_iter()
            .flatten()
        {
            log::debug!("{}", entry.path().display());
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
        let zip = dir_to_zip(&temp);
        file_to(&temp, "extracted/src/file overwrites file", "old");
        file_to(
            &temp,
            "extracted/src/file overwrites dir/some dir/some file",
            "",
        );
        file_to(&temp, "extracted/src/dir overwrites file", "old");

        MockPlugin::extract_student_files(
            std::io::Cursor::new(zip),
            Compression::Zip,
            &temp.path().join("extracted"),
        )
        .unwrap();

        for entry in WalkDir::new(temp.path().join("extracted"))
            .into_iter()
            .flatten()
        {
            log::debug!("{}", entry.path().display());
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
    }

    #[test]
    fn extracts_project() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "dir/src/more/dirs/student file", "");
        file_to(&temp, "dir/test/exercise file", "");
        file_to(&temp, "not in project dir", "");
        let zip = dir_to_zip(&temp);

        let mut arch = Archive::zip(std::io::Cursor::new(zip)).unwrap();
        MockPlugin::extract_project(&mut arch, &temp.path().join("extracted"), false).unwrap();

        for entry in WalkDir::new(temp.path().join("extracted"))
            .into_iter()
            .flatten()
        {
            log::debug!("{}", entry.path().display());
        }

        assert!(
            temp.path()
                .join("extracted/src/more/dirs/student file")
                .exists()
        );
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

        let mut arch = Archive::zip(std::io::Cursor::new(zip)).unwrap();
        MockPlugin::extract_project(&mut arch, &temp.path().join("extracted"), false).unwrap();

        for entry in WalkDir::new(temp.path().join("extracted"))
            .into_iter()
            .flatten()
        {
            log::debug!("{}", entry.path().display());
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

        let mut arch = Archive::zip(std::io::Cursor::new(zip)).unwrap();
        MockPlugin::extract_project(&mut arch, &temp.path().join("extracted"), false).unwrap();

        for entry in WalkDir::new(temp.path().join("extracted"))
            .into_iter()
            .flatten()
        {
            log::debug!("{}", entry.path().display());
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

        for entry in WalkDir::new(temp.path().join("extracted"))
            .into_iter()
            .flatten()
        {
            log::debug!("{}", entry.path().display());
        }

        let mut arch = Archive::zip(std::io::Cursor::new(zip)).unwrap();
        MockPlugin::extract_project(&mut arch, &temp.path().join("extracted"), false).unwrap();

        for entry in WalkDir::new(temp.path().join("extracted"))
            .into_iter()
            .flatten()
        {
            log::debug!("{}", entry.path().display());
        }

        assert!(
            temp.path()
                .join("extracted/test/some existing non-student file")
                .exists()
        )
    }

    #[test]
    fn extract_project_cleans() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "dir/src/some file", "");
        let zip = dir_to_zip(&temp);
        file_to(&temp, "extracted/test/some existing non-student file", "");

        let mut arch = Archive::zip(std::io::Cursor::new(zip)).unwrap();
        MockPlugin::extract_project(&mut arch, &temp.path().join("extracted"), true).unwrap();

        for entry in WalkDir::new(temp.path().join("extracted"))
            .into_iter()
            .flatten()
        {
            log::debug!("{}", entry.path().display());
        }

        assert!(
            !temp
                .path()
                .join("extracted/test/some existing non-student file")
                .exists()
        )
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

    #[test]
    fn extract_student_files_does_not_clean_directories_incorrectly() {
        init();

        let temp = tempfile::tempdir().unwrap();
        file_to(&temp, "src/file", "");

        let buf = vec![];
        let mut zw = ZipWriter::new(std::io::Cursor::new(buf));
        zw.add_directory("src", SimpleFileOptions::default())
            .unwrap();
        let buf = zw.finish().unwrap();

        MockPlugin::extract_student_files(buf, Compression::Zip, temp.path()).unwrap();
        assert!(temp.path().join("src/file").exists());
    }

    #[test]
    fn extract_student_files_does_not_extract_tmcproject_yml() {
        init();

        let temp = tempfile::tempdir().unwrap();
        // create a project with a .tmcproject.yml in the project root and a student file under src
        file_to(&temp, "dir/src/student_file", "");
        file_to(&temp, "dir/.tmcproject.yml", "some: yaml");
        let zip = dir_to_zip(&temp);

        // extract student files
        MockPlugin::extract_student_files(
            std::io::Cursor::new(zip),
            Compression::Zip,
            &temp.path().join("extracted"),
        )
        .unwrap();

        // ensure student files are extracted
        assert!(temp.path().join("extracted/src/student_file").exists());
        // ensure .tmcproject.yml is NOT extracted
        assert!(!temp.path().join("extracted/.tmcproject.yml").exists());
    }

    #[test]
    fn safe_find_project_dir_fallback_to_tmcproject_yml() {
        init();

        let temp = tempfile::tempdir().unwrap();
        // create an archive without src directory (which would normally fail)
        // but with a .tmcproject.yml in a subdirectory
        file_to(&temp, "some/deep/path/.tmcproject.yml", "some: yaml");
        file_to(&temp, "some/deep/path/src/student_file", "content");
        let zip = dir_to_zip(&temp);

        // extract student files - should use fallback to .tmcproject.yml parent
        MockPlugin::extract_student_files(
            std::io::Cursor::new(zip),
            Compression::Zip,
            &temp.path().join("extracted"),
        )
        .unwrap();

        // ensure student files are extracted from the fallback directory
        assert!(temp.path().join("extracted/src/student_file").exists());
        let content =
            std::fs::read_to_string(temp.path().join("extracted/src/student_file")).unwrap();
        assert_eq!(content, "content");
    }

    #[test]
    /** Matches the format tmc-langs-cli sends submissions. This makes sure submissions created by official clients are supported. */
    fn safe_find_project_dir_fallback_to_single_folder() {
        init();

        let temp = tempfile::tempdir().unwrap();
        // create an archive with only one folder at root level
        file_to(&temp, "project_folder/src/student_file", "content");
        let zip = dir_to_zip(&temp);

        // extract student files - should use fallback to the single folder
        MockPlugin::extract_student_files(
            std::io::Cursor::new(zip),
            Compression::Zip,
            &temp.path().join("extracted"),
        )
        .unwrap();

        // ensure student files are extracted from the single folder
        assert!(temp.path().join("extracted/src/student_file").exists());
        let content =
            std::fs::read_to_string(temp.path().join("extracted/src/student_file")).unwrap();
        assert_eq!(content, "content");
    }

    #[test]
    fn safe_find_project_dir_fallback_to_root() {
        init();

        let temp = tempfile::tempdir().unwrap();
        // create an archive without src directory and without .tmcproject.yml
        // should fallback to root
        file_to(&temp, "src/student_file", "content");
        let zip = dir_to_zip(&temp);

        // extract student files - should use fallback to root
        MockPlugin::extract_student_files(
            std::io::Cursor::new(zip),
            Compression::Zip,
            &temp.path().join("extracted"),
        )
        .unwrap();

        // ensure student files are extracted from the root
        assert!(temp.path().join("extracted/src/student_file").exists());
        let content =
            std::fs::read_to_string(temp.path().join("extracted/src/student_file")).unwrap();
        assert_eq!(content, "content");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn safe_find_project_dir_single_folder_not_used_when_root_has_files() {
        init();

        let temp = tempfile::tempdir().unwrap();
        // root has a single folder and also a file at root
        // SimpleMockPlugin will never find project directory, so fallback logic will be used
        file_to(&temp, "project_folder/src/student_file", "content");
        file_to(&temp, "root_file.txt", "x");
        let zip = dir_to_zip(&temp);

        // extract student files - should NOT use the single-folder fallback because there's a root file
        // so it should fallback to root, which extracts project_folder/src/student_file
        SimpleMockPlugin::extract_student_files(
            std::io::Cursor::new(zip),
            Compression::Zip,
            &temp.path().join("extracted"),
        )
        .unwrap();

        // The file should be extracted as project_folder/src/student_file (preserving full path from root)
        // This test verifies that the single folder fallback is not used when there's a root file
        assert!(
            temp.path()
                .join("extracted/project_folder/src/student_file")
                .exists()
        );
        let content = std::fs::read_to_string(
            temp.path()
                .join("extracted/project_folder/src/student_file"),
        )
        .unwrap();
        assert_eq!(content, "content");
    }

    #[test]
    fn safe_find_project_dir_does_not_skip_over_src_folder() {
        init();

        let temp = tempfile::tempdir().unwrap();
        // root has only a single "src" folder, no root files
        // SimpleMockPlugin will never find project directory, so fallback logic will be used
        file_to(&temp, "src/student_file", "content");
        let zip = dir_to_zip(&temp);

        // extract student files - should use the "src" folder as project root
        SimpleMockPlugin::extract_student_files(
            std::io::Cursor::new(zip),
            Compression::Zip,
            &temp.path().join("extracted"),
        )
        .unwrap();

        // The file should be extracted as student_file (using the src folder as project root)
        // This test verifies that the "src" folder is used as the project directory
        assert!(temp.path().join("extracted/src/student_file").exists());
        let content =
            std::fs::read_to_string(temp.path().join("extracted/src/student_file")).unwrap();
        assert_eq!(content, "content");
    }
}
