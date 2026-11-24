//! Locating the external tools (java, python, make, ...) that plugins run.
//!
//! Tools are resolved to absolute paths before spawning: `CreateProcess` given a bare name
//! searches the current directory before `PATH`, and the CLI inherits an environment fixed when
//! VS Code started, so tools installed since are missing from it on Windows.

use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

/// Resolved location of a tool and the directories its own helpers may live in.
#[derive(Debug, PartialEq, Eq)]
pub struct ResolvedTool {
    pub path: PathBuf,
    /// `Some` when the tool was found outside the inherited `PATH`: the child's `PATH` should
    /// be this value so the tool finds its sibling programs (a MinGW `make` needs `gcc`).
    pub child_path: Option<OsString>,
}

/// Resolves a bare tool name to the file that would run, or `None` if it is not found or is
/// not a bare name.
pub fn resolve_tool(program: &OsStr) -> Option<ResolvedTool> {
    if !is_bare_name(program) {
        return None;
    }
    let tool = normalized_tool_name(&program.to_string_lossy());
    let tool = tool.as_str();
    let names = candidate_names(tool, cfg!(windows));
    let extensions = executable_extensions(tool, cfg!(windows), std::env::var_os("PATHEXT"));

    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let path_dirs: Vec<PathBuf> = std::env::split_paths(&path_var).collect();
    if let Some(path) = find_in_dirs(&names, &path_dirs, &extensions) {
        return Some(ResolvedTool {
            path,
            child_path: None,
        });
    }

    if !cfg!(windows) {
        return None;
    }
    let registry_dirs = windows_registry_path_dirs();
    let known_dirs = known_install_dirs(tool, &|name| std::env::var_os(name));
    let fallback_dirs: Vec<PathBuf> = registry_dirs.iter().cloned().chain(known_dirs).collect();
    let path = find_in_dirs(&names, &fallback_dirs, &extensions)?;
    let found_dir = path.parent().map(Path::to_path_buf);
    let extra_dirs: Vec<PathBuf> = registry_dirs.into_iter().chain(found_dir).collect();
    Some(ResolvedTool {
        path,
        child_path: Some(merge_path_dirs(&path_var, &extra_dirs)),
    })
}

/// `tool` lowercased without an `.exe` suffix, as Windows matches names.
fn normalized_tool_name(tool: &str) -> String {
    let lower = tool.to_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
}

fn is_bare_name(program: &OsStr) -> bool {
    !program.is_empty() && !program.to_string_lossy().contains(['/', '\\'])
}

/// Names a tool may be installed under; the first existing one wins.
fn candidate_names(tool: &str, is_windows: bool) -> Vec<String> {
    let mut names = vec![tool.to_string()];
    if is_windows {
        match tool {
            "make" => names.extend(["mingw32-make".to_string(), "gmake".to_string()]),
            "python3" => names.push("python".to_string()),
            _ => {}
        }
    }
    names
}

/// Suffixes tried after `tool`'s candidate names. On Windows the empty one is tried, last, only
/// when `tool` already carries an extension: an extensionless file there is a shell shim that
/// cannot be started (Rtools puts a `cmd` script on `PATH` ahead of `cmd.exe`).
fn executable_extensions(tool: &str, is_windows: bool, pathext: Option<OsString>) -> Vec<String> {
    if !is_windows {
        return vec![String::new()];
    }
    let pathext = pathext
        .map(|v| v.to_string_lossy().into_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string());
    let has_extension = Path::new(tool).extension().is_some();
    pathext
        .split(';')
        .filter(|e| !e.is_empty())
        .map(|e| e.to_lowercase())
        .chain(has_extension.then(String::new))
        .collect()
}

/// A regular file the shell could run; on Unix one without an execute bit is skipped like it does.
fn is_runnable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn find_in_dirs(names: &[String], dirs: &[PathBuf], extensions: &[String]) -> Option<PathBuf> {
    for dir in dirs {
        if dir.as_os_str().is_empty() || !dir.is_absolute() {
            // a relative PATH entry is the current-directory search by another name
            continue;
        }
        for name in names {
            for extension in extensions {
                let candidate = dir.join(format!("{name}{extension}"));
                if is_runnable_file(&candidate) && !is_store_alias_stub(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// The `python.exe` in `WindowsApps` is a Microsoft Store launcher stub that opens the Store
/// instead of running Python.
fn is_store_alias_stub(path: &Path) -> bool {
    let is_python = path
        .file_stem()
        .is_some_and(|stem| stem.to_string_lossy().to_lowercase().starts_with("python"));
    is_python
        && path
            .components()
            .any(|c| c.as_os_str().eq_ignore_ascii_case("WindowsApps"))
}

/// Per-tool install locations that installers use without adding to `PATH`.
fn known_install_dirs(tool: &str, env: &dyn Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let program_files: Vec<PathBuf> = ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"]
        .iter()
        .filter_map(|name| env(name))
        .map(PathBuf::from)
        .collect();
    let mut dirs = vec![];
    match tool {
        "rscript" => {
            if let Some(r_home) = env("R_HOME") {
                dirs.push(PathBuf::from(r_home).join("bin"));
            }
            for pf in &program_files {
                dirs.extend(
                    newest_first_subdirs(&pf.join("R"))
                        .into_iter()
                        .map(|d| d.join("bin")),
                );
            }
        }
        "make" => {
            for root in [
                "C:\\msys64",
                "C:\\MinGW",
                "C:\\Strawberry\\c",
                "C:\\ProgramData\\chocolatey",
            ] {
                dirs.push(PathBuf::from(root).join("usr").join("bin"));
                dirs.push(PathBuf::from(root).join("mingw64").join("bin"));
                dirs.push(PathBuf::from(root).join("bin"));
            }
        }
        "dotnet" => dirs.extend(program_files.iter().map(|pf| pf.join("dotnet"))),
        "python3" | "python" | "py" => {
            if let Some(local) = env("LocalAppData") {
                let programs = PathBuf::from(local).join("Programs").join("Python");
                for dir in newest_first_subdirs(&programs) {
                    dirs.push(dir.join("Scripts"));
                    dirs.push(dir);
                }
            }
            for pf in &program_files {
                dirs.extend(newest_first_subdirs(&pf.join("Python")));
            }
            if let Some(profile) = env("USERPROFILE") {
                for conda in ["anaconda3", "miniconda3"] {
                    dirs.push(PathBuf::from(&profile).join(conda));
                }
            }
            for conda in ["C:\\ProgramData\\anaconda3", "C:\\ProgramData\\miniconda3"] {
                dirs.push(PathBuf::from(conda));
            }
        }
        _ => {}
    }
    dirs
}

/// Subdirectories of `dir` in descending name order (so `R-4.4.1` before `R-4.3.0`).
fn newest_first_subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    dirs
}

/// `current` with each of `extra` appended unless already present (compared ignoring case,
/// as Windows does).
fn merge_path_dirs(current: &OsStr, extra: &[PathBuf]) -> OsString {
    let mut dirs: Vec<PathBuf> = std::env::split_paths(current).collect();
    for dir in extra {
        let is_present = dirs
            .iter()
            .any(|existing| existing.as_os_str().eq_ignore_ascii_case(dir.as_os_str()));
        if !is_present {
            dirs.push(dir.clone());
        }
    }
    std::env::join_paths(dirs).unwrap_or_else(|_| current.to_os_string())
}

/// `PATH` directories as an installer last saved them (machine, then user), which a process
/// started before the installation does not see.
fn windows_registry_path_dirs() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        windows_registry::path_dirs()
    }
    #[cfg(not(windows))]
    {
        vec![]
    }
}

/// `path` made absolute, for use as a toolchain's working directory or class-path entry.
///
/// Not canonicalized: resolving a mapped drive (`H:\x`) yields its UNC target, which `cmd.exe`
/// cannot use as a working directory. A `\\?\UNC\` form is rewritten to `\\server\share`, which
/// the JVM, Python and CreateProcess accept.
pub fn absolute_tool_path(path: &Path) -> std::io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    Ok(simplify_verbatim_path(&absolute))
}

fn simplify_verbatim_path(path: &Path) -> PathBuf {
    const VERBATIM_UNC: &str = "\\\\?\\UNC\\";
    const VERBATIM_DISK: &str = "\\\\?\\";
    let Some(text) = path.to_str() else {
        return path.to_path_buf();
    };
    if let Some(rest) = text.strip_prefix(VERBATIM_UNC) {
        return PathBuf::from(format!("\\\\{rest}"));
    }
    match text.strip_prefix(VERBATIM_DISK) {
        Some(rest) if rest.as_bytes().get(1..3) == Some(b":\\") => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

/// What to tell a user whose `tool` was not found.
pub fn missing_tool_hint(tool: &str) -> String {
    let install = match normalized_tool_name(tool).as_str() {
        "java" | "javac" | "mvn" | "mvn.cmd" | "ant" | "ant.bat" => {
            "Install a JDK (Java 11 or newer) and make sure JAVA_HOME points to it or java is on PATH."
        }
        "python3" | "python" | "py" => {
            "Install Python 3 or set TMC_LANGS_PYTHON_EXEC to the full path of a Python 3 interpreter."
        }
        "make" | "mingw32-make" | "gmake" => {
            "Install a C toolchain with make (on Windows, MSYS2 or MinGW) and add its bin folder to PATH."
        }
        "rscript" => {
            "Install R and add its bin folder (for example C:\\Program Files\\R\\R-x.y.z\\bin) to PATH."
        }
        "dotnet" => "Install the .NET SDK.",
        "git" => "Install Git.",
        _ => "Install it and make sure it is on PATH.",
    };
    format!(
        "{install} If you installed it after starting VS Code, close all VS Code windows and start it again."
    )
}

#[cfg(windows)]
mod windows_registry {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};
    use windows_sys::Win32::System::{
        Environment::ExpandEnvironmentStringsW,
        Registry::{
            HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_EXPAND_SZ, REG_SZ,
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
        },
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn read_path_value(root: HKEY, subkey: &str) -> Option<OsString> {
        let subkey = wide(subkey);
        let value_name = wide("Path");
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: pointers refer to live, NUL-terminated buffers for the call duration.
        unsafe {
            if RegOpenKeyExW(root, subkey.as_ptr(), 0, KEY_READ, &mut key) != 0 {
                return None;
            }
            let mut kind = 0u32;
            let mut size = 0u32;
            let mut result = None;
            if RegQueryValueExW(
                key,
                value_name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                std::ptr::null_mut(),
                &mut size,
            ) == 0
                && (kind == REG_SZ || kind == REG_EXPAND_SZ)
                && size > 0
            {
                let mut buffer = vec![0u16; (size as usize).div_ceil(2) + 1];
                let mut size_in_out = size;
                if RegQueryValueExW(
                    key,
                    value_name.as_ptr(),
                    std::ptr::null(),
                    &mut kind,
                    buffer.as_mut_ptr().cast(),
                    &mut size_in_out,
                ) == 0
                {
                    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
                    buffer.truncate(len);
                    buffer.push(0);
                    result = Some(if kind == REG_EXPAND_SZ {
                        expand(&buffer)
                    } else {
                        OsString::from_wide(&buffer[..buffer.len() - 1])
                    });
                }
            }
            RegCloseKey(key);
            result
        }
    }

    /// `source` is NUL-terminated.
    fn expand(source: &[u16]) -> OsString {
        // SAFETY: `source` is NUL-terminated; the buffer length passed is its real length.
        unsafe {
            let needed = ExpandEnvironmentStringsW(source.as_ptr(), std::ptr::null_mut(), 0);
            if needed == 0 {
                return OsString::from_wide(&source[..source.len() - 1]);
            }
            let mut buffer = vec![0u16; needed as usize];
            let written = ExpandEnvironmentStringsW(source.as_ptr(), buffer.as_mut_ptr(), needed);
            if written == 0 || written > needed {
                return OsString::from_wide(&source[..source.len() - 1]);
            }
            OsString::from_wide(&buffer[..(written as usize).saturating_sub(1)])
        }
    }

    pub fn path_dirs() -> Vec<PathBuf> {
        let machine = read_path_value(
            HKEY_LOCAL_MACHINE,
            "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
        );
        let user = read_path_value(HKEY_CURRENT_USER, "Environment");
        machine
            .into_iter()
            .chain(user)
            .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
            .collect()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::fs;

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn skips_a_file_without_the_execute_bit() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let (first, second) = (temp.path().join("a"), temp.path().join("b"));
        touch(&first.join("tool"));
        touch(&second.join("tool"));
        fs::set_permissions(first.join("tool"), fs::Permissions::from_mode(0o644)).unwrap();

        let found = find_in_dirs(
            &["tool".to_string()],
            &[first, second.clone()],
            &[String::new()],
        );

        assert_eq!(found, Some(second.join("tool")));
    }

    #[test]
    fn finds_tool_in_given_dirs_in_order() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        touch(&second.join("tool"));
        touch(&first.join("tool"));
        let found = find_in_dirs(
            &["tool".to_string()],
            &[first.clone(), second],
            &[String::new()],
        );
        assert_eq!(found, Some(first.join("tool")));
    }

    #[test]
    fn tries_extensions_and_alias_names() {
        let temp = tempfile::tempdir().unwrap();
        touch(&temp.path().join("mingw32-make.exe"));
        let names = candidate_names("make", true);
        let extensions = executable_extensions("make", true, Some(OsString::from(".EXE;.CMD")));
        assert_eq!(
            find_in_dirs(&names, &[temp.path().to_path_buf()], &extensions),
            Some(temp.path().join("mingw32-make.exe"))
        );
        assert_eq!(candidate_names("make", false), vec!["make"]);
    }

    #[test]
    fn skips_an_extensionless_shim_earlier_on_path_on_windows() {
        let temp = tempfile::tempdir().unwrap();
        let shim_dir = temp.path().join("rtools").join("usr").join("bin");
        let system_dir = temp.path().join("System32");
        touch(&shim_dir.join("cmd"));
        touch(&system_dir.join("cmd.exe"));
        let extensions = executable_extensions("cmd", true, Some(OsString::from(".EXE")));
        assert_eq!(
            find_in_dirs(
                &["cmd".to_string()],
                &[shim_dir, system_dir.clone()],
                &extensions
            ),
            Some(system_dir.join("cmd.exe"))
        );
    }

    #[test]
    fn tries_a_name_as_given_on_windows_only_when_it_has_an_extension() {
        let pathext = || Some(OsString::from(".EXE"));
        assert_eq!(executable_extensions("cmd", true, pathext()), vec![".exe"]);
        assert_eq!(
            executable_extensions("mvn.cmd", true, pathext()),
            vec![".exe", ""]
        );
        assert_eq!(executable_extensions("cmd", false, pathext()), vec![""]);
    }

    #[test]
    fn ignores_relative_path_entries() {
        let temp = tempfile::tempdir().unwrap();
        touch(&temp.path().join("tool"));
        assert_eq!(
            find_in_dirs(
                &["tool".to_string()],
                &[PathBuf::from("")],
                &[String::new()]
            ),
            None
        );
        assert_eq!(
            find_in_dirs(
                &["tool".to_string()],
                &[PathBuf::from(".")],
                &[String::new()]
            ),
            None
        );
    }

    #[test]
    fn skips_store_python_stub() {
        let temp = tempfile::tempdir().unwrap();
        let stub_dir = temp.path().join("Microsoft").join("WindowsApps");
        touch(&stub_dir.join("python.exe"));
        let real_dir = temp.path().join("Python312");
        touch(&real_dir.join("python.exe"));
        assert_eq!(
            find_in_dirs(
                &["python".to_string()],
                &[stub_dir, real_dir.clone()],
                &[".exe".to_string()],
            ),
            Some(real_dir.join("python.exe"))
        );
    }

    #[test]
    fn verbatim_paths_are_simplified() {
        assert_eq!(
            simplify_verbatim_path(Path::new(r"\\?\UNC\server\share\ex\src")),
            PathBuf::from(r"\\server\share\ex\src")
        );
        assert_eq!(
            simplify_verbatim_path(Path::new(r"\\?\C:\ex")),
            PathBuf::from(r"C:\ex")
        );
        assert_eq!(
            simplify_verbatim_path(Path::new(r"H:\ex")),
            PathBuf::from(r"H:\ex")
        );
        assert_eq!(
            simplify_verbatim_path(Path::new("/home/ex")),
            PathBuf::from("/home/ex")
        );
    }

    #[test]
    fn absolute_tool_path_keeps_symlinks_and_makes_relative_absolute() {
        let path = absolute_tool_path(Path::new("some/relative")).unwrap();
        assert!(path.is_absolute());
        assert!(path.ends_with("some/relative"));
    }

    #[test]
    fn bare_names_only() {
        assert!(is_bare_name(OsStr::new("make")));
        assert!(!is_bare_name(OsStr::new("/usr/bin/make")));
        assert!(!is_bare_name(OsStr::new("C:\\tools\\make.exe")));
        assert!(!is_bare_name(OsStr::new("")));
    }

    #[test]
    fn merge_appends_only_missing_dirs() {
        let current = std::env::join_paths(["/a", "/b"]).unwrap();
        let merged = merge_path_dirs(&current, &[PathBuf::from("/B"), PathBuf::from("/c")]);
        let dirs: Vec<_> = std::env::split_paths(&merged).collect();
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/c")
            ]
        );
    }

    #[test]
    fn r_install_dirs_are_newest_first() {
        let temp = tempfile::tempdir().unwrap();
        let program_files = temp.path().to_path_buf();
        fs::create_dir_all(program_files.join("R").join("R-4.3.0")).unwrap();
        fs::create_dir_all(program_files.join("R").join("R-4.4.1")).unwrap();
        let env =
            |name: &str| (name == "ProgramFiles").then(|| program_files.clone().into_os_string());
        let dirs = known_install_dirs("rscript", &env);
        assert_eq!(
            dirs,
            vec![
                program_files.join("R").join("R-4.4.1").join("bin"),
                program_files.join("R").join("R-4.3.0").join("bin"),
            ]
        );
    }

    #[test]
    fn missing_tool_hint_names_remedy_and_restart() {
        let hint = missing_tool_hint("make");
        assert!(hint.contains("MSYS2"));
        assert!(hint.contains("close all VS Code windows"));
        assert!(missing_tool_hint("Rscript").contains("Install R"));
        assert!(missing_tool_hint("py").contains("TMC_LANGS_PYTHON_EXEC"));
        assert!(missing_tool_hint("unknown-tool").contains("PATH"));
    }
}
