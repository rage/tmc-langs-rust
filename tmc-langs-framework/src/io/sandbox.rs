use super::super::Result;
use super::StudentFilePolicy;
use std::path::Path;
use walkdir::WalkDir;

pub fn move_files(
    student_file_policy: Box<dyn StudentFilePolicy>,
    source: &Path,
    target: &Path,
) -> Result<()> {
    for entry in WalkDir::new(source).into_iter().filter_map(|e| e.ok()) {
        if entry.path().is_file() {
            if student_file_policy.is_student_file(entry.path(), source)? {}
        }
    }
    todo!()
}
