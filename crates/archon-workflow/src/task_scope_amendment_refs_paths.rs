//! Module-path resolution for the code-dependency index: the files a
//! `crate::` / `super::` / `self::` / crate-name path walks through, and
//! the one file at or below a module that defines a named item.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use regex::Regex;

use super::{Index, crate_dir};

impl Index {
    /// The files the `crate::`, `super::`, `self::` and crate-name paths of
    /// `text` end at -- a module file, or the one file at or below the
    /// module reached that defines the named item -- resolved from `file`'s
    /// own place.
    pub(super) fn resolved_paths(&self, root: &Path, file: &str, text: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let own = module_dir(file);
        for caps in self.paths.captures_iter(text) {
            let head = &caps[1];
            let mut dir = match head {
                "crate" => crate_dir(root, file).map(|dir| dir.join("src")),
                "self" => Some(own.clone()),
                "super" => own.parent().map(Path::to_path_buf),
                name => self.crates.get(name).cloned(),
            };
            let segments: Vec<&str> = caps[2]
                .split("::")
                .map(str::trim)
                .filter(|segment| !segment.is_empty())
                .collect();
            let last_segment = segments.len().saturating_sub(1);
            for (index_of, segment) in segments.into_iter().enumerate() {
                let Some(current) = dir.clone() else {
                    break;
                };
                if segment == "super" {
                    dir = current.parent().map(Path::to_path_buf);
                    continue;
                }
                let names: Vec<&str> = match segment.strip_prefix('{') {
                    Some(list) => list
                        .trim_end_matches('}')
                        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .filter(|word| !word.is_empty())
                        .collect(),
                    None => vec![segment],
                };
                let mut next = None;
                let last = index_of == last_segment;
                for name in names {
                    let mut module = false;
                    for candidate in [
                        current.join(format!("{name}.rs")),
                        current.join(name).join("mod.rs"),
                    ] {
                        let rel = normalize(&candidate);
                        if root.join(&rel).is_file() {
                            // Only where the path ENDS: the modules it
                            // walks through are namespaces, not uses.
                            if last {
                                out.insert(rel);
                            }
                            next = Some(current.join(name));
                            module = true;
                        }
                    }
                    // An item of the module the path reached: the one file
                    // at or below that module that defines it.
                    if !module {
                        out.extend(self.defined_under(&current, name));
                    }
                }
                dir = next;
            }
        }
        out
    }
}

impl Index {
    /// The file at or below module directory `dir` that defines `name`,
    /// when exactly one does (its own module file counts).
    fn defined_under(&self, dir: &Path, name: &str) -> Option<String> {
        let prefix = format!("{}/", normalize(dir));
        let own = format!("{}.rs", normalize(dir));
        let found: Vec<&String> = self
            .defs
            .get(name)?
            .iter()
            .map(|(file, _)| file)
            .filter(|file| file.starts_with(&prefix) || **file == own)
            .collect();
        match found.as_slice() {
            [only] => Some((*only).clone()),
            _ => None,
        }
    }
}

/// The directory holding `file`'s child modules: beside a crate root
/// (`lib.rs`, `main.rs`, each integration test file directly under
/// `tests/`) or a `mod.rs`; for any other module file, the directory named
/// after it.
pub(super) fn module_dir(file: &str) -> PathBuf {
    let path = Path::new(file);
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let crate_root = parent.file_name().is_some_and(|dir| dir == "tests");
    match path.file_name().and_then(|name| name.to_str()) {
        Some("mod.rs" | "lib.rs" | "main.rs") => parent.to_path_buf(),
        _ if crate_root => parent.to_path_buf(),
        _ => parent.join(path.file_stem().unwrap_or_default()),
    }
}

/// Each indexed crate's name as code spells it (`-` as `_`) -> its `src`.
pub(super) fn crate_names(root: &Path, crates: &BTreeSet<PathBuf>) -> BTreeMap<String, PathBuf> {
    let name = Regex::new(r#"(?m)^\s*name\s*=\s*"([^"]+)""#).expect("name pattern");
    crates
        .iter()
        .filter_map(|dir| {
            let manifest = std::fs::read_to_string(root.join(dir).join("Cargo.toml")).ok()?;
            let package = manifest.split("[package]").nth(1)?;
            let caps = name.captures(package)?;
            Some((caps[1].replace('-', "_"), dir.join("src")))
        })
        .collect()
}

/// `path` with `.` and `..` segments resolved, as a `/`-joined string.
pub(super) fn normalize(path: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::Normal(name) => parts.push(name.to_string_lossy().to_string()),
            _ => {}
        }
    }
    parts.join("/")
}
