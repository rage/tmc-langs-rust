//! Functions for processing submissions.

use lazy_static::lazy_static;
use regex::Regex;
use std::path::Path;
use tmc_langs_framework::file_util;
use tmc_langs_framework::meta_syntax::{MetaString, MetaSyntaxParser};
use tmc_langs_framework::TmcError;
use walkdir::{DirEntry, WalkDir};

lazy_static! {
    static ref FILES_TO_SKIP_ALWAYS: Regex = Regex::new(r"\.tmcrc|^metadata\.yml$").unwrap();
    static ref NON_TEXT_TYPES: Regex =
        Regex::new("class|jar|exe|jpg|jpeg|gif|png|zip|tar|gz|db|bin|csv|tsv|sqlite3|^$").unwrap();
}

// Filter for hidden directories (directories with names starting with '.')
pub fn is_hidden_dir(entry: &DirEntry) -> bool {
    let skip = entry.metadata().map(|e| e.is_dir()).unwrap_or_default()
        && entry
            .file_name()
            .to_str()
            .map(|s| s.starts_with('.'))
            .unwrap_or_default();
    if skip {
        log::debug!("is hidden dir: {:?}", entry.path());
    }
    skip
}

// Filter for skipping directories on `FILES_TO_SKIP_ALWAYS` or named 'private', and files in a 'test' directory that contain 'Hidden' in their name.
fn on_skip_list(entry: &DirEntry) -> bool {
    // check if entry's filename matchees the skip list or is 'private'
    let entry_file_name = entry.file_name().to_str();
    let on_skip_list = entry_file_name
        .map(|s| FILES_TO_SKIP_ALWAYS.is_match(s) || s == "private")
        .unwrap_or_default();

    // check if the current entry is a file in a "test" directory that contains "Hidden" in its name
    let hidden_in_test = if entry.path().is_file() {
        if let Some(parent) = entry.path().parent().and_then(|p| p.file_name()) {
            parent == "test"
                && entry_file_name
                    .map(|n| n.contains("Hidden"))
                    .unwrap_or_default()
        } else {
            false
        }
    } else {
        false
    };

    let skip = on_skip_list || hidden_in_test;
    if skip {
        log::debug!("on skip list: {:?}", entry.path());
    }
    skip
}

// Filter for skipping directories that contain a '.tmcignore' file
pub fn contains_tmcignore(entry: &DirEntry) -> bool {
    for entry in WalkDir::new(entry.path())
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let is_file = entry.metadata().map(|e| e.is_file()).unwrap_or_default();
        if is_file && entry.file_name() == ".tmcignore" {
            log::debug!("contains .tmcignore: {:?}", entry.path());
            return true;
        }
    }
    false
}

// Copies the entry to the destination. Filters files according to `file_filter`, and filters the contents of each file according to `line_filter`.
fn copy_file(
    file: &Path,
    source_root: &Path,
    dest_root: &Path,
    line_filter: &mut impl Fn(&MetaString) -> bool,
    file_filter: &mut impl Fn(&[MetaString]) -> bool,
) -> Result<(), TmcError> {
    if file.is_dir() {
        return Ok(());
    }
    // get relative path
    let relative_path = file
        .strip_prefix(source_root)
        .unwrap_or_else(|_| Path::new(""));
    let dest_path = dest_root.join(&relative_path);
    if let Some(parent) = dest_path.parent() {
        file_util::create_dir_all(parent)?;
    }
    let extension = file.extension().and_then(|e| e.to_str());
    let is_binary = extension
        .map(|e| NON_TEXT_TYPES.is_match(e))
        .unwrap_or(true); // paths with no extension are interpreted to be binary files
    if is_binary {
        // copy binary files
        log::debug!("copying binary file from {:?} to {:?}", file, dest_path);
        file_util::copy(file, &dest_path)?;
    } else {
        // filter text files
        let source_file = file_util::open_file(file)?;

        let parser = MetaSyntaxParser::new(source_file, extension.unwrap_or_default());
        let parse_result: Result<Vec<_>, _> = parser.collect();
        let parsed = match parse_result {
            Ok(parsed) => parsed,
            Err(err) => return Err(TmcError::SubmissionParse(file.to_path_buf(), Box::new(err))),
        };

        // files that don't pass the filter are skipped
        if !file_filter(&parsed) {
            log::debug!("skipping {} due to file filter", file.display());
            return Ok(());
        }

        // todo: reduce collection?
        // filtered metastrings
        let filtered: Vec<MetaString> = parsed.into_iter().filter(line_filter).collect();
        // collects the filtered lines into a byte vector
        let mut write_lines = vec![];
        for line in filtered {
            match line {
                MetaString::Solution(s) | MetaString::String(s) | MetaString::Stub(s) => {
                    write_lines.extend(s.as_bytes())
                }
                MetaString::SolutionFileMarker => (), // write nothing for solution file markers
            }
        }
        // writes all lines
        log::trace!(
            "filtered file {} to {}",
            file.display(),
            dest_path.display()
        );
        file_util::write_to_file(&mut write_lines.as_slice(), &dest_path)?;
    }
    Ok(())
}

// Processes all files in path, copying files in directories that are not skipped.
fn process_files(
    source: &Path,
    dest_root: &Path,
    mut line_filter: impl Fn(&MetaString) -> bool,
    mut file_filter: impl Fn(&[MetaString]) -> bool,
) -> Result<(), TmcError> {
    log::info!("Project: {:?}", source);

    let walker = WalkDir::new(source).into_iter();
    // silently skips over errors, for example when there's a directory we don't have permissions for
    for entry in walker
        .filter_entry(|e| !is_hidden_dir(e) && !on_skip_list(e) && !contains_tmcignore(e))
        .filter_map(|e| e.ok())
    {
        copy_file(
            entry.path(),
            source,
            dest_root,
            &mut line_filter,
            &mut file_filter,
        )?;
    }
    Ok(())
}

/// Walks through each given path, processing files and copying them into the destination.
///
/// Skips hidden directories, directories that contain a `.tmcignore` file in their root, as well as
/// files matching patterns defined in ```FILES_TO_SKIP_ALWAYS``` and directories and files named ```private```.
///
/// Binary files are copied without extra processing, while text files are parsed to remove solution tags and stubs.
pub fn prepare_solution(exercise_path: &Path, dest_root: &Path) -> Result<(), TmcError> {
    log::debug!(
        "preparing solution from {} to {}",
        exercise_path.display(),
        dest_root.display()
    );

    let line_filter = |meta: &MetaString| !matches!(meta, MetaString::Stub(_));
    let file_filter = |_metas: &[_]| true; // include all files in solution
    process_files(exercise_path, dest_root, line_filter, file_filter)?;
    Ok(())
}

/// Walks through each given path, processing files and copying them into the destination.
///
/// Skips hidden directories, directories that contain a ```.tmcignore``` file in their root, as well as
/// files matching patterns defined in ```FILES_TO_SKIP_ALWAYS``` and directories and files named ```private```.
///
/// Binary files are copied without extra processing, while text files are parsed to remove stub tags and solutions.
///
/// Additionally, copies any shared files with the corresponding language plugins.
pub fn prepare_stub(exercise_path: &Path, dest_root: &Path) -> Result<(), TmcError> {
    log::debug!(
        "preparing stub from {} to {}",
        exercise_path.display(),
        dest_root.display()
    );

    let line_filter = |meta: &MetaString| !matches!(meta, MetaString::Solution(_));
    let file_filter = |metas: &[MetaString]| {
        !metas
            .iter()
            .any(|ms| matches!(ms, MetaString::SolutionFileMarker))
    };
    process_files(&exercise_path, dest_root, line_filter, file_filter)?;
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use std::collections::HashSet;
    use std::fs::File;
    use std::io::{Read, Write};
    use std::path::PathBuf;
    use tempfile::tempdir;
    use tmc_langs_framework::TmcProjectYml;

    const TESTDATA_ROOT: &str = "tests/data";
    const BINARY_REL: &str = "dir/inner/binary.bin";

    fn init() {
        use log::*;
        use simple_logger::*;
        let _ = SimpleLogger::new().with_level(LevelFilter::Debug).init();
    }

    #[test]
    fn prepare_solutions_preserves_structure() {
        init();

        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        prepare_solution(Path::new("tests/data/dir"), temp_path).unwrap();

        let mut dest_files = HashSet::new();
        for entry in walkdir::WalkDir::new(temp_path) {
            let entry = entry.unwrap();
            dest_files.insert(entry.into_path());
        }

        let exp = &temp_path.join("inner/binary.bin");
        assert!(
            dest_files.contains(exp),
            "{:?} did not contain {:?}",
            dest_files,
            exp
        );
        let exp = &temp_path.join("nonbinary.java");
        assert!(
            dest_files.contains(exp),
            "{:?} did not contain {:?}",
            dest_files,
            exp
        );
    }

    #[test]
    fn prepare_solutions_filters_text_files() {
        init();

        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        prepare_solution(Path::new("tests/data/dir"), temp_path).unwrap();

        let exp = &temp_path.join("nonbinary.java");
        let mut file = File::open(exp).unwrap();
        let mut s = String::new();
        file.read_to_string(&mut s).unwrap();
        let expected = r#"public class JavaTestCase {
    public int foo() {
        return 3;
    }

    public void bar() {
        System.out.println("hello");
    }

    public int xoo() {
        return 3;
    }
}
"#;

        let expected = if cfg!(windows) {
            expected.replace('\n', "\r\n")
        } else {
            expected.to_string()
        };
        assert_eq!(s, expected, "expected:\n{:#}\nfound:\n{:#}", expected, s);
    }

    #[test]
    fn prepare_solutions_does_not_filter_binary_files() {
        init();

        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        prepare_solution(Path::new("tests/data/dir"), temp_path).unwrap();

        let original: PathBuf = [TESTDATA_ROOT, BINARY_REL].iter().collect();
        let mut original = File::open(original).unwrap();
        let mut original_s = String::new();
        original.read_to_string(&mut original_s).unwrap();

        let copied = &temp_path.join("inner/binary.bin");
        let mut copied = File::open(copied).unwrap();
        let mut copied_s = String::new();
        copied.read_to_string(&mut copied_s).unwrap();

        assert_eq!(
            original_s, copied_s,
            "expected:\n{:#}\nfound:\n{:#}",
            copied_s, original_s
        );
    }

    #[test]
    fn prepare_solutions_does_not_filter_solution_files() {
        init();

        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        prepare_solution(Path::new("tests/data/dir"), temp_path).unwrap();

        assert!(dbg!(temp_path.join("solution_file.java")).exists());
    }

    #[test]
    fn prepares_stubs() {
        init();

        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        prepare_stub(Path::new("tests/data/dir"), &temp_path).unwrap();

        let exp = &temp_path.join("nonbinary.java");
        let mut file = File::open(exp).unwrap();
        let mut s = String::new();
        file.read_to_string(&mut s).unwrap();
        let mut expected = r#"public class JavaTestCase {

    public void bar() {
    }

    public int xoo() {
        return 0;
    }
}
"#
        .to_string();

        if cfg!(windows) {
            expected = expected.replace("\n", "\r\n");
        }

        assert_eq!(s, expected, "expected:\n{:#}\nfound:\n{:#}", expected, s);
    }

    #[test]
    fn prepare_stubs_filters_solution_files() {
        init();

        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        prepare_stub(Path::new("tests/data/dir"), temp_path).unwrap();

        assert!(!temp_path.join("solution_file.java").exists());
    }

    #[test]
    fn tmc_project_yml_parses() {
        let temp = tempdir().unwrap();
        let mut path = temp.path().to_owned();
        path.push(".tmcproject.yml");
        let mut file = File::create(&path).unwrap();
        file.write_all(
            r#"
extra_student_files:
  - test/StudentTest.java
  - test/OtherTest.java
"#
            .as_bytes(),
        )
        .unwrap();
        let conf = TmcProjectYml::from(&temp.path()).unwrap();
        assert!(conf.extra_student_files[0] == PathBuf::from("test/StudentTest.java"));
        assert!(conf.extra_student_files[1] == PathBuf::from("test/OtherTest.java"));
    }

    #[test]
    fn hides_test_hidden_files() {
        init();

        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        prepare_solution(Path::new("tests/data/dir"), temp_path).unwrap();

        assert!(dbg!(temp_path.join("NotHidden.java")).exists());
        assert!(!dbg!(temp_path.join("ActuallyHidden.java")).exists());
    }
}
