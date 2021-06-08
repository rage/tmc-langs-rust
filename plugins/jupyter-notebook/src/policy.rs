//! Contains the Jupyter Notebook student file policy

use std::path::Path;
use tmc_langs_framework::{StudentFilePolicy, TmcProjectYml};

pub struct JupyterNotebookStudentPolicy {
    project_config: TmcProjectYml,
}

impl StudentFilePolicy for JupyterNotebookStudentPolicy {
    fn new_with_project_config(project_config: TmcProjectYml) -> Self
    where
        Self: Sized,
    {
        Self { project_config }
    }

    fn get_project_config(&self) -> &TmcProjectYml {
        &self.project_config
    }

    fn is_student_source_file(path: &Path) -> bool {
        // TODO: Expand

        // all .pynb files are student source files
        path.ends_with(".pynb")
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn pynb_is_source_file() {
        assert!(JupyterNotebookStudentPolicy::is_student_source_file(
            Path::new("some_file.pynb")
        ))
    }
}
