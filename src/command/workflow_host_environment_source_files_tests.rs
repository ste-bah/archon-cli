use super::*;
use crate::command::workflow_host_command_logic_items::{child_file, facts};

pub(super) fn production_files(base: &Path) -> BTreeSet<PathBuf> {
    let manifests = std::iter::once(base.to_owned())
        .chain(
            std::fs::read_dir(base.join("crates"))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.join("Cargo.toml").is_file()),
        )
        .collect::<Vec<_>>();
    let mut paths = BTreeSet::new();
    let mut roots = Vec::new();
    for manifest in manifests {
        for entry in std::fs::read_dir(&manifest).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "rs") {
                paths.insert(path.canonicalize().unwrap());
            }
        }
        for root in ["src/lib.rs", "src/main.rs", "build.rs"] {
            let root = manifest.join(root);
            if root.is_file() {
                roots.push((root, manifest.clone()));
            }
        }
        for entry in walkdir::WalkDir::new(&manifest)
            .into_iter()
            .filter_entry(|entry| {
                entry.depth() == 0
                    || !entry.file_type().is_dir()
                    || (!entry.file_name().to_string_lossy().starts_with('.')
                        && !matches!(
                            entry.file_name().to_str(),
                            Some(
                                "test"
                                    | "tests"
                                    | "fixtures"
                                    | "testdata"
                                    | "target"
                                    | "vendor"
                                    | "node_modules"
                                    | "examples"
                                    | "benches"
                            )
                        ))
            })
        {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
                paths.insert(path.canonicalize().unwrap());
                if path.strip_prefix(manifest.join("src/bin")).is_ok() {
                    roots.push((path.to_owned(), manifest.clone()));
                }
            }
        }
    }
    // Module reachability only removes files owned solely by test modules.
    // Discovery itself is a directory walk: unreferenced source is still read.
    let production = reachable(&roots, false);
    paths.extend(production.iter().cloned());
    let including_tests = reachable(&roots, true);
    let test_files = including_tests
        .difference(&production)
        .collect::<BTreeSet<_>>();
    paths.retain(|path| {
        !build_output(path)
            && !test_files.contains(path)
            && path.file_name().is_none_or(|name| {
                let name = name.to_string_lossy();
                name != "tests.rs" && !name.ends_with("_tests.rs")
            })
    });
    paths
}

fn reachable(roots: &[(PathBuf, PathBuf)], including_tests: bool) -> BTreeSet<PathBuf> {
    let mut pending = roots.to_vec();
    let mut visited = BTreeSet::new();
    while let Some((path, manifest)) = pending.pop() {
        if build_output(&path) {
            continue;
        }
        let path = path.canonicalize().unwrap();
        if build_output(&path) {
            continue;
        }
        if !visited.insert(path.clone()) {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let text = if including_tests {
            without_test_attributes(&text)
        } else {
            token_text(&production_tokens_at(&path, &text).tokens)
        };
        let declarations = facts(&text);
        if declarations.test_only {
            visited.remove(&path);
            continue;
        }
        for (scope, name, attr) in declarations.mods {
            let child = child_file(&path, &scope, &name, attr.as_deref());
            if child.is_file() {
                pending.push((child, manifest.clone()));
            }
        }
        for (code, from_manifest, relative) in declarations.includes {
            if code {
                let parent = if from_manifest {
                    &manifest
                } else {
                    path.parent().unwrap()
                };
                let relative = if from_manifest {
                    relative.trim_start_matches('/')
                } else {
                    &relative
                };
                pending.push((parent.join(relative), manifest.clone()));
            }
        }
    }
    visited
}

pub(super) fn build_output(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "target")
}

fn without_test_attributes(text: &str) -> String {
    let (tokens, _, _) = name_lexer::lex_names(text);
    let mut out = Vec::new();
    let mut at = 0;
    while at < tokens.len() {
        if is(tokens.get(at), '#') {
            let (attr, after) = attribute(&tokens, at);
            if test_cfg(attr) {
                at = after;
                continue;
            }
        }
        out.push(tokens[at].clone());
        at += 1;
    }
    token_text(&out)
}

fn token_text(tokens: &[Tok]) -> String {
    tokens
        .iter()
        .map(|token| match token {
            Tok::Ident(text) | Tok::Number(text) => text.clone(),
            Tok::Str(text) => format!("{text:?}"),
            Tok::PathSep => "::".into(),
            Tok::Punct(ch) => ch.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}
