use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

/// If the path contains a component that equals the component argument, returns its parent.
/// Ignores paths that contain __MACOSX in the parent.
pub fn get_parent_of_component_in_path(path: &Path, component: &str) -> Option<PathBuf> {
    if path.components().any(|c| c.as_os_str() == component) {
        let path: PathBuf = path
            .components()
            .take_while(|c| c.as_os_str() != component)
            .collect();
        if !path.components().any(|c| c.as_os_str() == "__MACOSX") {
            return Some(path);
        }
    }
    None
}

/// Returns the path's parent path if the path's name equals the name argument.
/// Ignores paths that contain __MACOSX in the parent.
pub fn get_parent_of_named(path: &Path, name: &str) -> Option<PathBuf> {
    if path.file_name() == Some(OsStr::new(name))
        && !path.components().any(|c| c.as_os_str() == "__MACOSX")
    {
        return Some(path.parent().map(Path::to_path_buf).unwrap_or_default());
    }
    None
}
