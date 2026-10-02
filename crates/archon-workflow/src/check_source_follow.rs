//! What a pinned source pulls in, pinned with it (PLAN-11,
//! [`crate::check_source_resolve`]).
//!
//! A test is only as fixed as everything it reads. For every resolved
//! source the resolver follows, recursively:
//!
//! - Rust: `include!` / `include_str!` / `include_bytes!` of a literal path;
//! - Python: `import a.b` / `from a.b import c` / `from . import x` that
//!   resolve to a file under the script's directory or the cwd;
//! - JavaScript / TypeScript: relative `require('./x')` / `from './x'`;
//! - shell: `source x` / `. x`;
//! - every source's FIXTURES: string literals naming an existing data file
//!   (or a fixture directory) beside it, its package or the cwd.
//!
//! An include or a source whose path is built at run time cannot be
//! followed; it is recorded in `unresolved`. An import that names no file
//! in either tree is a library the toolchain provides, not a source.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::check_source_resolve::{Found, Resolution, Roots, files_under};

pub(crate) const ROLE_INCLUDED: &str = "included file";
pub(crate) const ROLE_IMPORTED: &str = "imported module";
pub(crate) const ROLE_FIXTURE: &str = "fixture";

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a valid pattern")
}

/// Follow every source in `out`, adding what each pulls in.
pub(crate) fn follow(roots: &Roots, cwd: &Path, out: &mut Resolution) {
    let mut queue: Vec<Found> = out.found.clone();
    let mut seen: BTreeSet<(PathBuf, Option<String>)> = BTreeSet::new();
    while let Some(found) = queue.pop() {
        let abs = roots.of(found.root).join(&found.path);
        if !seen.insert((abs.clone(), found.item.clone())) {
            continue;
        }
        // A module declaration, a cfg line or a manifest entry pulls in
        // nothing of its own; a fixture is data.
        if found.role == ROLE_FIXTURE
            || found
                .item
                .as_deref()
                .is_some_and(|key| !key.starts_with("fn:"))
        {
            continue;
        }
        let Some(text) = text_of(&abs, found.item.as_deref()) else {
            continue;
        };
        let dir = abs.parent().unwrap_or(Path::new("/")).to_path_buf();
        let before = out.found.len();
        match extension(&abs).as_str() {
            "rs" => rust(roots, &abs, &dir, &text, out),
            "py" => python(roots, cwd, &dir, &text, out),
            "js" | "mjs" | "cjs" | "ts" | "mts" | "tsx" | "jsx" => {
                javascript(roots, &dir, &text, out)
            }
            _ if is_shell(&abs, &text) => shell(roots, cwd, &dir, &text, out),
            _ => {}
        }
        fixtures(roots, cwd, &abs, &text, out);
        queue.extend(out.found[before..].iter().cloned());
    }
}

fn text_of(abs: &Path, item: Option<&str>) -> Option<String> {
    let text = std::fs::read_to_string(abs).ok()?;
    match item {
        Some(key) => crate::check_source_rust::item_text(&text, key),
        None => Some(text),
    }
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn is_shell(path: &Path, text: &str) -> bool {
    matches!(extension(path).as_str(), "sh" | "bash" | "zsh")
        || text
            .lines()
            .next()
            .is_some_and(|line| line.starts_with("#!") && line.contains("sh"))
}

fn add(roots: &Roots, abs: &Path, role: &str, out: &mut Resolution) {
    if out
        .found
        .iter()
        .any(|f| f.item.is_none() && roots.of(f.root).join(&f.path) == abs)
    {
        return;
    }
    out.file(roots, abs, role);
}

fn rust(roots: &Roots, file: &Path, dir: &Path, text: &str, out: &mut Resolution) {
    let any = regex(r"include(?:_str|_bytes)?!\s*\(");
    let literal = regex(r#"include(?:_str|_bytes)?!\s*\(\s*"([^"\\]+)"\s*\)"#);
    let literals: Vec<String> = literal
        .captures_iter(text)
        .map(|c| c[1].to_string())
        .collect();
    for path in &literals {
        add(roots, &dir.join(path), ROLE_INCLUDED, out);
    }
    if any.find_iter(text).count() > literals.len() {
        out.unresolved.push(format!(
            "{} includes a file by a path built at compile time; it is not pinned",
            file.display()
        ));
    }
}

fn python(roots: &Roots, cwd: &Path, dir: &Path, text: &str, out: &mut Resolution) {
    let import = regex(r"(?m)^\s*(?:from\s+(\.*[\w.]*)\s+import\s+([\w, ]+)|import\s+([\w.]+))");
    for capture in import.captures_iter(text) {
        let (module, names) = match (capture.get(1), capture.get(3)) {
            (Some(from), _) => (
                from.as_str().to_string(),
                capture.get(2).map(|m| m.as_str()),
            ),
            (None, Some(module)) => (module.as_str().to_string(), None),
            _ => continue,
        };
        let dots = module.chars().take_while(|c| *c == '.').count();
        let rest = module[dots..].replace('.', "/");
        let bases: Vec<PathBuf> = if dots > 0 {
            let mut base = dir.to_path_buf();
            for _ in 1..dots {
                base.pop();
            }
            vec![base]
        } else {
            vec![dir.to_path_buf(), cwd.to_path_buf()]
        };
        let mut candidates = Vec::new();
        for base in &bases {
            let at = if rest.is_empty() {
                base.clone()
            } else {
                base.join(&rest)
            };
            candidates.push(at.with_extension("py"));
            candidates.push(at.join("__init__.py"));
            for name in names.into_iter().flat_map(|n| n.split(',')) {
                candidates.push(at.join(format!("{}.py", name.trim())));
            }
        }
        for candidate in candidates.into_iter().filter(|c| c.is_file()) {
            add(roots, &candidate, ROLE_IMPORTED, out);
        }
    }
}

fn javascript(roots: &Roots, dir: &Path, text: &str, out: &mut Resolution) {
    let relative = regex(r#"(?:require\s*\(\s*|from\s+|import\s+)['"](\.{1,2}/[^'"]+)['"]"#);
    for capture in relative.captures_iter(text) {
        let base = dir.join(&capture[1]);
        let candidates = [
            base.clone(),
            base.with_extension("js"),
            base.with_extension("ts"),
            base.with_extension("mjs"),
            base.join("index.js"),
            base.join("index.ts"),
        ];
        match candidates.iter().find(|c| c.is_file()) {
            Some(found) => add(roots, found, ROLE_IMPORTED, out),
            None => add(roots, &base, ROLE_IMPORTED, out),
        }
    }
}

fn shell(roots: &Roots, cwd: &Path, dir: &Path, text: &str, out: &mut Resolution) {
    let source = regex(r"(?m)^\s*(?:source|\.)\s+(\S+)");
    for capture in source.captures_iter(text) {
        let path = capture[1].trim_matches(['"', '\'']);
        if path.contains('$') || path.contains('`') {
            out.unresolved.push(format!(
                "a script sources `{path}`, a path built at run time; it is not pinned"
            ));
            continue;
        }
        let at = [dir.join(path), cwd.join(path)]
            .into_iter()
            .find(|c| c.is_file())
            .unwrap_or_else(|| dir.join(path));
        add(roots, &at, "sourced script", out);
    }
}

/// String literals naming an existing data file beside the source, in its
/// package or the cwd, or a fixture directory: what the test reads. An
/// implementation file or a manifest is never a fixture.
fn fixtures(roots: &Roots, cwd: &Path, file: &Path, text: &str, out: &mut Resolution) {
    let literal = regex(r#""([^"\\\n]{2,200})""#);
    let dir = file.parent().unwrap_or(Path::new("/"));
    let package = file
        .ancestors()
        .skip(1)
        .find(|a| a.join("Cargo.toml").is_file() || a.join("package.json").is_file())
        .map(Path::to_path_buf);
    for capture in literal.captures_iter(text) {
        let value = &capture[1];
        if !(value.contains('/') || value.contains('.')) || value.contains(char::is_whitespace) {
            continue;
        }
        if value.starts_with('/') || value.contains("://") || value.contains("..") {
            continue;
        }
        let bases = [
            Some(dir.to_path_buf()),
            package.clone(),
            Some(cwd.to_path_buf()),
        ];
        let Some(hit) = bases
            .into_iter()
            .flatten()
            .map(|base| base.join(value))
            .find(|candidate| candidate.exists())
        else {
            continue;
        };
        if hit.is_file() && data_file(&hit) {
            add(roots, &hit, ROLE_FIXTURE, out);
        } else if hit.is_dir() && fixture_dir(&hit, file) {
            for inner in files_under(&hit) {
                add(roots, &inner, ROLE_FIXTURE, out);
            }
        }
    }
}

fn data_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let parts: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    !matches!(name, "Cargo.toml" | "Cargo.lock" | "package.json")
        && !parts
            .iter()
            .any(|p| p == "src" || p == "target" || p == ".git")
        && extension(path) != "rs"
}

/// A directory a source names is the data it reads -- unless it holds the
/// source itself (naming its own tree is not naming data) or is a package
/// of its own. Judged on the tree, never on the directory's name.
fn fixture_dir(dir: &Path, source: &Path) -> bool {
    let dir = dir
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap_or_else(|_| dir.to_path_buf());
    let source = source
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap_or_else(|_| source.to_path_buf());
    !source.starts_with(&dir)
        && !dir.join("Cargo.toml").is_file()
        && !dir.join("package.json").is_file()
}
