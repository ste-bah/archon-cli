#!/usr/bin/env python3
"""Run parser/coverage unit tests without a build-system invocation.

Usage: python3 scripts/test-prd-format-isolated.py /path/to/existing/deps
Only existing dependency artifacts are read. Generated sources and executables
live in a temporary directory. One full task-parser integration test needs the
complete library and is excluded; the command extractor's unit tests run.
"""
import os
import shlex
import pathlib
import re
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
SOURCE = ROOT / "crates/archon-workflow/src"
DEPS = pathlib.Path(sys.argv[1]).resolve()


def source(name):
    return (SOURCE / name).read_text()


with tempfile.TemporaryDirectory(prefix="prd-format-") as directory:
    target = pathlib.Path(directory)
    names = [
        "obligation_ids.rs", "obligation_declarations.rs", "obligation_inventory_fixture_tests.rs", "obligation_constraints.rs", "obligation_continuations.rs",
        "obligation_source_sections.rs", "obligation_ids_tests.rs",
        "obligation_constraints_tests.rs", "obligation_format_tests.rs",
        "requirement_inventory.rs", "requirement_inventory_tests.rs", "prd_sections.rs", "prd_sections_tests.rs", "prd_markdown.rs",
        "task_universe_focused_commands.rs", "task_universe_focused_commands_tests.rs",
        "v2/acceptance_coverage.rs", "v2/acceptance_coverage_tests.rs",
        "v2/prd_intake.rs", "v2/prd_intake_rules.rs", "v2/prd_intake_sections.rs", "v2/prd_intake_format_tests.rs",
        "v2/task_record.rs", "defect.rs",
    ]
    for name in names:
        text = source(name)
        text = re.sub(r'include_str!\("([^"]+)"\)', lambda match:
                      'include_str!("' + str((SOURCE / name).parent.joinpath(match[1]).resolve()) + '")', text)
        if name == "task_universe_focused_commands_tests.rs":
            text = text.replace('#[test]\nfn a_task_files_focused_tests_parse_into_commands_and_required_tools',
                                '#[cfg(any())]\n#[test]\nfn a_task_files_focused_tests_parse_into_commands_and_required_tools')
        destination = target / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(text)
    contract = source("task_set_contract.rs")
    start = contract.rfind("#[derive", 0, contract.index("pub struct AcceptanceContract"))
    end = contract.rfind("#[derive", 0, contract.index("pub enum FreezeGateMode"))
    (target / "types.rs").write_text("use serde::{Serialize, Deserialize};\nuse std::collections::BTreeSet;\ntype WorkflowV2DeliverableContract = serde_json::Value;\n" + contract[start:end] + "\npub const ACCEPTANCE_CONTRACT_FILE: &str = \"acceptance-contract.json\";\n")
    skeleton = source("task_skeleton.rs")
    start = skeleton.index("#[derive")
    end = skeleton.rfind("#[derive", 0, skeleton.index("pub struct TaskSkeletonLock"))
    if end < start:
        raise RuntimeError("cannot isolate skeleton data types")
    # Include the source's skeleton types, then its finding type, without I/O.
    finding_start = skeleton.rfind("#[derive", 0, skeleton.index("pub struct TaskSetFinding"))
    finding_end = skeleton.index("\n}", finding_start) + 2
    (target / "skeleton.rs").write_text("use serde::{Serialize, Deserialize};\ntype WorkflowV2DeliverableContract = serde_json::Value;\n" + skeleton[start:end] + skeleton[finding_start:finding_end])
    knowledge = (ROOT / "crates/archon-knowledge/src/traceability/requirements.rs").read_text()
    start = knowledge.rfind("#[derive", 0, knowledge.index("pub enum Severity"))
    end = knowledge.index("fn bullet_re")
    classify = knowledge[knowledge.index("fn classify"):knowledge.index("/// Project a requirement")]
    (target / "trace_types.rs").write_text("use serde::{Serialize, Deserialize};\n" + knowledge[start:end] + classify)
    (target / "external_obligation_tests.rs").write_text((ROOT / "crates/archon-workflow/tests/obligation_ids.rs").read_text())
    host_prd = (ROOT / "src/command/workflow_task_set_prd.rs").read_text()
    # Input-file validation is integration-only; keep the real enrichment code
    # and its unchanged regression assertions, with no digest stub.
    start = host_prd.index("pub(crate) fn validate_prd_input")
    end = host_prd.index("#[cfg(test)]", start)
    host_prd = host_prd[:start] + host_prd[end:]
    host_prd = host_prd.replace("use archon_workflow::task_set_contract::content_digest;", "")
    (target / "host_prd.rs").write_text(host_prd)
    (target / "trace_inventory.rs").write_text((ROOT / "src/command/requirement_trace/inventory.rs").read_text())
    (target / "harness.rs").write_text('''
#[path="types.rs"] pub mod task_set_contract;
#[path="skeleton.rs"] pub mod task_skeleton;
mod defect;
mod obligation_ids;
#[path="task_universe_focused_commands.rs"] mod focused;
#[path="v2/task_record.rs"] mod task_record;
pub use task_record::*;
#[path="v2/prd_intake.rs"] mod intake;
#[path="v2/acceptance_coverage.rs"] pub mod coverage;
extern crate self as archon_workflow;
extern crate self as archon_knowledge;
pub mod v2 { pub mod acceptance_stage { pub use crate::coverage; } }
pub mod traceability {
    pub mod requirements { include!("trace_types.rs"); }
    pub use requirements::Requirement;
}
mod trace_inventory;
mod requirement_inventory;
#[path="external_obligation_tests.rs"] mod external_obligation_tests;
#[path="host_prd.rs"] mod host_prd;
// Coverage only reads these data fields; full task parsing is integration-only.
mod task_universe {
    #[derive(Default)] pub struct WorkflowV2TaskUniverseTask {
        pub canonical_task_id: String, pub source_path: String,
        pub implements: Vec<String>,
    }
    #[derive(Default)] pub struct WorkflowV2TaskUniverse {
        pub schema_version: String, pub source_roots: Vec<String>,
        pub tasks: Vec<WorkflowV2TaskUniverseTask>,
    }
}
''')
    audit_path = os.environ.get("PRD_INVENTORY_AUDIT_PATH")
    if audit_path:
        baseline = target / "baseline"
        baseline.mkdir()
        for name in ["obligation_ids.rs", "obligation_constraints.rs", "obligation_continuations.rs",
                     "obligation_source_sections.rs", "prd_sections.rs"]:
            original = subprocess.check_output(["git", "show", "253a5c3bc:crates/archon-workflow/src/" + name], cwd=ROOT, text=True)
            (baseline / name).write_text(original.replace("#[cfg(test)]", "#[cfg(any())]"))
        with (target / "harness.rs").open("a") as harness:
            harness.write('''
#[path="baseline/obligation_ids.rs"] mod baseline;
#[test]
fn current_read_only_prd_matches_base_inventory() {
    let source = std::fs::read_to_string(std::env::var("PRD_INVENTORY_AUDIT_PATH").unwrap()).unwrap();
    let expected = baseline::obligation_texts(&source).into_iter()
        .filter(|(id, _)| id.starts_with("REQ-")).collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(expected.len(), 93);
    assert_eq!(obligation_ids::requirement_texts(&source), expected);
    assert_eq!(obligation_ids::obligation_ids(&source), baseline::obligation_ids(&source));
    assert_eq!(obligation_ids::acceptance_ids(&source), baseline::acceptance_ids(&source));
    assert_eq!(obligation_ids::constraint_ids(&source), baseline::constraint_ids(&source));
    let done = |rows: std::collections::BTreeMap<String, String>| rows.into_iter()
        .filter(|(id, _)| id.starts_with("DONE-")).collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(done(obligation_ids::obligation_texts(&source)).len(), 14);
    assert_eq!(done(obligation_ids::obligation_texts(&source)), done(baseline::obligation_texts(&source)));
}
''')
    command = ["rustc", "--edition=2024", "-A", "dead_code", "-A", "unused_imports", "--test", str(target / "harness.rs"),
               "-L", f"dependency={DEPS}", "-o", str(target / "tests")]
    command += shlex.split(os.environ.get("PRD_TEST_RUSTFLAGS", ""))
    for name in ["regex", "serde", "serde_json", "thiserror", "anyhow", "tempfile"]:
        candidates = sorted(DEPS.glob(f"lib{name}-*.rlib"))
        if not candidates:
            raise RuntimeError(f"missing existing dependency: {name}")
        selected = next((value.split("=", 1)[1] for value in sys.argv[2:] if value.startswith(name + "=")), str(candidates[0]))
        command += ["--extern", f"{name}={selected}"]
    subprocess.run(command, check=True)
    subprocess.run([str(target / "tests")], check=True)
