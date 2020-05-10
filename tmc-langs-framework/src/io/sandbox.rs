use super::super::Result;
use super::StudentFilePolicy;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

/// Moves some of the contents of source to target based on the given policy.
/// For example, a file source/foo.java would be moved to target/foo.java.
pub fn move_files(
    student_file_policy: Box<dyn StudentFilePolicy>,
    source: &Path,
    target: &Path,
) -> Result<()> {
    for entry in WalkDir::new(source)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            student_file_policy
                .is_student_file(e.path(), source)
                .unwrap_or(false)
        })
    {
        if entry.path().is_file() {
            let relative = entry.path().strip_prefix(source).unwrap();
            let target_path = target.join(&relative);
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(entry.path(), target_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::super::{EverythingIsStudentFilePolicy, NothingIsStudentFilePolicy};
    use super::*;
    use std::collections::HashSet;
    use tempdir::TempDir;

    #[test]
    fn moves_files() {
        let source = TempDir::new("moves_files_source").unwrap();
        let target = TempDir::new("moves_files_target").unwrap();
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
        let source = TempDir::new("moves_files_source").unwrap();
        let target = TempDir::new("moves_files_target").unwrap();
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
}
