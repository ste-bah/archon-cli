use super::*;

#[path = "workflow_host_environment_shell_source_tests.rs"]
mod shell_source;

pub(super) fn production_sources(base: &Path) -> Vec<Source> {
    let mut pending = source_files::production_files(base)
        .into_iter()
        .map(|path| (path, true))
        .collect::<Vec<_>>();
    let mut visited = BTreeSet::new();
    let mut sources = Vec::new();
    while let Some((path, code)) = pending.pop() {
        if source_files::build_output(&path) {
            continue;
        }
        let path = path.canonicalize().unwrap();
        if source_files::build_output(&path) {
            continue;
        }
        if !visited.insert((path.clone(), code)) {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        let source = if code {
            production_tokens_at(&path, &text)
        } else if path.extension().is_some_and(|extension| extension == "sh")
            || text.starts_with("#!/bin/sh")
            || text.starts_with("#!/bin/bash")
            || text.starts_with("#!/usr/bin/env bash")
        {
            shell_source::source(&path, &text)
        } else {
            // Other embedded resources retain their original text and lines.
            Source {
                path: path.clone(),
                tokens: text.lines().map(|line| Tok::Str(line.to_owned())).collect(),
                lines: (1..=text.lines().count()).collect(),
            }
        };
        if code {
            let manifest = path
                .ancestors()
                .find(|dir| dir.join("Cargo.toml").is_file())
                .unwrap_or(base);
            for at in 0..source.tokens.len() {
                let Some(kind @ ("include" | "include_str" | "include_bytes")) =
                    word(source.tokens.get(at))
                else {
                    continue;
                };
                if !is(source.tokens.get(at + 1), '!') {
                    continue;
                }
                let location = format!(
                    "{}:{}",
                    path.to_string_lossy().replace('\\', "/"),
                    source.lines[at]
                );
                let (relative, _) = path_value(&source.tokens, at + 3, manifest)
                    .filter(|(_, end)| is(source.tokens.get(*end), ')'))
                    .unwrap_or_else(|| panic!("{location}: unresolved embedded resource"));
                assert!(
                    is(source.tokens.get(at + 2), '('),
                    "{location}: unresolved embedded resource"
                );
                // Generated output is derived data, not a source name origin.
                let Some(relative) = relative else {
                    continue;
                };
                let target = path.parent().unwrap().join(relative);
                if source_files::build_output(&target) {
                    continue;
                }
                assert!(
                    target.is_file(),
                    "{location}: unreadable embedded resource: {}",
                    target.display()
                );
                pending.push((target, kind == "include"));
            }
        }
        sources.push(source);
    }
    sources
}

// Consume the complete include argument. None in the value marks generated
// output without looking up or reading a build directory. Other computed paths
// fail closed through the outer Option.
fn path_value(tokens: &[Tok], at: usize, manifest: &Path) -> Option<(Option<String>, usize)> {
    if let Some(Tok::Str(text)) = tokens.get(at) {
        return Some((Some(text.clone()), at + 1));
    }
    if !is(tokens.get(at + 1), '!') || !is(tokens.get(at + 2), '(') {
        return None;
    }
    let mut cursor = at + 3;
    match word(tokens.get(at))? {
        "concat" => {
            let mut value = Some(String::new());
            while !is(tokens.get(cursor), ')') {
                let (part, end) = path_value(tokens, cursor, manifest)?;
                value = value.zip(part).map(|(mut value, part)| {
                    value.push_str(&part);
                    value
                });
                cursor = end;
                if is(tokens.get(cursor), ',') {
                    cursor += 1;
                } else if !is(tokens.get(cursor), ')') {
                    return None;
                }
            }
            Some((value, cursor + 1))
        }
        "env" if is(tokens.get(cursor + 1), ')') => {
            let Some(Tok::Str(name)) = tokens.get(cursor) else {
                return None;
            };
            let value = match name.as_str() {
                "CARGO_MANIFEST_DIR" => Some(manifest.to_str()?.to_owned()),
                "OUT_DIR" => None,
                _ => return None,
            };
            Some((value, cursor + 2))
        }
        _ => None,
    }
}
