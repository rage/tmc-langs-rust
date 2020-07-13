//! Module for calling different tasks of TMC-langs language plug-ins.

use super::{
    tar, Error, ExerciseDesc, ExercisePackagingConfiguration, RunResult, ValidationResult,
};
use log::info;
use std::path::{Path, PathBuf};
use tmc_langs_framework::{
    io::{submission_processing, zip},
    plugin::{Language, LanguagePlugin},
    policy::NothingIsStudentFilePolicy,
};
use tmc_langs_java::AntPlugin;
use tmc_langs_java::MavenPlugin;
use tmc_langs_make::MakePlugin;
use tmc_langs_notests::NoTestsPlugin;
use tmc_langs_python3::Python3Plugin;
use tmc_langs_r::RPlugin;

/// See `domain::prepare_solutions`.
pub fn prepare_solutions<'a, I: IntoIterator<Item = &'a PathBuf>>(
    exercise_paths: I,
    dest_root: &Path,
) -> Result<(), Error> {
    submission_processing::prepare_solutions(exercise_paths, dest_root)?;
    Ok(())
}

/// See `LanguagePlugin::prepare_stubs`.
pub fn prepare_stubs<I: IntoIterator<Item = PathBuf>>(
    exercise_paths: I,
    repo_path: &Path,
    dest_path: &Path,
) -> Result<(), Error> {
    for exercise_path in exercise_paths {
        let plugin = get_language_plugin(&exercise_path)?;
        plugin.prepare_stub(&exercise_path, repo_path, dest_path)?;
    }
    Ok(())
}

/// See `LanguagePlugin::check_code_style`.
pub fn run_check_code_style(
    path: &Path,
    locale: Language,
) -> Result<Option<ValidationResult>, Error> {
    Ok(get_language_plugin(path)?.check_code_style(path, locale))
}

/// See `LanguagePlugin::run_tests`.
pub fn run_tests(path: &Path) -> Result<RunResult, Error> {
    get_language_plugin(path)?.run_tests(&path)
}

/// See `LanguagePlugin::scan_exercise`.
pub fn scan_exercise(path: &Path, exercise_name: String) -> Result<ExerciseDesc, Error> {
    Ok(get_language_plugin(path)?.scan_exercise(path, exercise_name)?)
}

/// Tries to find a language plugin for the path, returning `true` if one is found.
pub fn is_exercise_root_directory(path: &Path) -> bool {
    get_language_plugin(path).is_ok()
}

/// Finds the correct language plug-in for the given exercise path and calls `LanguagePlugin::extract_project`,
/// If no language plugin matches, see `extract_project_overwrite`.
pub fn extract_project(compressed_project: &Path, target_location: &Path) -> Result<(), Error> {
    if let Ok(plugin) = get_language_plugin(compressed_project) {
        plugin.extract_project(compressed_project, target_location)?;
    } else {
        extract_project_overwrite(compressed_project, target_location)?;
    }
    Ok(())
}

/// Extract a given archive file containing a compressed project to a target location.
/// This will overwrite any existing files.
// TODO: used?
pub fn extract_project_overwrite(
    compressed_project: &Path,
    target_location: &Path,
) -> Result<(), Error> {
    zip::unzip(
        NothingIsStudentFilePolicy {},
        compressed_project,
        target_location,
    )?;
    Ok(())
}

/// See `LanguagePlugin::compress_project`.
pub fn compress_project(path: &Path) -> Result<Vec<u8>, Error> {
    Ok(get_language_plugin(path)?.compress_project(path)?)
}

/// See `LanguagePlugin::get_exercise_packaging_configuration`.
pub fn get_exercise_packaging_configuration(
    path: &Path,
) -> Result<ExercisePackagingConfiguration, Error> {
    Ok(get_language_plugin(path)?.get_exercise_packaging_configuration(path)?)
}

/// Creates a tarball that can be submitted to TMC-sandbox.
// TODO: used?
pub fn compress_tar_for_submitting(
    project_dir: &Path,
    tmc_langs: &Path,
    tmcrun: &Path,
    target_location: &Path,
) -> Result<(), Error> {
    tar::create_tar_from_project(project_dir, tmc_langs, tmcrun, target_location)?;
    Ok(())
}

/// See `LanguagePlugin::clean`.
pub fn clean(path: &Path) -> Result<(), Error> {
    get_language_plugin(path)?.clean(path)?;
    Ok(())
}

enum Plugin {
    Make(MakePlugin),
    Maven(MavenPlugin),
    NoTests(NoTestsPlugin),
    Python3(Python3Plugin),
    R(RPlugin),
    Ant(AntPlugin),
}

// TODO: write proc macro
impl Plugin {
    fn clean(&self, path: &Path) -> Result<(), Error> {
        match self {
            Self::Make(plugin) => plugin.clean(path),
            Self::Maven(plugin) => plugin.clean(path),
            Self::NoTests(plugin) => plugin.clean(path),
            Self::Python3(plugin) => plugin.clean(path),
            Self::R(plugin) => plugin.clean(path),
            Self::Ant(plugin) => plugin.clean(path),
        }
    }

    fn get_exercise_packaging_configuration(
        &self,
        path: &Path,
    ) -> Result<ExercisePackagingConfiguration, Error> {
        match self {
            Self::Make(plugin) => plugin.get_exercise_packaging_configuration(path),
            Self::Maven(plugin) => plugin.get_exercise_packaging_configuration(path),
            Self::NoTests(plugin) => plugin.get_exercise_packaging_configuration(path),
            Self::Python3(plugin) => plugin.get_exercise_packaging_configuration(path),
            Self::R(plugin) => plugin.get_exercise_packaging_configuration(path),
            Self::Ant(plugin) => plugin.get_exercise_packaging_configuration(path),
        }
    }

    fn compress_project(&self, path: &Path) -> Result<Vec<u8>, Error> {
        match self {
            Self::Make(plugin) => plugin.compress_project(path),
            Self::Maven(plugin) => plugin.compress_project(path),
            Self::NoTests(plugin) => plugin.compress_project(path),
            Self::Python3(plugin) => plugin.compress_project(path),
            Self::R(plugin) => plugin.compress_project(path),
            Self::Ant(plugin) => plugin.compress_project(path),
        }
    }

    fn extract_project(
        &self,
        cmpressed_project: &Path,
        target_location: &Path,
    ) -> Result<(), Error> {
        match self {
            Self::Make(plugin) => plugin.extract_project(cmpressed_project, target_location),
            Self::Maven(plugin) => plugin.extract_project(cmpressed_project, target_location),
            Self::NoTests(plugin) => plugin.extract_project(cmpressed_project, target_location),
            Self::Python3(plugin) => plugin.extract_project(cmpressed_project, target_location),
            Self::R(plugin) => plugin.extract_project(cmpressed_project, target_location),
            Self::Ant(plugin) => plugin.extract_project(cmpressed_project, target_location),
        }
    }

    fn scan_exercise(&self, path: &Path, exercise_name: String) -> Result<ExerciseDesc, Error> {
        match self {
            Self::Make(plugin) => plugin.scan_exercise(path, exercise_name),
            Self::Maven(plugin) => plugin.scan_exercise(path, exercise_name),
            Self::NoTests(plugin) => plugin.scan_exercise(path, exercise_name),
            Self::Python3(plugin) => plugin.scan_exercise(path, exercise_name),
            Self::R(plugin) => plugin.scan_exercise(path, exercise_name),
            Self::Ant(plugin) => plugin.scan_exercise(path, exercise_name),
        }
    }

    fn run_tests(&self, path: &Path) -> Result<RunResult, Error> {
        match self {
            Self::Make(plugin) => plugin.run_tests(path),
            Self::Maven(plugin) => plugin.run_tests(path),
            Self::NoTests(plugin) => plugin.run_tests(path),
            Self::Python3(plugin) => plugin.run_tests(path),
            Self::R(plugin) => plugin.run_tests(path),
            Self::Ant(plugin) => plugin.run_tests(path),
        }
    }

    fn check_code_style(&self, path: &Path, locale: Language) -> Option<ValidationResult> {
        match self {
            Self::Make(plugin) => plugin.check_code_style(path, locale),
            Self::Maven(plugin) => plugin.check_code_style(path, locale),
            Self::NoTests(plugin) => plugin.check_code_style(path, locale),
            Self::Python3(plugin) => plugin.check_code_style(path, locale),
            Self::R(plugin) => plugin.check_code_style(path, locale),
            Self::Ant(plugin) => plugin.check_code_style(path, locale),
        }
    }

    fn prepare_stub(
        &self,
        exercise_path: &Path,
        repo_path: &Path,
        dest_path: &Path,
    ) -> Result<(), Error> {
        match self {
            Self::Make(plugin) => plugin.prepare_stub(exercise_path, repo_path, dest_path),
            Self::Maven(plugin) => plugin.prepare_stub(exercise_path, repo_path, dest_path),
            Self::NoTests(plugin) => plugin.prepare_stub(exercise_path, repo_path, dest_path),
            Self::Python3(plugin) => plugin.prepare_stub(exercise_path, repo_path, dest_path),
            Self::R(plugin) => plugin.prepare_stub(exercise_path, repo_path, dest_path),
            Self::Ant(plugin) => plugin.prepare_stub(exercise_path, repo_path, dest_path),
        }
    }
}

// Get language plugin for the given path.
fn get_language_plugin(path: &Path) -> Result<Plugin, Error> {
    if MakePlugin::is_exercise_type_correct(path) {
        let make = MakePlugin::new();
        info!("Detected project as {}", MakePlugin::PLUGIN_NAME);
        Ok(Plugin::Make(make))
    } else if NoTestsPlugin::is_exercise_type_correct(path) {
        info!("Detected project as {}", NoTestsPlugin::PLUGIN_NAME);
        Ok(Plugin::NoTests(NoTestsPlugin::new()))
    } else if Python3Plugin::is_exercise_type_correct(path) {
        info!("Detected project as {}", Python3Plugin::PLUGIN_NAME);
        Ok(Plugin::Python3(Python3Plugin::new()))
    } else if RPlugin::is_exercise_type_correct(path) {
        info!("Detected project as {}", RPlugin::PLUGIN_NAME);
        Ok(Plugin::R(RPlugin::new()))
    } else if MavenPlugin::is_exercise_type_correct(path) {
        info!("Detected project as {}", MavenPlugin::PLUGIN_NAME);
        Ok(Plugin::Maven(MavenPlugin::new()?))
    } else if AntPlugin::is_exercise_type_correct(path) {
        // TODO: currently, ant needs to be last because any project with src and test are recognized as ant
        info!("Detected project as {}", AntPlugin::PLUGIN_NAME);
        Ok(Plugin::Ant(AntPlugin::new()?))
    } else {
        Err(Error::PluginNotFound(path.to_path_buf()))
    }
}
