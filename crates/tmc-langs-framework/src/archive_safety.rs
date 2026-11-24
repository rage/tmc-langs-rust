//! Validation of archive entry names and limits on extraction size, shared by every archive format.

use crate::TmcError;
use std::path::PathBuf;

/// Upper bounds for extracting one archive. Archives from other users or servers are untrusted,
/// so extraction fails instead of exhausting the disk.
#[derive(Debug, Clone, Copy)]
pub struct ExtractionLimits {
    pub max_total_bytes: u64,
    pub max_entry_bytes: u64,
    pub max_entries: u64,
}

impl Default for ExtractionLimits {
    fn default() -> Self {
        Self {
            max_total_bytes: 512 * 1024 * 1024,
            max_entry_bytes: 256 * 1024 * 1024,
            max_entries: 50_000,
        }
    }
}

/// Tracks one extraction against its [`ExtractionLimits`]. Use a fresh budget per archive.
#[derive(Debug)]
pub struct ExtractionBudget {
    limits: ExtractionLimits,
    entries: u64,
    declared_bytes: u64,
    written_bytes: u64,
}

impl ExtractionBudget {
    pub fn new(limits: ExtractionLimits) -> Self {
        Self {
            limits,
            entries: 0,
            declared_bytes: 0,
            written_bytes: 0,
        }
    }

    /// Counts an entry and its header-declared size, which also bounds the decompression work
    /// spent on entries that are skipped. Call once per entry before using it.
    pub fn admit_entry(&mut self, name: &str, declared_size: u64) -> Result<(), TmcError> {
        self.entries += 1;
        if self.entries > self.limits.max_entries {
            return Err(exceeded(name, "entries", self.limits.max_entries));
        }
        if declared_size > self.limits.max_entry_bytes {
            return Err(exceeded(
                name,
                "bytes per entry",
                self.limits.max_entry_bytes,
            ));
        }
        self.declared_bytes = self.declared_bytes.saturating_add(declared_size);
        if self.declared_bytes > self.limits.max_total_bytes {
            return Err(exceeded(
                name,
                "bytes in total",
                self.limits.max_total_bytes,
            ));
        }
        Ok(())
    }

    /// Counts bytes actually written for the entry, as headers cannot be trusted.
    pub fn charge_written(
        &mut self,
        name: &str,
        entry_bytes: u64,
        new_bytes: u64,
    ) -> Result<(), TmcError> {
        if entry_bytes > self.limits.max_entry_bytes {
            return Err(exceeded(
                name,
                "bytes per entry",
                self.limits.max_entry_bytes,
            ));
        }
        self.written_bytes = self.written_bytes.saturating_add(new_bytes);
        if self.written_bytes > self.limits.max_total_bytes {
            return Err(exceeded(
                name,
                "bytes in total",
                self.limits.max_total_bytes,
            ));
        }
        Ok(())
    }
}

fn exceeded(entry: &str, limit: &'static str, max: u64) -> TmcError {
    TmcError::ArchiveExtractionLimitExceeded {
        entry: entry.to_string(),
        limit,
        max,
    }
}

/// Which separator conventions the archive format's writers use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryNameStyle {
    /// tar: `/` only, so a backslash is never a separator and is refused.
    Tar,
    /// zip: Windows tools write `\` separators, which are accepted as `/`.
    Zip,
}

/// Converts an archive entry name into a relative path that is safe to join to an extraction
/// directory on every OS, or errors naming the entry.
pub fn validate_entry_name(raw: &str, style: EntryNameStyle) -> Result<PathBuf, TmcError> {
    let reject = |reason: &'static str| TmcError::UnsafeArchiveEntry {
        entry: raw.to_string(),
        reason,
    };
    let name = match style {
        EntryNameStyle::Tar => {
            if raw.contains('\\') {
                return Err(reject("contains a backslash"));
            }
            std::borrow::Cow::Borrowed(raw)
        }
        EntryNameStyle::Zip => std::borrow::Cow::Owned(raw.replace('\\', "/")),
    };
    if name.starts_with('/') {
        return Err(reject("is an absolute path"));
    }
    let mut path = PathBuf::new();
    for component in name.split('/') {
        match component {
            "" | "." => {}
            ".." => return Err(reject("contains a parent directory component")),
            component => {
                check_component(component).map_err(reject)?;
                path.push(component);
            }
        }
    }
    Ok(path)
}

const RESERVED_DEVICE_NAMES: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];

fn check_component(component: &str) -> Result<(), &'static str> {
    if component.contains('\0') {
        return Err("contains a NUL character");
    }
    // Names valid elsewhere, such as a macOS `Icon\r` or `notes:v2.txt`, must still extract
    // where they are valid, or the whole submission becomes ungradable.
    if !cfg!(windows) {
        return Ok(());
    }
    if component
        .chars()
        .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return Err("contains a character that is invalid in Windows file names");
    }
    if component.ends_with('.') || component.ends_with(' ') {
        return Err("ends with a dot or a space");
    }
    // Windows ignores the extension and trailing spaces when matching device names
    let stem = component
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    let is_numbered_device = |prefix: &str| {
        stem.strip_prefix(prefix)
            .is_some_and(|n| matches!(n.as_bytes(), [b'1'..=b'9']))
    };
    if RESERVED_DEVICE_NAMES.contains(&stem.as_str())
        || is_numbered_device("COM")
        || is_numbered_device("LPT")
    {
        return Err("is a reserved Windows device name");
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn accepts_ordinary_names() {
        for style in [EntryNameStyle::Tar, EntryNameStyle::Zip] {
            for name in [
                "src/Main.java",
                "./src/Main.java",
                "a/",
                "Makefile",
                ".tmcproject.yml",
                "console.c",
                "com10.txt",
                "auxiliary.txt",
                "my file.txt",
                "a..b",
                "",
                ".",
            ] {
                validate_entry_name(name, style).unwrap_or_else(|e| panic!("{name}: {e}"));
            }
        }
    }

    #[test]
    fn rejects_unsafe_names_in_every_style() {
        for style in [EntryNameStyle::Tar, EntryNameStyle::Zip] {
            for name in ["../x", "a/../../x", "/abs", "a\0b"] {
                assert!(
                    validate_entry_name(name, style).is_err(),
                    "{style:?} {name}"
                );
            }
        }
    }

    #[test]
    fn rejects_names_invalid_on_windows_only_there() {
        for style in [EntryNameStyle::Tar, EntryNameStyle::Zip] {
            for name in [
                "C:/x",
                "C:x",
                "CON",
                "con",
                "aux.txt",
                "src/NUL.c",
                "COM1",
                "lpt9.log",
                "CON .txt",
                "x:stream",
                "a/b:stream",
                "name.",
                "name ",
                "a<b",
                "a|b",
                "a?b",
                "a*b",
                "a\"b",
                "a\u{1}b",
            ] {
                assert_eq!(
                    validate_entry_name(name, style).is_err(),
                    cfg!(windows),
                    "{style:?} {name}"
                );
            }
        }
    }

    #[test]
    fn backslashes_are_refused_in_tar_and_separators_in_zip() {
        for name in [
            "C:\\x",
            "a\\..\\..\\x",
            "..\\x",
            "\\\\server\\share\\x",
            "a\\b",
        ] {
            assert!(
                validate_entry_name(name, EntryNameStyle::Tar).is_err(),
                "{name}"
            );
        }
        for name in ["a\\..\\..\\x", "..\\x", "\\\\server\\share\\x"] {
            assert!(
                validate_entry_name(name, EntryNameStyle::Zip).is_err(),
                "{name}"
            );
        }
        assert_eq!(
            validate_entry_name("C:\\x", EntryNameStyle::Zip).is_err(),
            cfg!(windows)
        );
        assert_eq!(
            validate_entry_name("a\\b", EntryNameStyle::Zip).unwrap(),
            PathBuf::from("a").join("b")
        );
    }

    #[test]
    fn error_names_the_entry() {
        let err = validate_entry_name("../x", EntryNameStyle::Tar).unwrap_err();
        assert!(err.to_string().contains("../x"));
    }

    #[test]
    fn budget_enforces_each_limit() {
        let limits = ExtractionLimits {
            max_total_bytes: 10,
            max_entry_bytes: 6,
            max_entries: 2,
        };
        let mut budget = ExtractionBudget::new(limits);
        assert!(budget.admit_entry("a", 7).is_err());
        let mut budget = ExtractionBudget::new(limits);
        budget.admit_entry("a", 6).unwrap();
        assert!(budget.admit_entry("b", 5).is_err());
        let mut budget = ExtractionBudget::new(limits);
        budget.admit_entry("a", 0).unwrap();
        budget.admit_entry("b", 0).unwrap();
        assert!(budget.admit_entry("c", 0).is_err());
        let mut budget = ExtractionBudget::new(limits);
        assert!(budget.charge_written("a", 7, 7).is_err());
        let mut budget = ExtractionBudget::new(limits);
        budget.charge_written("a", 6, 6).unwrap();
        assert!(budget.charge_written("b", 5, 5).is_err());
    }
}
