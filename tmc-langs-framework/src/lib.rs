pub mod domain;

use std::path::Path;

pub trait LanguagePlugin {
    fn maybe_copy_shared_stuff(&self, path: &Path);
}
