//! Module for calling different tasks of TMC-langs language plug-ins.

mod course_refresher;
mod submission_packaging;
mod submission_processing;
mod tar_helper;
mod tmc_zip;

pub use self::course_refresher::{
    Course, CourseRefresher, GroupBits, ModeBits, Options, RefreshData, RefreshExercise,
    SourceBackend,
};
pub use self::submission_packaging::{OutputFormat, TmcParams};

use crate::error::UtilError;
use crate::{ExerciseDesc, ExercisePackagingConfiguration, RunResult, StyleValidationResult};
use std::path::{Path, PathBuf};
use tmc_langs_csharp::CSharpPlugin;
use tmc_langs_framework::{
    anyhow,
    plugin::{Language, LanguagePlugin},
    policy::NothingIsStudentFilePolicy,
    StudentFilePolicy, TmcError, TmcProjectYml,
};
use tmc_langs_java::AntPlugin;
use tmc_langs_java::MavenPlugin;
use tmc_langs_make::MakePlugin;
use tmc_langs_notests::NoTestsPlugin;
use tmc_langs_python3::Python3Plugin;
use tmc_langs_r::RPlugin;
use walkdir::WalkDir;

/// See `submission_processing::prepare_solution`.
pub fn prepare_solution(exercise_path: &Path, dest_root: &Path) -> Result<(), UtilError> {
    submission_processing::prepare_solution(exercise_path, dest_root)?;
    Ok(())
}

/// See `submission_processing::prepare_stub`.
pub fn prepare_stub(exercise_path: &Path, dest_path: &Path) -> Result<(), UtilError> {
    submission_processing::prepare_stub(&exercise_path, dest_path)?;

    // The Ant plugin needs some additional files to be copied over.
    if AntPlugin::is_exercise_type_correct(&exercise_path) {
        AntPlugin::copy_tmc_junit_runner(dest_path).map_err(|e| TmcError::Plugin(Box::new(e)))?;
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
) -> Result<Option<StyleValidationResult>, UtilError> {
    Ok(get_language_plugin(path)?.check_code_style(path, locale)?)
}

/// See `LanguagePlugin::run_tests`.
pub fn run_tests(path: &Path, warnings: &mut Vec<anyhow::Error>) -> Result<RunResult, UtilError> {
    Ok(get_language_plugin(path)?.run_tests(path, warnings)?)
}

/// See `LanguagePlugin::scan_exercise`.
pub fn scan_exercise(
    path: &Path,
    exercise_name: String,
    warnings: &mut Vec<anyhow::Error>,
) -> Result<ExerciseDesc, UtilError> {
    Ok(get_language_plugin(path)?.scan_exercise(path, exercise_name, warnings)?)
}

/// Tries to find a language plugin for the path, returning `true` if one is found.
pub fn is_exercise_root_directory(path: &Path) -> bool {
    get_language_plugin(path).is_ok()
}

/// Finds the correct language plug-in for the given exercise path and calls `LanguagePlugin::extract_project`,
/// If no language plugin matches, see `extract_project_overwrite`.
pub fn extract_project(
    compressed_project: impl std::io::Read + std::io::Seek,
    target_location: &Path,
    clean: bool,
) -> Result<(), UtilError> {
    if let Ok(plugin) = get_language_plugin(target_location) {
        plugin.extract_project(compressed_project, target_location, clean)?;
    } else {
        log::debug!(
            "no matching language plugin found for {}, overwriting",
            target_location.display()
        );
        extract_project_overwrite(compressed_project, target_location)?;
    }
    Ok(())
}

/// Extract a given archive file containing a compressed project to a target location.
/// This will overwrite any existing files.
pub fn extract_project_overwrite(
    compressed_project: impl std::io::Read + std::io::Seek,
    target_location: &Path,
) -> Result<(), UtilError> {
    tmc_zip::unzip(
        NothingIsStudentFilePolicy::new(target_location)?,
        compressed_project,
        target_location,
    )?;
    Ok(())
}

/// Extracts a project archive, only taking out files classified as student files.
pub fn extract_student_files(
    compressed_project: impl std::io::Read + std::io::Seek,
    target_location: &Path,
) -> Result<(), UtilError> {
    if let Ok(plugin) = get_language_plugin(target_location) {
        plugin.extract_student_files(compressed_project, target_location)?;
    } else {
        log::debug!(
            "no matching language plugin found for {}, overwriting",
            target_location.display()
        );
        extract_project_overwrite(compressed_project, target_location)?;
    }
    Ok(())
}

/// See `LanguagePlugin::compress_project`.
// TODO: clean up
pub fn compress_project(path: &Path) -> Result<Vec<u8>, UtilError> {
    let plugin = get_language_plugin(path)?;
    match plugin {
        Plugin::CSharp(_) => Ok(tmc_zip::zip(
            <CSharpPlugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
        )?),
        Plugin::Make(_) => Ok(tmc_zip::zip(
            <MakePlugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
        )?),
        Plugin::Maven(_) => Ok(tmc_zip::zip(
            <MavenPlugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
        )?),
        Plugin::NoTests(_) => Ok(tmc_zip::zip(
            <NoTestsPlugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
        )?),
        Plugin::Python3(_) => Ok(tmc_zip::zip(
            <Python3Plugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
        )?),
        Plugin::R(_) => Ok(tmc_zip::zip(
            <RPlugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
        )?),
        Plugin::Ant(_) => Ok(tmc_zip::zip(
            <AntPlugin as LanguagePlugin>::StudentFilePolicy::new(path)?,
            path,
        )?),
    }
}

/// See `LanguagePlugin::get_exercise_packaging_configuration`.
pub fn get_exercise_packaging_configuration(
    path: &Path,
) -> Result<ExercisePackagingConfiguration, UtilError> {
    let config = TmcProjectYml::from(path)?;
    Ok(get_language_plugin(path)?.get_exercise_packaging_configuration(config)?)
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
pub fn find_exercise_directories(exercise_path: &Path) -> Result<Vec<PathBuf>, UtilError> {
    let mut paths = vec![];
    for entry in WalkDir::new(exercise_path).into_iter().filter_entry(|e| {
        !submission_processing::is_hidden_dir(e)
            || e.file_name() == "private"
            || submission_processing::contains_tmcignore(e)
    }) {
        let entry = entry?;
        // TODO: Java implementation doesn't scan root directories
        if is_exercise_root_directory(entry.path()) {
            paths.push(entry.into_path())
        }
    }
    Ok(paths)
}

/// Parses available exercise points from the exercise without compiling it.
pub fn get_available_points(exercise_path: &Path) -> Result<Vec<String>, UtilError> {
    let points = get_language_plugin(exercise_path)?.get_available_points(exercise_path)?;
    Ok(points)
}

pub fn refresh_course(
    course: Course,
    options: Options,
    chmod_bits: Option<ModeBits>,
    chgrp_uid: Option<GroupBits>,
    cache_root: PathBuf,
    rails_root: PathBuf,
) -> Result<RefreshData, UtilError> {
    course_refresher::refresh_course(
        course, options, chmod_bits, chgrp_uid, cache_root, rails_root,
    )
}

// enum containing all the plugins
#[impl_enum::with_methods(
    fn clean(&self, path: &Path) -> Result<(), TmcError> {}
    fn get_exercise_packaging_configuration(config: TmcProjectYml) -> Result<ExercisePackagingConfiguration, TmcError> {}
    fn extract_project(compressed_project: impl std::io::Read + std::io::Seek, target_location: &Path, clean: bool) -> Result<(), TmcError> {}
    fn extract_student_files(compressed_project: impl std::io::Read + std::io::Seek, target_location: &Path) -> Result<(), TmcError> {}
    fn scan_exercise(&self, path: &Path, exercise_name: String, warnings: &mut Vec<anyhow::Error>) -> Result<ExerciseDesc, TmcError> {}
    fn run_tests(&self, path: &Path, warnings: &mut Vec<anyhow::Error>) -> Result<RunResult, TmcError> {}
    fn check_code_style(&self, path: &Path, locale: Language) -> Result<Option<StyleValidationResult>, TmcError> {}
    fn get_available_points(exercise_path: &Path) -> Result<Vec<String>, TmcError> {}
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
    if NoTestsPlugin::is_exercise_type_correct(path) {
        log::info!("Detected project as {}", NoTestsPlugin::PLUGIN_NAME);
        Ok(Plugin::NoTests(NoTestsPlugin::new()))
    } else if CSharpPlugin::is_exercise_type_correct(path) {
        let csharp = CSharpPlugin::new();
        log::info!("Detected project as {}", CSharpPlugin::PLUGIN_NAME);
        Ok(Plugin::CSharp(csharp))
    } else if MakePlugin::is_exercise_type_correct(path) {
        let make = MakePlugin::new();
        log::info!("Detected project as {}", MakePlugin::PLUGIN_NAME);
        Ok(Plugin::Make(make))
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
