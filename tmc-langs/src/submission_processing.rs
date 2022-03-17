//! Functions for processing submissions.

use crate::error::LangsError;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
};
use tmc_langs_framework::{MetaString, MetaSyntaxParser};
use tmc_langs_util::{deserialize, file_util, FileError};
use walkdir::{DirEntry, WalkDir};

#[allow(clippy::unwrap_used)]
static FILES_TO_SKIP_ALWAYS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\.tmcrc|^metadata\.yml$").unwrap());

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
    process_files(exercise_path, dest_root, line_filter, file_filter)?;
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
    {
        process_file(entry, source, dest_root, &mut line_filter, &mut file_filter)?;
    }
    Ok(())
}

fn process_file(
    entry: DirEntry,
    source: &Path,
    dest_root: &Path,
    line_filter: &mut impl Fn(&MetaString) -> bool,
    file_filter: &mut impl Fn(&[MetaString]) -> bool,
) -> Result<(), LangsError> {
    if entry.path().is_dir() {
        return Ok(());
    }

    let relative_path = entry
        .path()
        .strip_prefix(source)
        .unwrap_or_else(|_| Path::new(""));
    let dest_path = dest_root.join(&relative_path);
    if let Some(extension) = entry.path().extension().and_then(|o| o.to_str()) {
        // todo: stop checking extension twice here and in meta_syntax
        // NOTE: if you change these extensions make sure to change them in meta_syntax.rs as well
        match extension {
            "java" | "c" | "cpp" | "h" | "hpp" | "js" | "css" | "rs" | "qml" | "cs" | "xml"
            | "http" | "html" | "qrc" | "properties" | "py" | "R" | "pro" => {
                // process line by line
                let source_file = file_util::open_file(entry.path())?;
                let iter = LossyFileIterator {
                    file: BufReader::new(source_file),
                };
                if let Some(lines) = process_lines(iter, line_filter, file_filter, extension)
                    .map_err(|e| FileError::FileRead(entry.path().to_path_buf(), e))?
                {
                    // write all lines to target file
                    if let Some(parent) = dest_path.parent() {
                        file_util::create_dir_all(parent)?;
                    }
                    let mut file = BufWriter::new(file_util::create_file(&dest_path)?);
                    for line in lines {
                        file.write_all(line.as_bytes())
                            .map_err(|e| FileError::FileWrite(dest_path.to_path_buf(), e))?;
                    }
                }
            }
            "ipynb" => {
                // process each cell in the notebook
                let file = file_util::open_file(entry.path())?;
                let mut json: Value = deserialize::json_from_reader(file)
                    .map_err(|e| LangsError::DeserializeJson(entry.path().to_path_buf(), e))?;
                let cells = json
                    .get_mut("cells")
                    .and_then(|cs| cs.as_array_mut())
                    .ok_or(LangsError::InvalidNotebook(
                        "Invalid or missing value for 'cells'",
                    ))?;

                for cell in cells {
                    let is_cell_type_code = cell
                        .get("cell_type")
                        .and_then(|c| c.as_str())
                        .map(|c| c == "code")
                        .unwrap_or_default();

                    if is_cell_type_code {
                        // read the source for each code cell
                        let cell_source = cell
                            .get_mut("source")
                            .and_then(|s| s.as_array_mut())
                            .ok_or(LangsError::InvalidNotebook(
                                "Invalid or missing value for 'source'",
                            ))?;
                        let source = cell_source.iter().map(|v| {
                            v.as_str()
                                .map(String::from)
                                .ok_or(LangsError::InvalidNotebook("Invalid value in 'source'"))
                        });

                        let lines: Option<Vec<Value>> =
                            process_lines(source, line_filter, file_filter, extension)?
                                .map(|i| i.map(Value::String).collect());
                        if let Some(lines) = lines {
                            // replace cell source with filtered output
                            *cell_source = lines;
                        } else {
                            // file should be skipped
                            return Ok(());
                        }
                    }
                }
                // writes the JSON with filtered sources to the target path
                file_util::write_to_file(serde_json::to_vec_pretty(&json)?, &dest_path)?;
                log::trace!(
                    "filtered file {} to {}",
                    entry.path().display(),
                    dest_path.display()
                );
            }
            _ => {
                // copy other files as is
                file_util::copy(entry.path(), dest_path)?;
            }
        }
    }
    Ok(())
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

/// Serves the same functionality as BufRead::lines, but uses lossy string conversion.
struct LossyFileIterator {
    file: BufReader<File>,
}

impl Iterator for LossyFileIterator {
    type Item = Result<String, std::io::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut buf = vec![];
        match self.file.read_until(b'\n', &mut buf) {
            Ok(0) => None,
            Ok(_) => Some(Ok(String::from_utf8_lossy(&buf).into_owned())),
            Err(e) => Some(Err(e)),
        }
    }
}

/// Processes the lines from the given iterator according to the filters and extension given.
/// Returns None if the file should be skipped.
fn process_lines<'a, 'b, I, E>(
    line_iterator: I,
    line_filter: &'b mut impl Fn(&MetaString) -> bool,
    file_filter: &'b mut impl Fn(&[MetaString]) -> bool,
    extension: &str,
) -> Result<Option<impl Iterator<Item = String> + 'a>, E>
where
    I: Iterator<Item = Result<String, E>>,
    'b: 'a,
{
    let parser = MetaSyntaxParser::new(line_iterator, extension);
    let parse_result: Result<Vec<_>, _> = parser.collect();
    let parsed = parse_result?;

    // files that don't pass the filter are skipped
    if !file_filter(&parsed) {
        return Ok(None);
    }

    // filter into iterator of strings
    let iter = parsed.into_iter().filter(line_filter).filter_map(|ms| {
        match ms {
            MetaString::Solution(s) | MetaString::String(s) | MetaString::Stub(s) => Some(s),
            MetaString::SolutionFileMarker | MetaString::HiddenFileMarker => None, // write nothing for file markers
            MetaString::Hidden(_) => None, // write nothing for hidden text
        }
    });
    Ok(Some(iter))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use std::{fs::File, io::Write, path::PathBuf};
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

    #[test]
    fn filters_notebooks() {
        init();

        let temp_source = tempfile::tempdir().unwrap();
        file_to(
            &temp_source,
            "hidden.ipynb",
            serde_json::json!({
                "cells": [
                    {
                        "cell_type": "code",
                        "source": [
                            "code"
                        ]
                    },
                    {
                        "cell_type": "code",
                        "source": [
                            "# HIDDEN FILE"
                        ]
                    },
                ]
            })
            .to_string(),
        );
        file_to(
            &temp_source,
            "notebook.ipynb",
            serde_json::json!({
                "cells": [
                    {
                        "cell_type": "other",
                        "source": [
                            "# BEGIN SOLUTION",
                            "solution code",
                            "more code",
                            "# END SOLUTION",
                        ]
                    },
                    {
                        "cell_type": "code",
                        "source": [
                            "code"
                        ]
                    },
                    {
                        "cell_type": "code",
                        "source": [
                            "code",
                            "# BEGIN SOLUTION",
                            "solution code",
                            "more code",
                            "# END SOLUTION",
                            "non-solution code",
                        ]
                    },
                ],
                "some other key": "some other value",
            })
            .to_string(),
        );

        let temp_target = tempfile::tempdir().unwrap();

        prepare_stub(temp_source.path(), temp_target.path()).unwrap();

        assert!(!temp_target.path().join("hidden.ipynb").exists());

        let val: serde_json::Value = deserialize::json_from_reader(
            file_util::open_file(temp_target.path().join("notebook.ipynb")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            val,
            serde_json::json!({
                "cells": [
                    {
                        "cell_type": "other",
                        "source": [
                            "# BEGIN SOLUTION",
                            "solution code",
                            "more code",
                            "# END SOLUTION",
                        ]
                    },
                    {
                        "cell_type": "code",
                        "source": [
                            "code"
                        ]
                    },
                    {
                        "cell_type": "code",
                        "source": [
                            "code",
                            "non-solution code",
                        ]
                    },
                ],
                "some other key": "some other value",
            })
        );
    }
}
