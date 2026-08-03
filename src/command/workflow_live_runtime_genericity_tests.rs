//! The runtime-genericity gate (D52/D75).
//!
//! This lives in the bin crate rather than beside the task-universe code it
//! grew up with because it scans two source trees — `src/command/workflow_live*`
//! here and `crates/archon-workflow/src` next door — and it finds both by
//! walking out from `CARGO_MANIFEST_DIR`. Only the workspace-root crate has a
//! manifest directory from which both halves resolve. Moving it into
//! archon-workflow silently pointed it at paths that do not exist, and a
//! `read_dir` on a missing directory is the one failure mode a scanning gate
//! must never treat as "nothing to report".

use std::fs;

#[test]
fn runtime_workflow_code_contains_no_fixture_task_ids() {
    // D52/D75 gate: the generic workflow runtime must carry NO fixture ids,
    // fixture paths, or fixture-domain vocabulary. Ids/paths would break other
    // PRDs outright; domain vocabulary is how fixture assumptions quietly
    // fossilize into "generic" prompts and detectors.
    const FIXTURE_LITERALS: &[&str] = &["task-tdl", "trading-lab"];
    const DOMAIN_VOCABULARY: &[&str] = &[
        "backtest",
        "paper trading",
        "paper-trading",
        "paper_trading",
        "paper-readiness",
        "pine",
        "ohlcv",
        "polygon",
        "tradingview",
        "openbb",
    ];
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut runtime_sources = Vec::new();
    for entry in fs::read_dir(manifest_dir.join("src/command")).expect("read command sources") {
        let path = entry.expect("source entry").path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with("workflow_live") && name.ends_with(".rs") && !name.contains("_tests") {
            runtime_sources.push(path);
        }
    }
    collect_workflow_crate_sources(
        &manifest_dir.join("crates/archon-workflow/src"),
        &mut runtime_sources,
    );
    assert!(
        !runtime_sources.is_empty(),
        "gate found no runtime sources to scan"
    );
    for path in runtime_sources {
        let source = fs::read_to_string(&path).expect("read runtime source");
        let runtime_only = source
            .split("\n#[cfg(test)]")
            .next()
            .unwrap_or(&source)
            .to_ascii_lowercase();
        for literal in FIXTURE_LITERALS {
            assert!(
                !runtime_only.contains(literal),
                "fixture literal '{literal}' leaked into runtime source {}",
                path.display()
            );
        }
        for word in DOMAIN_VOCABULARY {
            assert!(
                !runtime_only.contains(word),
                "fixture-domain vocabulary '{word}' leaked into runtime source {}",
                path.display()
            );
        }
    }
}

fn collect_workflow_crate_sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if path.is_dir() {
            if !name.contains("fixture") && !name.contains("tests") {
                collect_workflow_crate_sources(&path, out);
            }
            continue;
        }
        if name.ends_with(".rs") && !name.contains("_tests") && name != "tests.rs" {
            out.push(path);
        }
    }
}
