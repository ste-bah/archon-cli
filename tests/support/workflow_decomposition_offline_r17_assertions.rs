//! Durable observations shared by the R17 cells; no new product seams.
use crate::{deadline, project, server};
use project::TestProject;
use serde_json::Value;
use server::variants::r17::Cell;
use std::{collections::BTreeMap, fs, path::Path, process::Output};

pub fn fixture(cell: Cell) -> TestProject {
    project::project_with_files(
        server::Plan {
            variant: server::Variant::R17(cell),
            block: None,
        },
        &[(
            "prd.md",
            include_str!("../fixtures/offline_decomposition_r17/prd.md").into(),
        )],
    )
}

pub fn success(output: &Output) {
    assert!(
        output.status.success(),
        "exit={}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn completed(cell: Cell) -> TestProject {
    let p = fixture(cell);
    success(&deadline::run(p.path(), &project::decompose_args()));
    p
}

pub fn read(path: &Path) -> Value {
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

pub fn pin_path(root: &Path) -> std::path::PathBuf {
    let pins: Vec<_> = fs::read_dir(root.join(".archon/task-set-pins"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|ext| ext == "json"))
        .collect();
    assert_eq!(pins.len(), 1, "one current task-set pin: {pins:?}");
    pins.into_iter().next().unwrap()
}

pub fn events(p: &TestProject) -> Vec<Value> {
    fs::read_to_string(project::one_run(p.path()).join("events.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

pub fn records(p: &TestProject) -> Vec<Value> {
    let rows = events(p);
    let mut records: Vec<Value> = fs::read_dir(project::one_run(p.path()).join("v2/results"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .map(|p| read(&p))
        .collect();
    records.sort_by_key(|r| {
        rows.iter()
            .find(|e| e["detail"]["call_id"] == r["call"]["id"])
            .and_then(|e| e["seq"].as_u64())
            .unwrap_or(u64::MAX)
    });
    records
}

pub fn gates(p: &TestProject, command: &str) -> Vec<Value> {
    records(p)
        .into_iter()
        .filter(|r| {
            r["call"]["method"] == "hostCommand"
                && (r
                    .pointer("/call/options/host_command/commandId")
                    .and_then(Value::as_str)
                    == Some(command)
                    || r.pointer("/call/options/extra/commandId")
                        .and_then(Value::as_str)
                        == Some(command))
        })
        .collect()
}

pub fn findings(record: &Value) -> &[Value] {
    record
        .pointer("/result/data/gateEnvelope/policy_findings")
        .and_then(Value::as_array)
        .expect("host gate envelope carries policy_findings")
}

pub fn landed_candidates(p: &TestProject) -> Vec<Value> {
    records(p)
        .iter()
        .filter(|r| {
            r["call"]["method"] == "agent"
                && r["call"]["id"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("skeleton-author-")
        })
        .map(|r| serde_json::from_str(r["result"]["data"]["content"].as_str().unwrap()).unwrap())
        .collect()
}

/// Every author role has an exact count. Skeleton exchanges are two HTTP
/// requests per author call (landing plus receipt), not two author calls.
/// Pause fixtures explicitly account for their setup pause; no cell may
/// introduce any further pause, and all final states must be completed.
pub fn shared(p: &TestProject, skeleton_calls: usize, body_calls: [usize; 2], setup_pauses: usize) {
    shared_with_skeleton_exchanges(
        p,
        skeleton_calls,
        body_calls,
        setup_pauses,
        skeleton_calls * 2,
    );
}

pub fn shared_with_skeleton_exchanges(
    p: &TestProject,
    skeleton_calls: usize,
    body_calls: [usize; 2],
    setup_pauses: usize,
    skeleton_exchanges: usize,
) {
    let run = project::one_run(p.path());
    assert_eq!(read(&run.join("state.json"))["status"], "completed");
    let rows = events(p);
    let pauses: Vec<_> = rows
        .iter()
        .filter(|e| matches!(e["kind"].as_str(), Some("paused" | "pause" | "run_paused")))
        .collect();
    assert_eq!(pauses.len(), setup_pauses, "unexpected pause: {pauses:?}");
    // Runtime throws pause with durable evidence, even if a later resume completes.
    assert!(
        !run.join("v2/script-error-pause.json").exists(),
        "script throw recorded"
    );
    assert!(
        rows.iter()
            .all(|e| e["detail"]["event"] != "script_error_pause"
                && !matches!(
                    e["detail"]["cause"].as_str(),
                    Some("script_error" | "recurring_script_error")
                )),
        "script threw: {rows:?}"
    );
    project::assert_no_unknown_prompts(&p.provider);
    let counts = p.provider.counts.snapshot();
    for (role, count) in [
        ("acceptance", 4),
        ("skeleton", skeleton_exchanges),
        ("body", body_calls.iter().sum()),
    ] {
        assert_eq!(
            counts.get(role).copied().unwrap_or_default(),
            count,
            "exact {role} calls: {counts:?}"
        );
    }
    let per_call = p.provider.requests.per_call();
    for (task, count) in [("TASK-X-010", body_calls[0]), ("TASK-X-020", body_calls[1])] {
        assert_eq!(
            per_call
                .get(&format!("body:{task}"))
                .copied()
                .unwrap_or_default(),
            count
        );
    }
    let author_counts: BTreeMap<_, usize> = records(p)
        .iter()
        .filter(|r| r["call"]["method"] == "agent")
        .fold(BTreeMap::new(), |mut counts, r| {
            let id = r["call"]["id"].as_str().unwrap();
            let role = if id.starts_with("acceptance-author-") {
                "acceptance"
            } else if id.starts_with("skeleton-author-") {
                "skeleton"
            } else if id.starts_with("body-") && id.contains("-author-") {
                "body"
            } else {
                id
            };
            *counts.entry(role.to_owned()).or_default() += 1;
            counts
        });
    assert_eq!(
        author_counts,
        BTreeMap::from([
            ("acceptance".into(), 4),
            ("skeleton".into(), skeleton_calls),
            ("body".into(), body_calls.iter().sum())
        ])
    );
}

pub fn assert_clean_set_rounds(p: &TestProject, rounds: usize) {
    for command in ["task-set-lint", "requirements-trace"] {
        let gates = gates(p, command);
        assert_eq!(gates.len(), rounds, "{command} rounds");
        for gate in gates {
            assert!(findings(&gate).is_empty(), "{command}: {gate}");
        }
    }
}
