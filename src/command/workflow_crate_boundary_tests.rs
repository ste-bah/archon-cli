//! What `src/command/workflow*.rs` is allowed to name.
//!
//! Every one of those files is destined for `crates/archon-workflow`. That
//! crate sits *below* `archon-core` in the build graph — `archon-core` depends
//! on `archon-topology`, which depends on `archon-workflow` — so an edge from
//! `archon-workflow` onto `archon-pipeline` (which takes `archon-core`) is a
//! cycle, and an edge onto `archon-tui` would make "run a workflow" and "have a
//! terminal attached" the same capability.
//!
//! Both are now reached through ports: `llm_client_port` for the provider,
//! `ui_sink_port` for user-visible output, each implemented by a host adapter
//! named deliberately outside the `workflow*` prefix so this scan can be a
//! filename rule. The SONA learning cluster moved out of the prefix for the
//! same reason — it opens a Cozo store through `archon_pipeline::learning` and
//! is host machinery, not execution.
//!
//! The exemptions below are the files that are staying in the bin crate, plus
//! the test scaffolding that still holds a channel receiver. Each one is named
//! individually: a blanket "tests may do anything" rule would let the next
//! execution file acquire the dependency by adding `_tests` to its name.

use std::path::{Path, PathBuf};

/// Crates `archon-workflow` cannot depend on, and why.
const FORBIDDEN: &[(&str, &str)] = &[
    (
        "archon_pipeline",
        "archon-pipeline depends on archon-core, which depends on archon-topology, \
         which depends on archon-workflow. Reach the provider through \
         archon_workflow::llm_client_port and implement the adapter in a file outside \
         the workflow* prefix, as pipeline_workflow_llm.rs does.",
    ),
    (
        "archon_tui",
        "workflow execution must not require a terminal UI. Emit through \
         archon_workflow::ui_sink_port and let the host decide where it lands, as \
         tui_workflow_ui_sink.rs does.",
    ),
    (
        "archon_topology",
        "archon-topology depends on archon-workflow directly, so this is the tightest \
         cycle of the three and default-features = false does not break it -- Cargo \
         rejects the package-level cycle whether or not the optional feature is on. \
         Topology-facing code belongs beside its consumers in topology_task_graph.rs.",
    ),
];

/// Files under `src/command/workflow*.rs` that may still name a forbidden
/// crate, each for a stated reason.
const EXEMPT: &[(&str, &str)] = &[
    (
        "workflow.rs",
        "the slash/CLI surface, staying in the bin crate",
    ),
    (
        "workflow_live.rs",
        "the CLI entry point, staying in the bin crate; it builds the drained \
         channel a headless run still needs",
    ),
    (
        "workflow_tests.rs",
        "tests of the slash surface, which emits TuiEvents directly",
    ),
    (
        "workflow_live_tests.rs",
        "the delivery lint matches on the literal crate path it forbids",
    ),
    (
        "workflow_live_test_support.rs",
        "holds the receiver half so tests can assert on real delivery",
    ),
    (
        "workflow_live_v2_wire_tests.rs",
        "holds the receiver half so tests can assert on real delivery",
    ),
];

/// This scanner. Its own explanatory prose names the crates it forbids, and a
/// guard that fails on its own error messages teaches nothing.
const SELF: &str = "workflow_crate_boundary_tests.rs";

fn command_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/command")
}

fn workflow_sources() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(command_dir())
        .expect("read src/command")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|ext| ext.to_str()) == Some("rs")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("workflow"))
        })
        .collect();
    files.sort();
    files
}

fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .expect("source file name")
}

#[test]
fn workflow_command_sources_do_not_name_crates_archon_workflow_cannot_reach() {
    let files = workflow_sources();
    assert!(
        !files.is_empty(),
        "found no src/command/workflow*.rs sources; the guard would pass vacuously"
    );

    let mut offenders = Vec::new();
    for path in &files {
        let name = file_name(path);
        if name == SELF || EXEMPT.iter().any(|(exempt, _)| *exempt == name) {
            continue;
        }
        let text = std::fs::read_to_string(path).expect("read workflow source");
        for (crate_name, why) in FORBIDDEN {
            if text.contains(&format!("{crate_name}::")) {
                offenders.push(format!("{name} names {crate_name}. {why}"));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "src/command/workflow*.rs must stay movable into crates/archon-workflow:\n{}",
        offenders.join("\n")
    );
}

/// An exemption for a file that no longer needs one is an exemption that will
/// be inherited by whatever is written next under that name.
#[test]
fn every_boundary_exemption_is_still_earned() {
    let files = workflow_sources();
    let mut stale = Vec::new();
    for (name, reason) in EXEMPT {
        let Some(path) = files.iter().find(|path| file_name(path) == *name) else {
            stale.push(format!("{name} no longer exists ({reason})"));
            continue;
        };
        let text = std::fs::read_to_string(path).expect("read workflow source");
        if !FORBIDDEN
            .iter()
            .any(|(crate_name, _)| text.contains(&format!("{crate_name}::")))
        {
            stale.push(format!(
                "{name} names no forbidden crate; drop its exemption ({reason})"
            ));
        }
    }

    assert!(
        stale.is_empty(),
        "stale entries in the workflow crate-boundary exemption list:\n{}",
        stale.join("\n")
    );
}
