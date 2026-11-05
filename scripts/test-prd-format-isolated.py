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
        "obligation_ids.rs", "obligation_constraints.rs", "obligation_continuations.rs",
        "obligation_source_sections.rs", "obligation_ids_tests.rs",
        "obligation_constraints_tests.rs", "obligation_format_tests.rs",
        "prd_sections.rs", "prd_sections_tests.rs", "prd_markdown.rs",
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
            text = re.sub(r'#\[test\]\nfn a_task_files_focused_tests_parse_into_commands_and_required_tools\(\) \{.*?\n\}',
                          '', text, flags=re.S)
        destination = target / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(text)
    contract = source("task_set_contract.rs")
    start = contract.rfind("#[derive", 0, contract.index("pub struct AcceptanceContract"))
    end = contract.rfind("#[derive", 0, contract.index("pub enum FreezeGateMode"))
    (target / "types.rs").write_text("use serde::{Serialize, Deserialize};\nuse std::collections::BTreeSet;\ntype WorkflowV2DeliverableContract = serde_json::Value;\n" + contract[start:end])
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
// Coverage only reads these data fields; full task parsing is integration-only.
mod task_universe {
    #[derive(Default)] pub struct WorkflowV2TaskUniverseTask {
        pub canonical_task_id: String, pub source_path: String,
        pub implements: Vec<String>,
    }
    pub struct WorkflowV2TaskUniverse {
        pub schema_version: String, pub source_roots: Vec<String>,
        pub tasks: Vec<WorkflowV2TaskUniverseTask>,
    }
}
''')
    command = ["rustc", "--edition=2024", "-A", "dead_code", "-A", "unused_imports", "--test", str(target / "harness.rs"),
               "-L", f"dependency={DEPS}", "-o", str(target / "tests")]
    command += shlex.split(os.environ.get("PRD_TEST_RUSTFLAGS", ""))
    for name in ["regex", "serde", "serde_json", "thiserror"]:
        candidates = sorted(DEPS.glob(f"lib{name}-*.rlib"))
        if not candidates:
            raise RuntimeError(f"missing existing dependency: {name}")
        selected = next((value.split("=", 1)[1] for value in sys.argv[2:] if value.startswith(name + "=")), str(candidates[0]))
        command += ["--extern", f"{name}={selected}"]
    subprocess.run(command, check=True)
    subprocess.run([str(target / "tests")], check=True)
