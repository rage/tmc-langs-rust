use super::StudentFilePolicy;
use std::path::Path;

pub struct UnzipResult {}

pub fn student_file_aware_unzip(
    policy: Box<dyn StudentFilePolicy>,
    zip: &Path,
    target: &Path,
) -> UnzipResult {
    todo!()
}

pub fn student_file_aware_zip(
    policy: Box<dyn StudentFilePolicy>,
    root_directory: &Path,
) -> Vec<u8> {
    todo!()
}
