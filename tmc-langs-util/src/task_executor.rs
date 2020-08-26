//! Module for calling different tasks of TMC-langs language plug-ins.

mod submission_packaging;
mod tar_helper;

use crate::error::UtilError;
use crate::{ExerciseDesc, ExercisePackagingConfiguration, RunResult, ValidationResult};
use std::path::{Path, PathBuf};
pub use submission_packaging::{OutputFormat, TmcParams};
use tmc_langs_csharp::CSharpPlugin;
use tmc_langs_framework::{
    domain::TmcProjectYml,
    io::{self, submission_processing},
    plugin::{Language, LanguagePlugin},
    policy::NothingIsStudentFilePolicy,
    TmcError,
};
use tmc_langs_java::AntPlugin;
use tmc_langs_java::MavenPlugin;
use tmc_langs_make::MakePlugin;
use tmc_langs_notests::NoTestsPlugin;
use tmc_langs_python3::Python3Plugin;
use tmc_langs_r::RPlugin;
use walkdir::WalkDir;

/// See `domain::prepare_solutions`.
pub fn prepare_solutions<'a, I: IntoIterator<Item = &'a PathBuf>>(
    exercise_paths: I,
    dest_root: &Path,
) -> Result<(), UtilError> {
    io::submission_processing::prepare_solutions(exercise_paths, dest_root)?;
    Ok(())
}

/// See `LanguagePlugin::prepare_stubs`.
pub fn prepare_stubs<I: IntoIterator<Item = PathBuf>>(
    exercise_paths: I,
    repo_path: &Path,
    dest_path: &Path,
) -> Result<(), UtilError> {
    for exercise_path in exercise_paths {
        let plugin = get_language_plugin(&exercise_path)?;
        plugin.prepare_stub(&exercise_path, repo_path, dest_path)?;
    }
    Ok(())
}

/// Takes a submission zip and turns it into an archive suitable for
/// further processing by among other things resetting the test files
pub fn prepare_submission(
    zip_path: &Path,
    target_path: &Path,
    toplevel_dir_name: Option<String>,
    tmc_params: TmcParams,
    clone_path: &Path,
    stub_zip_path: Option<&Path>,
    output_format: OutputFormat,
) -> Result<(), UtilError> {
    submission_packaging::prepare_submission(
        zip_path,
        target_path,
        toplevel_dir_name,
        tmc_params,
        clone_path,
        stub_zip_path,
        output_format,
    )
}

/// See `LanguagePlugin::check_code_style`.
pub fn run_check_code_style(
    path: &Path,
    locale: Language,
) -> Result<Option<ValidationResult>, UtilError> {
    Ok(get_language_plugin(path)?.check_code_style(path, locale)?)
}

/// See `LanguagePlugin::run_tests`.
pub fn run_tests(path: &Path) -> Result<RunResult, UtilError> {
    Ok(get_language_plugin(path)?.run_tests(&path)?)
}

/// See `LanguagePlugin::scan_exercise`.
pub fn scan_exercise(path: &Path, exercise_name: String) -> Result<ExerciseDesc, UtilError> {
    Ok(get_language_plugin(path)?.scan_exercise(path, exercise_name)?)
}

/// Tries to find a language plugin for the path, returning `true` if one is found.
pub fn is_exercise_root_directory(path: &Path) -> bool {
    get_language_plugin(path).is_ok()
}

/// Finds the correct language plug-in for the given exercise path and calls `LanguagePlugin::extract_project`,
/// If no language plugin matches, see `extract_project_overwrite`.
pub fn extract_project(
    compressed_project: &Path,
    target_location: &Path,
    clean: bool,
) -> Result<(), UtilError> {
    if let Ok(plugin) = get_language_plugin(target_location) {
        plugin.extract_project(compressed_project, target_location, clean)?;
    } else {
        log::debug!(
            "no matching language plugin found for {}, overwriting",
            compressed_project.display()
        );
        extract_project_overwrite(compressed_project, target_location)?;
    }
    Ok(())
}

/// Extract a given archive file containing a compressed project to a target location.
/// This will overwrite any existing files.
pub fn extract_project_overwrite(
    compressed_project: &Path,
    target_location: &Path,
) -> Result<(), UtilError> {
    io::tmc_zip::unzip(
        NothingIsStudentFilePolicy {},
        compressed_project,
        target_location,
    )?;
    Ok(())
}

/// Extracts a project archive, only taking out files classified as student files.
pub fn extract_student_files(
    compressed_project: &Path,
    target_location: &Path,
) -> Result<(), UtilError> {
    if let Ok(plugin) = get_language_plugin(target_location) {
        plugin.extract_student_files(compressed_project, target_location)?;
    } else {
        log::debug!(
            "no matching language plugin found for {}, overwriting",
            compressed_project.display()
        );
        extract_project_overwrite(compressed_project, target_location)?;
    }
    Ok(())
}

/// See `LanguagePlugin::compress_project`.
pub fn compress_project(path: &Path) -> Result<Vec<u8>, UtilError> {
    Ok(get_language_plugin(path)?.compress_project(path)?)
}

/// See `LanguagePlugin::get_exercise_packaging_configuration`.
pub fn get_exercise_packaging_configuration(
    path: &Path,
) -> Result<ExercisePackagingConfiguration, UtilError> {
    Ok(get_language_plugin(path)?.get_exercise_packaging_configuration(path)?)
}

/// Creates a tarball that can be submitted to TMC-sandbox.
// TODO: used?
pub fn compress_tar_for_submitting(
    project_dir: &Path,
    tmc_langs: &Path,
    tmcrun: &Path,
    target_location: &Path,
) -> Result<(), UtilError> {
    tar_helper::create_tar_from_project(project_dir, tmc_langs, tmcrun, target_location)?;
    Ok(())
}

/// See `LanguagePlugin::clean`.
pub fn clean(path: &Path) -> Result<(), UtilError> {
    get_language_plugin(path)?.clean(path)?;
    Ok(())
}

/// Recursively searches for valid exercise directories in the path.
pub fn find_exercise_directories(exercise_path: &Path) -> Vec<PathBuf> {
    let mut paths = vec![];
    for entry in WalkDir::new(exercise_path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(submission_processing::is_hidden_dir)
        .filter(|e| e.file_name() == "private")
        .filter(submission_processing::contains_tmcignore)
    {
        // TODO: Java implementation doesn't scan root directories
        if is_exercise_root_directory(entry.path()) {
            paths.push(entry.into_path())
        }
    }
    paths
}

// enum containing all the plugins
#[impl_enum::with_methods(
    fn clean(&self, path: &Path) -> Result<(), TmcError> {}
    fn get_exercise_packaging_configuration(&self, path: &Path) -> Result<ExercisePackagingConfiguration, TmcError> {}
    fn compress_project(&self, path: &Path) -> Result<Vec<u8>, TmcError> {}
    fn extract_project(&self, compressed_project: &Path, target_location: &Path, clean: bool) -> Result<(), TmcError> {}
    fn extract_student_files(&self, compressed_project: &Path, target_location: &Path) -> Result<(), TmcError> {}
    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc, TmcError> {}
    fn run_tests(&self, path: &Path) -> Result<RunResult, TmcError> {}
    fn check_code_style(&self, path: &Path, locale: Language) -> Result<Option<ValidationResult>, TmcError> {}
    fn prepare_stub(&self, exercise_path: &Path, repo_path: &Path, dest_path: &Path) -> Result<(), TmcError> {}
)]
enum Plugin {
    CSharp(CSharpPlugin),
    Make(MakePlugin),
    Maven(MavenPlugin),
    NoTests(NoTestsPlugin),
    Python3(Python3Plugin),
    R(RPlugin),
    Ant(AntPlugin),
}

// Get language plugin for the given path.
fn get_language_plugin(path: &Path) -> Result<Plugin, TmcError> {
    if CSharpPlugin::is_exercise_type_correct(path) {
        let csharp = CSharpPlugin::new();
        log::info!("Detected project as {}", CSharpPlugin::PLUGIN_NAME);
        Ok(Plugin::CSharp(csharp))
    } else if MakePlugin::is_exercise_type_correct(path) {
        let make = MakePlugin::new();
        log::info!("Detected project as {}", MakePlugin::PLUGIN_NAME);
        Ok(Plugin::Make(make))
    } else if NoTestsPlugin::is_exercise_type_correct(path) {
        log::info!("Detected project as {}", NoTestsPlugin::PLUGIN_NAME);
        Ok(Plugin::NoTests(NoTestsPlugin::new()))
    } else if Python3Plugin::is_exercise_type_correct(path) {
        log::info!("Detected project as {}", Python3Plugin::PLUGIN_NAME);
        Ok(Plugin::Python3(Python3Plugin::new()))
    } else if RPlugin::is_exercise_type_correct(path) {
        log::info!("Detected project as {}", RPlugin::PLUGIN_NAME);
        Ok(Plugin::R(RPlugin::new()))
    } else if MavenPlugin::is_exercise_type_correct(path) {
        log::info!("Detected project as {}", MavenPlugin::PLUGIN_NAME);
        Ok(Plugin::Maven(MavenPlugin::new()?))
    } else if AntPlugin::is_exercise_type_correct(path) {
        // TODO: currently, ant needs to be last because any project with src and test are recognized as ant
        log::info!("Detected project as {}", AntPlugin::PLUGIN_NAME);
        Ok(Plugin::Ant(AntPlugin::new()?))
    } else {
        Err(TmcError::PluginNotFound(path.to_path_buf()))
    }
}
