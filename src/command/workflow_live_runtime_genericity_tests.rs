//! The runtime-genericity gate (D52/D75).
//!
//! This lives in the bin crate rather than beside the task-universe code it
//! grew up with because it scans several source trees — `src/command/workflow_live*`
//! here and the engine crates next door — and it finds them all by walking out
//! from `CARGO_MANIFEST_DIR`. Only the workspace-root crate has a manifest
//! directory from which every tree resolves. Moving it into archon-workflow
//! silently pointed it at paths that do not exist, and a `read_dir` on a
//! missing directory is the one failure mode a scanning gate must never treat
//! as "nothing to report" — hence `SCANNED_CRATE_ROOTS` is checked for
//! emptiness per root, not just in aggregate.
//!
//! The scan covers `archon-workflow` and every other engine crate, because
//! limiting it to one crate is how the leak it exists to prevent actually
//! happened: three provider-specific variable names sat in `archon-core` for
//! months, in a hardcoded list, unseen — that crate was simply never looked at.

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
        // Added after the gate was widened and immediately tested against the
        // leak it was widened for: of the three provider variable names found
        // sitting in `archon-core`, `ARCHON_STOOQ_CSV_URL` matched nothing on
        // this list. A gate that misses one of the three cases that motivated
        // it is worse than none, because it certifies the crate as clean.
        "stooq",
        "yfinance",
    ];
    // Every crate that must stay domain-free. `archon-trading` is deliberately
    // absent: it *is* the domain, and scanning it would be a category error.
    const SCANNED_CRATE_ROOTS: &[&str] = &[
        "crates/archon-workflow/src",
        "crates/archon-core/src",
        "crates/archon-tools/src",
        "crates/archon-llm/src",
        "crates/archon-tui/src",
    ];
    // Known debt, named so it is visible rather than absent. `archon-tools`
    // carries a trading tool binding — command names, permission gating — which
    // belongs with the trading crate. Listing it here keeps the rest of
    // `archon-tools` gated today instead of leaving the whole crate unscanned
    // until the move happens.
    const KNOWN_DOMAIN_DEBT: &[&str] = &["crates/archon-tools/src/trading"];

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
    assert!(
        !runtime_sources.is_empty(),
        "gate found no workflow_live runtime sources to scan"
    );
    for root in SCANNED_CRATE_ROOTS {
        let before = runtime_sources.len();
        collect_workflow_crate_sources(&manifest_dir.join(root), &mut runtime_sources);
        assert!(
            runtime_sources.len() > before,
            "gate found no sources under {root}; a renamed or moved crate must \
             fail this gate rather than silently shrink its own coverage"
        );
    }
    runtime_sources.retain(|path| {
        let text = path.to_string_lossy().replace('\\', "/");
        !KNOWN_DOMAIN_DEBT
            .iter()
            .any(|excluded| text.contains(excluded))
    });
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
