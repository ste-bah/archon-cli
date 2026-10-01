//! `cargo test` / `cargo nextest run` as a frozen check runs them: which
//! packages, which targets, which test functions (PLAN-11,
//! [`crate::check_source_resolve`]).

use std::path::{Path, PathBuf};

use crate::check_source_chain::{rust_file_tree, src_items, test_module};
use crate::check_source_manifest::test_entry_key;
use crate::check_source_resolve::{Found, Resolution, Roots, Watch, files_under};
use crate::check_source_rust::matching_tests;

/// Options that consume the next word when not written `--opt=value`.
const WITH_VALUE: &[&str] = &[
    "-p",
    "--package",
    "--exclude",
    "--test",
    "--bin",
    "--example",
    "--bench",
    "-F",
    "--features",
    "--target",
    "--target-dir",
    "--manifest-path",
    "-j",
    "--jobs",
    "--profile",
    "--cargo-profile",
    "-P",
    "--nextest-profile",
    "--config",
    "--color",
    "-E",
    "--filterset",
    "--partition",
    "--skip",
    "--test-threads",
    "--format",
    "--logfile",
];

#[derive(Default)]
struct Selection {
    packages: Vec<String>,
    manifest: Option<String>,
    tests: Vec<String>,
    filters: Vec<String>,
    lib_only: bool,
    exact: bool,
    filterset: bool,
}

fn select(args: &[String]) -> Option<Selection> {
    let rest = match args.first().map(String::as_str) {
        Some("test") => &args[1..],
        Some("nextest") if args.get(1).is_some_and(|a| a == "run") => &args[2..],
        _ => return None,
    };
    let mut selection = Selection::default();
    let mut words = rest.iter();
    while let Some(word) = words.next() {
        let (flag, inline) = match word.split_once('=') {
            Some((flag, value)) if flag.starts_with('-') => (flag, Some(value.to_string())),
            _ => (word.as_str(), None),
        };
        if WITH_VALUE.contains(&flag) {
            let value = inline.or_else(|| words.next().cloned()).unwrap_or_default();
            match flag {
                "-p" | "--package" => selection.packages.push(value),
                "--manifest-path" => selection.manifest = Some(value),
                "--test" => selection.tests.push(value),
                "-E" | "--filterset" => selection.filterset = true,
                _ => {}
            }
            continue;
        }
        match flag {
            "--lib" => selection.lib_only = true,
            "--exact" => selection.exact = true,
            "--" => {}
            _ if flag.starts_with('-') => {}
            _ => selection.filters.push(word.clone()),
        }
    }
    Some(selection)
}

/// Resolve one `cargo ...` simple command run in `cwd`.
pub(crate) fn resolve_cargo(args: &[String], cwd: &Path, roots: &Roots, out: &mut Resolution) {
    let args: Vec<String> = args
        .iter()
        .filter(|arg| !arg.starts_with('+'))
        .cloned()
        .collect();
    let Some(selection) = select(&args) else {
        return;
    };
    if selection.filterset {
        out.unresolved.push(
            "a nextest filterset (-E) selects tests this resolver does not evaluate; the selected packages' whole suites are pinned instead".into(),
        );
    }
    let workspace = selection
        .manifest
        .as_ref()
        .map(|manifest| cwd.join(manifest))
        .and_then(|manifest| manifest.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| cwd.to_path_buf());
    let packages = package_dirs(&workspace, roots, &selection.packages, out);
    for dir in packages {
        build_settings(&dir, cwd, roots, out);
        resolve_package(&dir, &selection, roots, out);
    }
}

/// What decides how a package's tests are built and run, beyond their
/// sources: its build script, its manifest's test settings, the workspace
/// profiles, and every `.cargo/config.toml` cargo reads from the cwd up.
/// Each is pinned, absent ones absent: adding one is a change too.
fn build_settings(dir: &Path, cwd: &Path, roots: &Roots, out: &mut Resolution) {
    let item = |out: &mut Resolution, file: &Path, key: &str| {
        if let Some((root, path)) = roots.relative(file) {
            out.found.push(Found {
                root,
                path,
                item: Some(key.to_string()),
                role: "manifest test setting".into(),
            });
        }
    };
    out_file(roots, &dir.join("build.rs"), "build script", out);
    let manifest = dir.join("Cargo.toml");
    for key in [
        "toml:table:lib",
        "toml:key:package.autotests",
        "toml:key:package.build",
        "toml:table:profile.test",
        "toml:table:profile.dev",
    ] {
        item(out, &manifest, key);
    }
    let workspace = workspace_root(dir, roots).join("Cargo.toml");
    if workspace != manifest {
        for key in ["toml:table:profile.test", "toml:table:profile.dev"] {
            item(out, &workspace, key);
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for base in cwd.ancestors().chain(dir.ancestors()) {
        if roots.relative(base).is_none() || !seen.insert(base.to_path_buf()) {
            continue;
        }
        for name in ["config.toml", "config"] {
            out_file(
                roots,
                &base.join(".cargo").join(name),
                "cargo configuration",
                out,
            );
        }
    }
}

fn resolve_package(dir: &Path, selection: &Selection, roots: &Roots, out: &mut Resolution) {
    for name in &selection.tests {
        integration_target(dir, name, roots, out);
    }
    if !selection.tests.is_empty() {
        return;
    }
    let names: Vec<(String, bool)> = selection
        .filters
        .iter()
        .map(|filter| {
            let name = filter.rsplit("::").next().unwrap_or(filter).to_string();
            (name, selection.exact)
        })
        .collect();
    if names.is_empty() || selection.filterset {
        // The whole suite runs: every integration test file and every unit
        // test function.
        if !selection.lib_only {
            for file in files_under(&dir.join("tests")) {
                if file.extension().and_then(|e| e.to_str()) == Some("rs") {
                    out_file(roots, &file, "package test suite file", out);
                }
            }
        }
        src_items(roots, dir, None, out);
        return;
    }
    for (name, exact) in names {
        let mut any = src_items(roots, dir, Some((&name, exact)), out);
        if !selection.lib_only {
            for file in files_under(&dir.join("tests")) {
                let defines = std::fs::read_to_string(&file)
                    .is_ok_and(|text| !matching_tests(&text, &name, exact).is_empty());
                if defines {
                    any = true;
                    test_module(roots, dir, &file, out);
                }
            }
        }
        if !any {
            match roots.relative(dir) {
                Some((root, rel)) => out.watches.push(Watch {
                    root,
                    dir: rel,
                    test_name: Some(name.clone()),
                    exact,
                    excluded: nested_packages(dir, roots),
                }),
                None => out.unresolved.push(format!(
                    "test `{name}` is defined nowhere yet, and its package {} lies outside both roots",
                    dir.display()
                )),
            }
        }
    }
}

fn out_file(roots: &Roots, abs: &Path, role: &str, out: &mut Resolution) {
    match roots.relative(abs) {
        Some((root, path)) => out.found.push(Found {
            root,
            path,
            item: None,
            role: role.into(),
        }),
        None => out.unresolved.push(format!(
            "{} ({role}) lies outside the repository and the project",
            abs.display()
        )),
    }
}

/// The integration target `name` of the package at `dir`: its `[[test]]`
/// path, else `tests/<name>.rs`, else `tests/<name>/main.rs`; absent, it is
/// pinned absent at `tests/<name>.rs` and `tests/<name>/` is watched.
fn integration_target(dir: &Path, name: &str, roots: &Roots, out: &mut Resolution) {
    // The manifest entry that selects the target -- or its absence -- is
    // pinned with it: adding or editing one can point the target elsewhere.
    if let Some((root, path)) = roots.relative(&dir.join("Cargo.toml")) {
        out.found.push(Found {
            root,
            path,
            item: Some(test_entry_key(name)),
            role: "manifest test entry".into(),
        });
    }
    let declared = declared_test_path(dir, name).map(|path| dir.join(path));
    let flat = dir.join("tests").join(format!("{name}.rs"));
    let nested = dir.join("tests").join(name).join("main.rs");
    let chosen = [declared.clone(), Some(flat.clone()), Some(nested)]
        .into_iter()
        .flatten()
        .find(|path| path.is_file());
    match chosen {
        Some(file) => rust_file_tree(roots, &file, true, "integration test target", out),
        None => {
            let target = declared.unwrap_or(flat);
            out_file(
                roots,
                &target,
                "integration test target (absent at pin)",
                out,
            );
            if let Some((root, rel)) = roots.relative(&dir.join("tests").join(name)) {
                out.watches.push(Watch {
                    root,
                    dir: rel,
                    test_name: None,
                    exact: false,
                    excluded: Vec::new(),
                });
            }
        }
    }
}

/// The `path` of the package's `[[test]]` table named `name`.
fn declared_test_path(dir: &Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("Cargo.toml")).ok()?;
    let manifest: toml::Value = toml::from_str(&text).ok()?;
    manifest
        .get("test")?
        .as_array()?
        .iter()
        .find(|table| table.get("name").and_then(toml::Value::as_str) == Some(name))?
        .get("path")?
        .as_str()
        .map(str::to_string)
}

/// The package directories the command selects: each `-p NAME` found in the
/// workspace at or above `start`, else the package at `start`, else every
/// member of a virtual workspace there.
fn package_dirs(
    start: &Path,
    roots: &Roots,
    names: &[String],
    out: &mut Resolution,
) -> Vec<PathBuf> {
    let workspace = workspace_root(start, roots);
    let mut manifests = manifest_dirs(&workspace);
    // A project cwd in a combined acceptance view runs cargo over the
    // repository overlaid on it.
    if let Some(overlaid) = roots.overlaid(start)
        && overlaid.is_dir()
    {
        manifests.extend(manifest_dirs(&workspace_root(&overlaid, roots)));
    }
    if names.is_empty() {
        let at = [start.to_path_buf()]
            .into_iter()
            .chain(roots.overlaid(start))
            .find(|dir| package_name(dir).is_some());
        if let Some(at) = at {
            return vec![at];
        }
        return manifests
            .into_iter()
            .filter(|dir| package_name(dir).is_some())
            .collect();
    }
    let mut dirs = Vec::new();
    for name in names {
        let hit = manifests
            .iter()
            .find(|dir| package_name(dir).as_deref() == Some(name.as_str()));
        match hit {
            Some(dir) => dirs.push(dir.clone()),
            None => out.unresolved.push(format!(
                "package `{name}` is named by no manifest under {}",
                workspace.display()
            )),
        }
    }
    dirs
}

/// The outermost directory at or above `start`, inside the repository or
/// project, holding a `Cargo.toml` with a `[workspace]`; else `start`.
fn workspace_root(start: &Path, roots: &Roots) -> PathBuf {
    let mut best = start.to_path_buf();
    let mut dir = Some(start);
    while let Some(current) = dir {
        if roots.relative(current).is_none() {
            break;
        }
        let manifest = current.join("Cargo.toml");
        if std::fs::read_to_string(&manifest).is_ok_and(|text| text.contains("[workspace]")) {
            best = current.to_path_buf();
        }
        dir = current.parent();
    }
    best
}

/// `dir` and every directory up to three levels below it holding a
/// `Cargo.toml`, build output and hidden directories skipped.
fn manifest_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((current, depth)) = stack.pop() {
        if current.join("Cargo.toml").is_file() {
            out.push(current.clone());
        }
        if depth == 3 {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || matches!(name.as_str(), "target" | "node_modules") {
                continue;
            }
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                stack.push((entry.path(), depth + 1));
            }
        }
    }
    out.sort();
    out
}

/// The other packages below `dir`, tree-relative.
fn nested_packages(dir: &Path, roots: &Roots) -> Vec<String> {
    manifest_dirs(dir)
        .into_iter()
        .filter(|nested| nested != dir)
        .filter_map(|nested| roots.relative(&nested).map(|(_, rel)| rel))
        .collect()
}

/// The `[package] name` of the manifest in `dir`.
fn package_name(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("Cargo.toml")).ok()?;
    let manifest: toml::Value = toml::from_str(&text).ok()?;
    manifest
        .get("package")?
        .get("name")?
        .as_str()
        .map(str::to_string)
}
