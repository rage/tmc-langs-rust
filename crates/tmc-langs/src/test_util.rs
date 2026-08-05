//! Test-only helpers shared across this crate's unit tests.

use std::sync::LazyLock;
use tempfile::TempDir;
use tmc_langs_util::file_util::LOCKS_DIR_ENV;

// directory locks would otherwise go to the real user data dir. The env var is
// process-global, so this has to happen once before any test locks a directory.
pub(crate) fn ensure_isolated_locks_dir() {
    static LOCKS_DIR: LazyLock<TempDir> = LazyLock::new(|| {
        let dir = tempfile::tempdir().expect("failed to create tempdir for test locks dir");
        // SAFETY: LazyLock runs this at most once, before any test reads the env var
        unsafe { std::env::set_var(LOCKS_DIR_ENV, dir.path()) };
        dir
    });
    LazyLock::force(&LOCKS_DIR);
}
