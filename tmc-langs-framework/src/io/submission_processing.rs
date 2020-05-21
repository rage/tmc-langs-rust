//! Functions for processing submissions.

use crate::policy::StudentFilePolicy;
use crate::{Error, Result};

use crate::domain::meta_syntax::{MetaString, MetaSyntaxParser};
use crate::plugin::LanguagePlugin;
use lazy_static::lazy_static;
use log::{debug, info};
use regex::Regex;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

lazy_static! {
    static ref FILES_TO_SKIP_ALWAYS: Regex =
        Regex::new("\\.tmcrc|metadata\\.yml|(.*)Hidden(.*)").unwrap();
    static ref NON_TEXT_TYPES: Regex =
        Regex::new("class|jar|exe|jpg|jpeg|gif|png|zip|tar|gz|db|bin|csv|tsv|^$").unwrap();
}

/// Moves some of the contents of source to target based on the given policy.
/// For example, a file source/foo.java would be moved to target/foo.java.
pub fn move_files(
    student_file_policy: Box<dyn StudentFilePolicy>,
    source: &Path,
    target: &Path,
) -> Result<()> {
    let tmc_project_yml = student_file_policy.get_tmc_project_yml()?;
    for entry in WalkDir::new(source)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            student_file_policy
                .is_student_file(e.path(), source, &tmc_project_yml)
                .unwrap_or(false)
        })
    {
        if entry.path().is_file() {
            let relative = entry.path().strip_prefix(source).unwrap();
            let target_path = target.join(&relative);
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| Error::CreateDir(parent.to_path_buf(), e))?;
            }
            fs::rename(entry.path(), &target_path).map_err(|e| {
                Error::Rename(entry.path().to_path_buf(), target_path.to_path_buf(), e)
            })?;
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
        debug!("is hidden dir: {:?}", entry.path());
    }
    skip
}

// Filter for skipping directories on `FILES_TO_SKIP_ALWAYS` or named 'private'
fn on_skip_list(entry: &DirEntry) -> bool {
    let skip = entry
        .file_name()
        .to_str()
        .map(|s| FILES_TO_SKIP_ALWAYS.is_match(s) || s == "private")
        .unwrap_or_default();
    if skip {
        debug!("on skip list: {:?}", entry.path());
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
        if is_file {
            if entry.file_name() == ".tmcignore" {
                debug!("contains .tmcignore: {:?}", entry.path());
                return true;
            }
        }
    }
    false
}

// Copies the entry to the destination. Parses and filters text files according to `filter`
fn copy_file<F: Fn(&MetaString) -> bool>(
    entry: &DirEntry,
    source_root: &Path,
    dest_root: &Path,
    filter: &mut F,
) -> Result<()> {
    let is_dir = entry.metadata().map(|e| e.is_dir()).unwrap_or_default();
    if is_dir {
        return Ok(());
    }
    // get relative path
    let relative_path = entry
        .path()
        .strip_prefix(source_root)
        .unwrap_or(Path::new(""));
    let dest_path = dest_root.join(&relative_path);
    dest_path.parent().map_or(Ok(()), fs::create_dir_all)?;
    let extension = entry.path().extension().and_then(|e| e.to_str());
    let is_binary = extension
        .map(|e| NON_TEXT_TYPES.is_match(e))
        .unwrap_or_default();
    if is_binary {
        // copy binary files
        debug!(
            "copying binary file from {:?} to {:?}",
            entry.path(),
            dest_path
        );
        fs::copy(entry.path(), dest_path)?;
    } else {
        // filter text files
        debug!(
            "filtering text file from {:?} to {:?}",
            entry.path(),
            dest_path
        );

        let source_file =
            File::open(entry.path()).map_err(|e| Error::OpenFile(entry.path().to_path_buf(), e))?;

        let mut target_file = File::create(dest_path)
            .map_err(|e| Error::CreateFile(entry.path().to_path_buf(), e))?;

        let parser = MetaSyntaxParser::new(source_file, extension.unwrap_or_default());
        for line in parser {
            let line = line?;
            if filter(&line) {
                debug!("write: {:?}", line);
                target_file.write_all(line.as_str().as_bytes())?;
            } else {
                debug!("skip: {:?}", line);
            }
        }
    }
    Ok(())
}

// Processes all files in path, copying files in directories that are not skipped
fn process_files<F: Fn(&MetaString) -> bool>(
    path: &Path,
    dest_root: &Path,
    mut filter: F,
) -> Result<()> {
    info!("Project: {:?}", path);

    let walker = WalkDir::new(path).into_iter();
    // silently skips over errors, for example when there's a directory we don't have permissions for
    for entry in walker
        .filter_entry(|e| !is_hidden_dir(e) && !on_skip_list(e) && !contains_tmcignore(e))
        .filter_map(|e| e.ok())
    {
        copy_file(&entry, path, dest_root, &mut filter)?;
    }
    Ok(())
}

/// Walks through each given path, processing files and copying them into the destination.
///
/// Skips hidden directories, directories that contain a `.tmcignore` file in their root, as well as
/// files matching patterns defined in ```FILES_TO_SKIP_ALWAYS``` and directories and files named ```private```.
///
/// Binary files are copied without extra processing, while text files are parsed to remove solution tags and stubs.
pub fn prepare_solutions<'a, I: IntoIterator<Item = &'a PathBuf>>(
    exercise_paths: I,
    dest_root: &Path,
) -> Result<()> {
    for path in exercise_paths {
        process_files(path, dest_root, |meta| match meta {
            MetaString::Stub(_) => false,
            _ => true,
        })?;
    }
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
pub fn prepare_stubs(
    exercise_map: HashMap<PathBuf, Box<&dyn LanguagePlugin>>,
    repo_path: &Path,
    dest_root: &Path,
) -> Result<()> {
    for (path, plugin) in exercise_map {
        process_files(&path, dest_root, |meta| match meta {
            MetaString::Solution(_) => false,
            _ => true,
        })?;

        let relative_path = path.strip_prefix(repo_path).unwrap_or(Path::new(""));
        plugin.maybe_copy_shared_stuff(&dest_root.join(relative_path))?;
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::domain::{ExerciseDesc, RunResult, TmcProjectYml};
    use crate::policy::{
        EverythingIsStudentFilePolicy, NothingIsStudentFilePolicy, StudentFilePolicy,
    };
    use std::collections::HashSet;
    use std::io::Read;
    use tempfile::tempdir;

    const TESTDATA_ROOT: &str = "testdata";
    const BINARY_REL: &str = "dir/inner/binary.bin";
    const TEXT_REL: &str = "dir/nonbinary.java";

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn moves_files() {
        let source = tempdir().unwrap();
        let target = tempdir().unwrap();
        let mock_file = "a/b/c/d/e/f/g";
        let file_path = source.path().join(mock_file);
        std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        let _file = std::fs::File::create(&file_path).unwrap();
        move_files(
            Box::new(EverythingIsStudentFilePolicy {}),
            source.path(),
            target.path(),
        )
        .unwrap();

        let mut paths = HashSet::new();
        for entry in WalkDir::new(target.path()) {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                paths.insert(entry.path().to_owned());
            }
        }
        assert!(
            paths.contains(&target.path().join(mock_file)),
            "{:?} did not contain {:?}",
            paths,
            file_path
        );
    }

    #[test]
    fn skips_student_files() {
        let source = tempdir().unwrap();
        let target = tempdir().unwrap();
        let mock_file = "a/b/c/d/e/f/g";
        let file_path = source.path().join(mock_file);
        std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        let _file = std::fs::File::create(&file_path).unwrap();
        move_files(
            Box::new(NothingIsStudentFilePolicy {}),
            source.path(),
            target.path(),
        )
        .unwrap();

        let mut paths = HashSet::new();
        for entry in WalkDir::new(target.path()) {
            let entry = entry.unwrap();
            if entry.path().is_file() {
                paths.insert(entry.path().to_owned());
            }
        }
        assert!(paths.is_empty());
    }

    #[test]
    fn prepare_solutions_preserves_structure() {
        init();

        let mut exercise_set = HashSet::new();
        exercise_set.insert(TESTDATA_ROOT.into());
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        prepare_solutions(&exercise_set, temp_path).unwrap();

        let mut dest_files = HashSet::new();
        for entry in walkdir::WalkDir::new(temp_path) {
            let entry = entry.unwrap();
            dest_files.insert(entry.into_path());
        }

        let exp = &temp_path.join(BINARY_REL);
        assert!(
            dest_files.contains(exp),
            "{:?} did not contain {:?}",
            dest_files,
            exp
        );
        let exp = &temp_path.join(TEXT_REL);
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

        let mut exercise_set = HashSet::new();
        exercise_set.insert(TESTDATA_ROOT.into());
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        prepare_solutions(&exercise_set, temp_path).unwrap();

        let exp = &temp_path.join(TEXT_REL);
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
        assert_eq!(s, expected, "expected:\n{:#}\nfound:\n{:#}", expected, s);
    }

    #[test]
    fn prepare_solutions_does_not_filter_binary_files() {
        init();

        let mut exercise_set = HashSet::new();
        exercise_set.insert(TESTDATA_ROOT.into());
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        prepare_solutions(&exercise_set, temp_path).unwrap();

        let original: PathBuf = [TESTDATA_ROOT, BINARY_REL].iter().collect();
        let mut original = File::open(original).unwrap();
        let mut original_s = String::new();
        original.read_to_string(&mut original_s).unwrap();

        let copied = &temp_path.join(BINARY_REL);
        let mut copied = File::open(copied).unwrap();
        let mut copied_s = String::new();
        copied.read_to_string(&mut copied_s).unwrap();

        assert_eq!(
            original_s, copied_s,
            "expected:\n{:#}\nfound:\n{:#}",
            copied_s, original_s
        );
    }

    struct MockPlugin {}

    struct MockPolicy {}

    impl StudentFilePolicy for MockPolicy {
        fn get_config_file_parent_path(&self) -> &Path {
            unimplemented!()
        }
        fn is_student_source_file(&self, path: &Path) -> bool {
            unimplemented!()
        }
    }

    impl LanguagePlugin for MockPlugin {
        fn get_student_file_policy(&self, project_path: &Path) -> Box<dyn StudentFilePolicy> {
            unimplemented!()
        }
        fn get_plugin_name(&self) -> &'static str {
            unimplemented!()
        }

        fn scan_exercise(&self, _path: &Path, _exercise_name: String) -> Result<ExerciseDesc> {
            unimplemented!()
        }

        fn run_tests(&self, _path: &Path) -> Result<RunResult> {
            unimplemented!()
        }

        fn check_code_style(
            &self,
            _path: &Path,
            _locale: isolang::Language,
        ) -> Option<tmc_langs_abstraction::ValidationResult> {
            unimplemented!()
        }

        fn is_exercise_type_correct(&self, path: &Path) -> bool {
            !path.to_str().unwrap().contains("ignored")
        }

        fn clean(&self, _path: &Path) -> Result<()> {
            unimplemented!()
        }
    }

    #[test]
    fn prepares_stubs() {
        init();

        let mut exercise_map = HashMap::new();
        let mut plugin = MockPlugin {};
        exercise_map.insert(
            TESTDATA_ROOT.into(),
            Box::new(&plugin as &dyn LanguagePlugin),
        );
        let temp = tempdir().unwrap();
        let temp_path = temp.path();

        let repo_path: PathBuf = "".into();
        prepare_stubs(exercise_map, &repo_path, &temp_path).unwrap();

        let exp = &temp_path.join(TEXT_REL);
        let mut file = File::open(exp).unwrap();
        let mut s = String::new();
        file.read_to_string(&mut s).unwrap();
        let expected = r#"public class JavaTestCase {

    public void bar() {
    }

    public int xoo() {
        return 0;
    }
}
"#;

        assert_eq!(s, expected, "expected:\n{:#}\nfound:\n{:#}", expected, s);
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
}
