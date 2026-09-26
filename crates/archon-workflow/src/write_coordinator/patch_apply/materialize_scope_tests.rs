//! The destination allowlist, and the gate that keeps [`ENGINE_LOADED`]
//! derived from the code.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::*;

/// `.archon/` namespaces runtime code reads only as the product's own data
/// stores -- a declared deliverable may legitimately live there. Anything the
/// engine loads behaviour from belongs in [`ENGINE_LOADED`] instead.
const DATA_STORES: &[&str] = &[
    // archon-trading's data store (`data_store/records.rs`) and the trading
    // providers under `src/command/trading_data_provider/`: datasets and
    // strategy artifacts, never configuration.
    "trading-lab",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Runtime `.rs` sources: no test files, test directories or build output.
fn runtime_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.contains("test") || name == "target" || name == "benches" || name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            runtime_sources(&path, out);
        } else if name.ends_with(".rs") {
            out.push(path);
        }
    }
}

fn name_at(text: &str) -> Option<String> {
    let name: String = text
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'))
        .collect();
    (!name.is_empty()).then(|| name.to_ascii_lowercase())
}

/// Every `.archon/<name>` a source names: `".archon/<name>`,
/// `join(".archon")` followed by `.join("<name>")`, and `".archon", "<name>"`.
fn archon_names(source: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for (at, _) in source.match_indices("\".archon/") {
        names.extend(name_at(&source[at + 9..]));
    }
    for (at, _) in source.match_indices("join(\".archon\")") {
        let rest = source[at + 15..].trim_start();
        if let Some(rest) = rest.strip_prefix(".join(\"") {
            names.extend(name_at(rest));
        }
    }
    for (at, _) in source.match_indices("\".archon\",") {
        let rest = source[at + 10..].trim_start();
        if let Some(rest) = rest.strip_prefix('"') {
            names.extend(name_at(rest));
        }
    }
    names
}

#[test]
fn every_archon_directory_the_runtime_names_is_classified() {
    let root = workspace_root();
    let mut sources = Vec::new();
    runtime_sources(&root.join("src"), &mut sources);
    runtime_sources(&root.join("crates"), &mut sources);
    assert!(
        sources.len() > 500,
        "the scan found {} sources",
        sources.len()
    );
    let mut found = BTreeSet::new();
    for path in &sources {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let runtime = text.split("\n#[cfg(test)]").next().unwrap_or(&text);
        for name in archon_names(runtime) {
            let classified =
                ENGINE_LOADED.contains(&name.as_str()) || DATA_STORES.contains(&name.as_str());
            assert!(
                classified,
                "{} names `.archon/{name}`, which is neither ENGINE_LOADED nor a DATA_STORE: \
                 classify it before a landing may write there",
                path.display()
            );
            found.insert(name);
        }
    }
    for loaded in [
        "agents",
        "plugins",
        "skills",
        "hooks.toml",
        "settings.json",
        "workflows",
    ] {
        assert!(
            found.contains(loaded),
            "the scan no longer finds `.archon/{loaded}`"
        );
    }
    for listed in ENGINE_LOADED {
        assert_eq!(listed.to_ascii_lowercase(), *listed, "lower-case only");
    }
}

#[test]
fn the_scanner_reads_every_spelling() {
    let names = archon_names(
        r#"root.join(".archon/agents/x.md"); root.join(".archon")
            .join("plugins"); segments([".archon", "skills"]);"#,
    );
    assert_eq!(
        names.into_iter().collect::<Vec<_>>(),
        vec!["agents", "plugins", "skills"]
    );
}

#[test]
fn only_a_namespace_no_engine_code_loads_from_is_a_destination() {
    let root = "/project";
    for refused in [
        ".archon/agents/verifier.md",
        ".archon/Agents/verifier.md",
        ".archon/plugins/p/plugin.json",
        ".archon/skills/s/SKILL.md",
        ".archon/workflows/run/state.json",
        ".archon/hooks.toml",
        ".archon/.hidden/x",
        ".archon/lab",
        "docs/report.md",
        ".archon/lab/../agents/x.md",
    ] {
        assert_eq!(destination(root, refused), None, "{refused}");
    }
    assert_eq!(
        destination(root, ".archon/lab/strategies/out.json"),
        Some(PathBuf::from("/project/.archon/lab/strategies/out.json"))
    );
}
