mod meta_syntax;

use super::LanguagePlugin;
use super::Result;
use super::StudentFilePolicy;
use lazy_static::lazy_static;
use log::{debug, info};
use meta_syntax::{MetaString, MetaSyntaxParser};
use regex::Regex;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
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

#[derive(Debug)]
pub struct TestDesc {
    pub name: String,
    pub points: Vec<String>,
}

impl TestDesc {
    pub fn new(name: String, points: Vec<String>) -> Self {
        Self { name, points }
    }
}

#[derive(Debug, Deserialize)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub points: Vec<String>,
    pub message: String,
    #[serde(default)]
    pub exception: Vec<String>,
}

#[derive(Debug)]
pub struct ExerciseDesc {
    pub name: String,
    pub tests: Vec<TestDesc>,
}

impl ExerciseDesc {
    pub fn new(name: String, tests: Vec<TestDesc>) -> Self {
        Self { name, tests }
    }
}

#[derive(Debug)]
pub struct RunResult {
    pub status: RunStatus,
    pub test_results: Vec<TestResult>,
    pub logs: HashMap<String, Vec<u8>>,
}

impl RunResult {
    pub fn new(
        status: RunStatus,
        test_results: Vec<TestResult>,
        logs: HashMap<String, Vec<u8>>,
    ) -> Self {
        Self {
            status,
            test_results,
            logs,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RunStatus {
    Passed,
    TestsFailed,
    CompileFailed,
    TestrunInterrupted,
    GenericError,
}

#[derive(Debug)]
pub struct ExercisePackagingConfiguration {
    pub student_file_paths: HashSet<PathBuf>,
    pub exercise_file_paths: HashSet<PathBuf>,
}

impl ExercisePackagingConfiguration {
    pub fn new(
        student_file_paths: HashSet<PathBuf>,
        exercise_file_paths: HashSet<PathBuf>,
    ) -> Self {
        Self {
            student_file_paths,
            exercise_file_paths,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct TmcProjectYml {
    #[serde(default)]
    pub extra_student_files: Vec<PathBuf>,
    #[serde(default)]
    pub extra_exercise_files: Vec<PathBuf>,
    #[serde(default)]
    pub force_update: Vec<PathBuf>,
}

impl TmcProjectYml {
    pub fn from(project_dir: &Path) -> Result<Self> {
        let mut config_path = project_dir.to_owned();
        config_path.push(".tmcproject.yml");
        let file = File::open(config_path)?;
        Ok(serde_yaml::from_reader(file)?)
    }
}

// Filter for hidden directories (directories with names starting with '.')
fn is_hidden_dir(entry: &DirEntry) -> bool {
    let skip = entry.metadata().map(|e| e.is_dir()).unwrap_or(false)
        && entry
            .file_name()
            .to_str()
            .map(|s| s.starts_with("."))
            .unwrap_or(false);
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
        .unwrap_or(false);
    if skip {
        debug!("on skip list: {:?}", entry.path());
    }
    skip
}

// Filter for skipping directories that contain a '.tmcignore' file
fn contains_tmcignore(entry: &DirEntry) -> bool {
    for entry in WalkDir::new(entry.path())
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let is_file = entry.metadata().map(|e| e.is_file()).unwrap_or(false);
        if is_file {
            let is_tmcignore = entry
                .file_name()
                .to_str()
                .map(|s| s == ".tmcignore")
                .unwrap_or(false);
            if is_tmcignore {
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
    skip_parts: usize,
    dest_root: &Path,
    filter: &mut F,
) -> io::Result<()> {
    let is_dir = entry.metadata().map(|e| e.is_dir()).unwrap_or(false);
    if is_dir {
        return Ok(());
    }
    // get relative path
    let relative_path = entry
        .path()
        .into_iter()
        .skip(skip_parts)
        .collect::<PathBuf>();
    let dest_path = dest_root.join(&relative_path);
    dest_path
        .parent()
        .map_or(Ok(()), |p| fs::create_dir_all(p))?;
    let extension = entry
        .path()
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let is_binary = NON_TEXT_TYPES.is_match(extension);
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
        let source_file = File::open(entry.path())?;
        let mut target_file = File::create(dest_path)?;
        let parser = MetaSyntaxParser::new(source_file, extension);
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
) -> io::Result<()> {
    info!("Project: {:?}", path);

    let skip_parts = path.components().count(); // used to get the relative path of files
    let walker = WalkDir::new(path).into_iter();
    // silently skips over errors, for example when there's a directory we don't have permissions for
    for entry in walker
        .filter_entry(|e| !is_hidden_dir(e) && !on_skip_list(e) && !contains_tmcignore(e))
        .filter_map(|e| e.ok())
    {
        copy_file(&entry, skip_parts, dest_root, &mut filter)?;
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
) -> io::Result<()> {
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
    exercise_map: HashMap<PathBuf, Box<dyn LanguagePlugin>>,
    repo_path: &Path,
    dest_root: &Path,
) -> io::Result<()> {
    for (path, plugin) in exercise_map {
        process_files(&path, dest_root, |meta| match meta {
            MetaString::Solution(_) => false,
            _ => true,
        })?;

        let relative_path = if repo_path.components().count() < path.iter().count() {
            let skip_count = repo_path.components().count();
            path.components().into_iter().skip(skip_count).collect()
        } else {
            PathBuf::from("")
        };
        plugin.maybe_copy_shared_stuff(&dest_root.join(relative_path));
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use std::collections::HashSet;
    use std::io::Read;
    use tempdir::TempDir;

    const TESTDATA_ROOT: &str = "testdata";
    const BINARY_REL: &str = "dir/inner/binary.bin";
    const TEXT_REL: &str = "dir/nonbinary.java";

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn prepare_solutions_preserves_structure() {
        init();

        let mut exercise_set = HashSet::new();
        exercise_set.insert(TESTDATA_ROOT.into());
        let temp = TempDir::new("prepare_solutions_preserves_structure").unwrap();
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
        let temp = TempDir::new("prepare_solutions_filters_text_files").unwrap();
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
        let temp = TempDir::new("prepare_solutions_does_not_filter_binary_files").unwrap();
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
            todo!()
        }
        fn is_student_source_file(&self, path: &Path) -> bool {
            todo!()
        }
    }

    impl LanguagePlugin for MockPlugin {
        fn get_student_file_policy(&self, project_path: &Path) -> Box<dyn StudentFilePolicy> {
            todo!()
        }
        fn get_plugin_name(&self) -> &'static str {
            todo!()
        }

        fn scan_exercise(&self, _path: &Path, _exercise_name: String) -> Result<ExerciseDesc> {
            todo!()
        }

        fn run_tests(&self, _path: &Path) -> RunResult {
            todo!()
        }

        fn check_code_style(
            &self,
            _path: &Path,
            _locale: isolang::Language,
        ) -> Option<tmc_langs_abstraction::ValidationResult> {
            todo!()
        }

        fn is_exercise_type_correct(&self, path: &Path) -> bool {
            !path.to_str().unwrap().contains("ignored")
        }

        fn clean(&self, _path: &Path) {
            todo!()
        }
    }

    #[test]
    fn prepares_stubs() {
        init();

        let mut exercise_map = HashMap::new();
        let mut plugin = MockPlugin {};
        exercise_map.insert(
            TESTDATA_ROOT.into(),
            Box::new(plugin) as Box<dyn LanguagePlugin>,
        );
        let temp = TempDir::new("prepares_stubs").unwrap();
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
        let temp = tempdir::TempDir::new("configuration_parses").unwrap();
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
