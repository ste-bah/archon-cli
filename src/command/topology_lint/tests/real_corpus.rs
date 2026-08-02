//! The lints against the real seventeen-task PRD.
//!
//! These are the tests that stop the suite being a set of opinions about
//! synthetic graphs. `tests/fixtures/prd-trading-data-lake-ahdm-001` is a real
//! decomposed PRD written by a user, checked in verbatim, with real
//! `depends_on`, real `deliverable_contracts`, and real prose in its
//! `## Files Expected to Change` sections.

use std::path::PathBuf;

use archon_topology::EdgeSupport;
use archon_topology::ir::{TaskGraph, WriteTarget};

use crate::command::topology_lint::{LintSource, run_lint};
use crate::command::workflow_live::workflow_live_task_universe::task_graph_from_root;

pub(super) fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/prd-trading-data-lake-ahdm-001")
}

fn real_task_graph() -> TaskGraph {
    task_graph_from_root(&fixture_root()).expect("the seventeen real task files lower")
}

fn registry() -> WriteTarget {
    WriteTarget::Artifact(".archon/trading-lab/data/registry.json".to_string())
}

#[test]
fn the_seventeen_real_tasks_lower_with_declared_dataflow_on_both_sides() {
    let graph = real_task_graph();
    assert_eq!(graph.len(), 17, "the PRD decomposes into 17 tasks");

    // Production is what makes the classification able to conclude anything
    // about the upstream end of an edge.
    let producing = graph
        .nodes
        .iter()
        .filter(|node| node.writes_are_known())
        .count();
    assert_eq!(
        producing, 17,
        "every task declares files it changes or artifacts it produces"
    );

    // Consumption is the half the design believed did not exist. It is sparse
    // in the real corpus — most tasks declare only what they produce — and that
    // sparseness is the reason the lint stays silent on most edges rather than
    // reporting them all. If this ever reaches zero the lint has gone silent,
    // not clean, and the assertion below is what would catch it.
    let consuming: Vec<&str> = graph
        .nodes
        .iter()
        .filter(|node| node.consumption_is_known())
        .map(|node| node.id.as_str())
        .collect();
    assert!(
        consuming.len() >= 2,
        "at least two tasks must name an upstream artifact; found {consuming:?}"
    );
}

/// The finding that motivated three-way classification, pinned.
///
/// `TASK-TDL-080` (the coverage-matrix command) declares a dependency on all
/// six upstream tasks and consumes the registry. Five of those six are ingest
/// tasks that declare a registry entry, so those edges carry dataflow.
/// `TASK-TDL-042` is the CLI/TUI surface: `deliverable_contracts: []` and
/// source-only production. Nothing flows across that edge and nothing should —
/// the command surface has to exist before a command that uses it, which is
/// ordering, not dataflow.
///
/// Pinned as an exact set rather than a count so a change in either direction —
/// a task file gaining a declaration, or the classification drifting — fails
/// here and names what moved.
#[test]
fn the_real_graph_reports_the_cli_surface_edge_as_ordering_only_and_nothing_as_a_defect() {
    let graph = real_task_graph();
    let edges = graph.classify_edges().expect("valid graph");

    let mut ordering: Vec<String> = edges
        .iter()
        .filter(|edge| edge.support == EdgeSupport::OrderingOnly)
        .map(|edge| format!("{} -> {}", edge.dependent, edge.dependency))
        .collect();
    ordering.sort();
    assert_eq!(ordering, vec!["TASK-TDL-080 -> TASK-TDL-042"]);

    let mut unsupported: Vec<String> = edges
        .iter()
        .filter(|edge| edge.is_defect())
        .map(|edge| format!("{} -> {}", edge.dependent, edge.dependency))
        .collect();
    unsupported.sort();
    assert!(
        unsupported.is_empty(),
        "no edge in the corrected corpus is a defect; found {unsupported:?}"
    );
}

/// The five ingest edges the registry declaration repaired. Asserted by name so
/// that losing one shows up as a named regression rather than a count.
#[test]
fn the_five_ingest_edges_into_the_coverage_matrix_carry_dataflow() {
    let graph = real_task_graph();
    let edges = graph.classify_edges().expect("valid graph");
    for ingest in [
        "TASK-TDL-040",
        "TASK-TDL-041",
        "TASK-TDL-050",
        "TASK-TDL-060",
        "TASK-TDL-070",
    ] {
        let edge = edges
            .iter()
            .find(|edge| edge.dependent == "TASK-TDL-080" && edge.dependency == ingest)
            .unwrap_or_else(|| panic!("TDL-080 -> {ingest} is classified"));
        assert_eq!(
            edge.support,
            EdgeSupport::Dataflow,
            "{ingest} declares the registry entry TDL-080 consumes"
        );
    }
}

/// Under-declaration, reintroduced. Strip one ingest task's registry contract
/// and its edge must come back as a defect — and the remedy must name the
/// producer as the likely cause rather than telling the reader to drop the
/// edge, because dropping it while the write is real would let the two race.
#[test]
fn removing_one_ingest_registry_contract_brings_the_finding_back() {
    let temp = tempfile::tempdir().expect("temp dir");
    let stripped = "TASK-TDL-070";
    for entry in std::fs::read_dir(fixture_root()).expect("read fixtures") {
        let path = entry.expect("fixture entry").path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("TASK-") {
            continue;
        }
        let raw = std::fs::read_to_string(&path).expect("read fixture");
        let raw = if name.contains(stripped) {
            raw.replace(
                "  - kind: trading_data_registry_entry\n    artifact_path: \
                 .archon/trading-lab/data/registry.json\n",
                "",
            )
        } else {
            raw
        };
        std::fs::write(temp.path().join(name), raw).expect("write fixture copy");
    }

    let graph = task_graph_from_root(temp.path()).expect("the stripped corpus still lowers");
    assert!(
        !graph
            .node(stripped)
            .expect("stripped task present")
            .writes
            .contains(&registry()),
        "the contract really was removed"
    );

    let found = graph.unsupported_edges().expect("valid graph");
    let edge = found
        .iter()
        .find(|edge| edge.dependent == "TASK-TDL-080" && edge.dependency == stripped)
        .unwrap_or_else(|| panic!("the under-declared edge must return; found {found:?}"));

    let remedy = edge.remedy();
    assert!(
        remedy.contains("under-declare"),
        "the remedy must name the producer cause: {remedy}"
    );
    assert!(
        remedy.contains("The graph favours the second"),
        "and rank it, since nothing consumes what TDL-070 still declares: {remedy}"
    );
    assert!(
        remedy.contains("before considering dropping the edge"),
        "and must not lead with dropping the edge: {remedy}"
    );
}

/// A prior run's findings are appended into the task file between markers, and
/// they quote reviewer evidence verbatim — including artifact paths the task
/// never claimed to read. Attributing those to the author invents a
/// declaration. `TASK-TDL-070` is the case that proved it: an appended finding
/// quotes an absolute path ending in `registry.json`, which briefly made the
/// task look like a registry *consumer*.
#[test]
fn appended_prior_run_findings_are_not_read_as_declared_consumption() {
    let graph = real_task_graph();
    let node = graph.node("TASK-TDL-070").expect("TDL-070 present");
    assert!(
        !node.reads.contains(&registry()),
        "TDL-070 writes the registry and does not read it; only its appended findings mention one"
    );
    assert!(
        node.writes.contains(&registry()),
        "and it declares the write"
    );
}

#[test]
fn the_registry_dataflow_between_tdl_010_and_tdl_020_is_recovered() {
    let graph = real_task_graph();
    let producer = graph.node("TASK-TDL-010").expect("TDL-010 present");
    assert!(
        producer.writes.contains(&registry()),
        "TDL-010 is contracted to produce the registry"
    );
    let consumer = graph.node("TASK-TDL-020").expect("TDL-020 present");
    assert!(
        consumer.reads.contains(&registry()),
        "TDL-020's contract declares the registry as its input"
    );
    assert!(
        consumer.depends_on.contains(&"TASK-TDL-010".to_string()),
        "and it declares the dependency too"
    );
}

/// The edge that carries the recovered dataflow must not be reported. A lint
/// that flags a real edge is worse than one that flags nothing.
#[test]
fn the_real_dataflow_edge_is_not_reported_as_a_defect() {
    let graph = real_task_graph();
    let found = graph
        .unsupported_edges()
        .expect("the real graph is structurally valid");
    assert!(
        !found
            .iter()
            .any(|edge| edge.dependent == "TASK-TDL-020" && edge.dependency == "TASK-TDL-010"),
        "TDL-020 -> TDL-010 carries declared dataflow"
    );
}

/// Every reported edge must be a real declared edge, and the dependent must
/// really have declared some consumption. This is the invariant that keeps the
/// output honest: a finding against a node that declared nothing would be the
/// unknown-dataflow rule being violated.
#[test]
fn every_classified_edge_on_the_real_graph_is_well_founded() {
    let graph = real_task_graph();
    for edge in graph.classify_edges().expect("valid graph") {
        let dependent = graph.node(&edge.dependent).expect("dependent exists");
        let dependency = graph.node(&edge.dependency).expect("dependency exists");
        assert!(
            dependent.depends_on.contains(&edge.dependency),
            "{} does not actually depend on {}",
            edge.dependent,
            edge.dependency
        );
        assert!(
            dependent.consumption_is_known(),
            "{} declared no consumption; the lint must have stayed silent",
            edge.dependent
        );
        assert!(
            dependency.writes_are_known(),
            "{} declared no production; the lint must have stayed silent",
            edge.dependency
        );
    }
}

#[test]
fn the_real_graph_renders_a_report_naming_its_nodes() {
    let text = run_lint(&fixture_root(), &LintSource::Tasks(fixture_root()))
        .expect("the real task set lints");
    assert!(text.contains("advisory only"), "the report says it advises");
    assert!(
        text.contains("## dependency edges"),
        "all three sections present"
    );
    assert!(text.contains("## diamond conformance"));
    assert!(text.contains("## stop-rule fusion"));
    assert!(
        text.contains("TASK-TDL-"),
        "findings name specific tasks:\n{text}"
    );
    assert!(
        text.contains("ordering-only (not findings"),
        "the ordering-only edge is shown as an observation, not a defect:\n{text}"
    );
    assert!(
        text.contains("no unsupported edges."),
        "and the corrected corpus reports no defect:\n{text}"
    );
}
