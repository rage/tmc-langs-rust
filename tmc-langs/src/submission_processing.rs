//! Functions for processing submissions.

use crate::error::LangsError;
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::Path;
use tmc_langs_framework::{MetaString, MetaSyntaxParser};
use tmc_langs_util::file_util;
use walkdir::{DirEntry, WalkDir};

#[allow(clippy::clippy::unwrap_used)]
static FILES_TO_SKIP_ALWAYS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\.tmcrc|^metadata\.yml$").unwrap());
#[allow(clippy::clippy::unwrap_used)]
static NON_TEXT_TYPES: Lazy<Regex> = Lazy::new(|| {
    Regex::new("class|jar|exe|jpg|jpeg|gif|png|zip|tar|gz|db|bin|csv|tsv|sqlite3|^$").unwrap()
});

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

    // check if the current entry is a file that contains "Hidden" in its name in a directory that contains "test" in its name
    let hidden_in_test = if entry.path().is_file() {
        let in_test = entry
            .path()
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|f| f.to_str())
            .map(|f| f.contains("test"))
            .unwrap_or_default();
        let contains_hidden = entry_file_name
            .map(|n| n.contains("Hidden"))
            .unwrap_or_default();
        in_test && contains_hidden
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
) -> Result<(), LangsError> {
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
            Err(err) => {
                return Err(LangsError::SubmissionParse(
                    file.to_path_buf(),
                    Box::new(LangsError::Tmc(err)),
                ))
            }
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
                MetaString::SolutionFileMarker | MetaString::HiddenFileMarker => (), // write nothing for file markers
                MetaString::Hidden(_) => (), // write nothing for hidden text
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
) -> Result<(), LangsError> {
    log::info!("Project: {:?}", source);

    let walker = WalkDir::new(source).min_depth(1).into_iter();
    // silently skips over errors, for example when there's a directory we don't have permissions for
    for entry in walker
        .filter_entry(|e| !is_hidden_dir(e) && !on_skip_list(e) && !contains_tmcignore(e))
        .filter_map(|e| e.ok())
        .into_iter()
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

/// Note: used by tmc-server.
/// Walks through each given path, processing files and copying them into the destination.
///
/// Skips hidden directories, directories that contain a `.tmcignore` file in their root, as well as
/// files matching patterns defined in ```FILES_TO_SKIP_ALWAYS``` and directories and files named ```private```.
///
/// Binary files are copied without extra processing, while text files are parsed to remove solution tags and stubs.
pub fn prepare_solution(exercise_path: &Path, dest_root: &Path) -> Result<(), LangsError> {
    log::debug!(
        "preparing solution from {} to {}",
        exercise_path.display(),
        dest_root.display()
    );

    let line_filter = |meta: &MetaString| {
        !matches!(meta, MetaString::Stub(_)) && !matches!(meta, MetaString::Hidden(_))
        // hide stub and hidden lines
    };
    let file_filter = |metas: &[MetaString]| {
        !metas
            .iter()
            .any(|ms| matches!(ms, MetaString::HiddenFileMarker)) // exclude hidden files
    };
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
pub fn prepare_stub(exercise_path: &Path, dest_root: &Path) -> Result<(), LangsError> {
    log::debug!(
        "preparing stub from {} to {}",
        exercise_path.display(),
        dest_root.display()
    );

    let line_filter = |meta: &MetaString| {
        !matches!(meta, MetaString::Solution(_)) && !matches!(meta, MetaString::Hidden(_))
        // exclude solution and hidden lines
    };
    let file_filter = |metas: &[MetaString]| {
        !metas.iter().any(|ms| {
            matches!(ms, MetaString::SolutionFileMarker) // exclude solution files
                || matches!(ms, MetaString::HiddenFileMarker) // exclude hidden files
        })
    };
    process_files(&exercise_path, dest_root, line_filter, file_filter)?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::clippy::unwrap_used)]
mod test {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;
    use tmc_langs_framework::TmcProjectYml;

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

    #[test]
    fn prepare_solutions_preserves_structure() {
        init();

        let temp_source = tempfile::tempdir().unwrap();
        file_to(&temp_source, "inner/binary.bin", "");
        file_to(&temp_source, "File.java", "");

        let temp_target = tempfile::tempdir().unwrap();

        prepare_solution(temp_source.path(), temp_target.path()).unwrap();

        assert!(temp_target.path().join("inner/binary.bin").exists());
        assert!(temp_target.path().join("File.java").exists());
    }

    #[test]
    fn prepare_solutions_filters_text_files() {
        init();

        let temp_source = tempfile::tempdir().unwrap();
        file_to(
            &temp_source,
            "Test.java",
            r#"public class JavaTestCase {
    // BEGIN SOLUTION
    public int foo() {
        return 3;
    }
    // END SOLUTION

    public void bar() {
        // BEGIN SOLUTION
        System.out.println("hello");
        // END SOLUTION
    }

    public int xoo() {
        // BEGIN SOLUTION
        return 3;
        // END SOLUTION
        // STUB: return 0;
    }
}
"#,
        );

        let temp_target = tempfile::tempdir().unwrap();

        prepare_solution(temp_source.path(), temp_target.path()).unwrap();

        let s = file_util::read_file_to_string(temp_target.path().join("Test.java")).unwrap();
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

        assert_eq!(s, expected, "expected:\n{:#}\nfound:\n{:#}", expected, s);
    }

    #[test]
    fn prepare_solutions_does_not_filter_binary_files() {
        init();

        let temp_source = tempfile::tempdir().unwrap();

        let contents = r#"public class JavaTestCase {
    // BEGIN SOLUTION
    public int foo() {
        return 3;
    }
    // END SOLUTION

    public void bar() {
        // BEGIN SOLUTION
        System.out.println("hello");
        // END SOLUTION
    }

    public int xoo() {
        // BEGIN SOLUTION
        return 3;
        // END SOLUTION
        // STUB: return 0;
    }
}
"#;

        file_to(&temp_source, "Test.bin", contents);

        let temp_target = tempfile::tempdir().unwrap();

        prepare_stub(temp_source.path(), temp_target.path()).unwrap();

        let s = file_util::read_file_to_string(temp_target.path().join("Test.bin")).unwrap();

        assert_eq!(s, contents, "expected:\n{:#}\nfound:\n{:#}", contents, s);
    }

    #[test]
    fn prepare_solutions_does_not_filter_solution_files() {
        init();

        let temp_source = tempfile::tempdir().unwrap();
        file_to(
            &temp_source,
            "Solution.java",
            r#"// SOLUTION FILE
class SomeClass {}
"#,
        );
        file_to(
            &temp_source,
            "NonSolution.java",
            r#"
class SomeClass {}
"#,
        );

        let temp_target = tempfile::tempdir().unwrap();

        prepare_solution(temp_source.path(), temp_target.path()).unwrap();

        assert!(dbg!(temp_source.path().join("Solution.java")).exists());
        assert!(dbg!(temp_source.path().join("NonSolution.java")).exists());
    }

    #[test]
    fn prepares_stubs() {
        init();

        let temp_source = tempfile::tempdir().unwrap();
        file_to(
            &temp_source,
            "Test.java",
            r#"public class JavaTestCase {
    // BEGIN SOLUTION
    public int foo() {
        return 3;
    }
    // END SOLUTION

    public void bar() {
        // BEGIN SOLUTION
        System.out.println("hello");
        // END SOLUTION
    }

    public int xoo() {
        // BEGIN SOLUTION
        return 3;
        // END SOLUTION
        // STUB: return 0;
    }
}
"#,
        );

        let temp_target = tempfile::tempdir().unwrap();

        prepare_stub(temp_source.path(), temp_target.path()).unwrap();

        let s = file_util::read_file_to_string(temp_target.path().join("Test.java")).unwrap();
        let expected = r#"public class JavaTestCase {

    public void bar() {
    }

    public int xoo() {
        return 0;
    }
}
"#
        .to_string();

        assert_eq!(s, expected, "expected:\n{:#}\nfound:\n{:#}", expected, s);
    }

    #[test]
    fn prepare_stubs_filters_solution_files() {
        init();

        let temp_source = tempfile::tempdir().unwrap();
        file_to(&temp_source, "NonSolution.java", "something something");
        file_to(&temp_source, "SolutionFile.java", "// SOLUTION FILE");

        let temp_target = tempfile::tempdir().unwrap();

        prepare_stub(temp_source.path(), temp_target.path()).unwrap();

        assert!(temp_target.path().join("NonSolution.java").exists());
        assert!(!temp_target.path().join("SolutionFile.java").exists());
    }

    #[test]
    fn tmc_project_yml_parses() {
        let temp = tempfile::tempdir().unwrap();
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
        let conf = TmcProjectYml::load_or_default(&temp.path()).unwrap();
        assert!(conf.extra_student_files[0] == PathBuf::from("test/StudentTest.java"));
        assert!(conf.extra_student_files[1] == PathBuf::from("test/OtherTest.java"));
    }

    #[test]
    fn hides_test_hidden_files_in_test() {
        init();

        let temp_source = tempfile::tempdir().unwrap();
        file_to(&temp_source, "NotHidden", "");
        file_to(&temp_source, "test/ActuallyHidden", "");

        let temp_target = tempfile::tempdir().unwrap();

        prepare_solution(temp_source.path(), temp_target.path()).unwrap();

        assert!(dbg!(temp_source.path().join("NotHidden")).exists());
        assert!(!dbg!(temp_source.path().join("ActuallyHidden")).exists());
    }

    #[test]
    fn solution_filters_hidden_files() {
        init();

        let temp_source = tempfile::tempdir().unwrap();
        file_to(
            &temp_source,
            "H.java",
            r"// HIDDEN FILE
etc etc",
        );
        file_to(&temp_source, "NonH.java", "etc etc");

        let temp_target = tempfile::tempdir().unwrap();

        prepare_solution(temp_source.path(), temp_target.path()).unwrap();

        assert!(!temp_target.path().join("H.java").exists());
        assert!(temp_target.path().join("NonH.java").exists());
    }

    #[test]
    fn stub_filters_hidden_files() {
        init();

        let temp_source = tempfile::tempdir().unwrap();
        file_to(
            &temp_source,
            "H.java",
            r"// HIDDEN FILE
etc etc",
        );
        file_to(&temp_source, "NonH.java", "etc etc");

        let temp_target = tempfile::tempdir().unwrap();

        prepare_stub(temp_source.path(), temp_target.path()).unwrap();

        assert!(!temp_target.path().join("H.java").exists());
        assert!(temp_target.path().join("NonH.java").exists());
    }

    #[test]
    fn solution_filters_hidden_lines() {
        init();

        let temp_source = tempfile::tempdir().unwrap();
        file_to(
            &temp_source,
            "ContainsHidden.java",
            r"etc etc
// BEGIN HIDDEN
hidden!
// END HIDDEN
etc etc",
        );

        let temp_target = tempfile::tempdir().unwrap();

        prepare_solution(temp_source.path(), temp_target.path()).unwrap();

        let s =
            file_util::read_file_to_string(temp_target.path().join("ContainsHidden.java")).unwrap();

        assert_eq!(
            s,
            r"etc etc
etc etc"
        );
    }

    #[test]
    fn stub_filters_hidden_lines() {
        init();

        let temp_source = tempfile::tempdir().unwrap();
        file_to(
            &temp_source,
            "ContainsHidden.java",
            r"etc etc
// BEGIN HIDDEN
hidden!
// END HIDDEN
etc etc",
        );

        let temp_target = tempfile::tempdir().unwrap();

        prepare_stub(temp_source.path(), temp_target.path()).unwrap();

        let s =
            file_util::read_file_to_string(temp_target.path().join("ContainsHidden.java")).unwrap();

        assert_eq!(
            s,
            r"etc etc
etc etc"
        );
    }
}
