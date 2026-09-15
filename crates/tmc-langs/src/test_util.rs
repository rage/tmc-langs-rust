//! Test-only helpers shared across this crate's unit tests.

use std::sync::LazyLock;
use tempfile::TempDir;
use tmc_langs_util::file_util::set_test_locks_dir_override;

// directory locks would otherwise go to the real user data dir.
pub(crate) fn ensure_isolated_locks_dir() {
    static LOCKS_DIR: LazyLock<TempDir> = LazyLock::new(|| {
        let dir = tempfile::tempdir().expect("failed to create tempdir for test locks dir");
        set_test_locks_dir_override(dir.path().to_path_buf());
        dir
    });
    LazyLock::force(&LOCKS_DIR);
}
