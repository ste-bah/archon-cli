//! A frozen task set whose checks' sources are pinned, over the write-wave
//! fixture, shared by the PLAN-11 wave tests.
#![allow(dead_code)]
use std::path::PathBuf;

use archon_workflow::check_source_pins::{ORIGIN_FREEZE, PinStore, pin_contract};
use archon_workflow::check_source_resolve::Roots;
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, AcceptanceContract, content_digest,
};
use archon_workflow::*;
use serde_json::json;

use crate::support::{Fixture, git};

pub const JUDGE_TEST: &str = "#[test]\nfn judges() {\n    assert_eq!(demo::value(), 2);\n}\n";
pub const LIB: &str = "pub fn value() -> u32 {\n    1\n}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn unit_guard() {\n        assert_eq!(super::value(), 2);\n    }\n}\n";
pub const LIB_WEAKENED: &str = "pub fn value() -> u32 {\n    2\n}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn unit_guard() {}\n}\n";
pub const LATER_TEST: &str = "#[test]\nfn later() {\n    assert_eq!(demo::value(), 2);\n}\n";

pub fn project_root(f: &Fixture) -> PathBuf {
    f.store
        .root()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// A Rust package in the fixture repository and a frozen task set whose
/// three checks' sources are pinned, as `freeze-acceptance` pins them.
pub fn frozen_task_set(f: &Fixture) -> (PathBuf, AcceptanceContract) {
    for (path, text) in [
        ("Cargo.toml", "[package]\nname = \"demo\"\n"),
        ("src/lib.rs", LIB),
        ("tests/judge.rs", JUDGE_TEST),
    ] {
        let target = f.repo.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, text).unwrap();
    }
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-qm", "package"]);
    let check = |id: &str, command: &str| {
        json!({"id": id, "criterion": format!("{id}: value is 2"),
            "check": {"kind": "command", "command": command, "cwd": "repo_root"},
            "judgment": {"verdict": "accepted", "counterexample": "none", "reason": "ok", "host_call_id": "judge"}})
    };
    let contract: AcceptanceContract = serde_json::from_value(json!({
        "schema_version": 1, "prd": {"path": "prd.md", "digest": "d"}, "gap_policy": {},
        "acceptance": [
            check("AC-1", "cargo test -p demo --test judge judges -- --exact"),
            check("AC-2", "cargo test -p demo --lib unit_guard"),
            check("AC-3", "cargo test -p demo --test later"),
        ]
    }))
    .unwrap();
    let project = project_root(f);
    let tasks = project.join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    let bytes = serde_json::to_vec_pretty(&contract).unwrap();
    std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), &bytes).unwrap();
    let store = PinStore::frozen(&project, &tasks);
    let roots = Roots {
        repository: &f.repo,
        project: &project,
    };
    let pins = pin_contract(
        &contract,
        &content_digest(&bytes),
        &roots,
        ORIGIN_FREEZE,
        &store.blobs,
    );
    assert_eq!(
        pins.checks["AC-3"]
            .sources
            .iter()
            .find(|s| s.path == "tests/later.rs")
            .unwrap()
            .digest,
        None,
        "absent at freeze"
    );
    store.write(&pins).unwrap();
    (tasks, contract)
}

pub fn run_root(f: &Fixture) -> PathBuf {
    f.store.run_dir(&f.run)
}

pub fn held(result: &WorkflowV2Result) -> Vec<(String, Option<String>)> {
    result.data["check_source_held"]
        .as_array()
        .unwrap_or_else(|| panic!("nothing held: {result:#?}"))
        .iter()
        .map(|entry| {
            (
                entry["path"].as_str().unwrap().to_string(),
                entry["item"].as_str().map(str::to_string),
            )
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}
