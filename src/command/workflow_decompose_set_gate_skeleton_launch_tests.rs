//! O3 on a launch whose skeleton the host verifies in place: the first
//! skeleton re-author re-authors only the bodies its change reaches, judged
//! against the frozen entries the launcher read (`frozenChain.tasks`, as the
//! host serialises the published skeleton) and the bodies already on disk.

use super::{UNCLAIMED, calls, driver_on, run, skeleton};

/// The launch-bound frozen chain: both bodies on disk; `tasks` (when given)
/// in the host's published form, which spells out every default field.
fn frozen(tasks: Option<serde_json::Value>) -> String {
    let mut chain = serde_json::json!({
        "acceptance": true, "skeleton": true,
        "subjects": [
            {"taskId": "TASK-X-010", "fileName": "TASK-X-010.md"},
            {"taskId": "TASK-X-020", "fileName": "TASK-X-020.md"},
        ],
        "bodies": ["TASK-X-010.md", "TASK-X-020.md"],
    });
    if let Some(tasks) = tasks {
        chain["tasks"] = tasks;
    }
    chain.to_string()
}

fn published_tasks() -> serde_json::Value {
    serde_json::json!([
        {
            "task_id": "TASK-X-010", "file_name": "TASK-X-010.md", "depends_on": [], "blocks": [],
            "implements": ["AC-X-001"],
            "deliverable_contracts": [{
                "kind": "report", "artifact_path": "out/a.json", "min_instances": 1,
                "required_universe": false, "registry_minimum_count": 0, "series_overlap_min_rows": 0
            }]
        },
        {
            "task_id": "TASK-X-020", "file_name": "TASK-X-020.md",
            "depends_on": [{"task_id": "TASK-X-010", "consumes": [{"artifact_path": "out/a.json"}], "ordering_only": false}],
            "blocks": [], "implements": ["AC-X-001"], "deliverable_contracts": []
        }
    ])
}

/// The re-authored skeleton as its author writes it, default fields left
/// out: TASK-X-010 gains REQ-X-009, TASK-X-020 keeps its frozen entry, and
/// TASK-X-030 is new.
fn reauthored() -> String {
    skeleton(vec![
        serde_json::json!({
            "task_id": "TASK-X-010", "file_name": "TASK-X-010.md",
            "implements": ["AC-X-001", "REQ-X-009"],
            "deliverable_contracts": [{"kind": "report", "artifact_path": "out/a.json", "min_instances": 1}]
        }),
        serde_json::json!({
            "task_id": "TASK-X-020", "file_name": "TASK-X-020.md",
            "depends_on": [{"task_id": "TASK-X-010", "consumes": [{"artifact_path": "out/a.json"}]}],
            "implements": ["AC-X-001"]
        }),
        serde_json::json!({
            "task_id": "TASK-X-030", "file_name": "TASK-X-030.md", "depends_on": [], "blocks": [],
            "implements": ["REQ-X-009"], "deliverable_contracts": []
        }),
    ])
}

fn body_authors(out: &serde_json::Value, task: &str) -> usize {
    let prefix = format!("body-{task}-author-");
    (calls(out, "agentCalls").iter())
        .filter(|id| id.starts_with(&prefix))
        .count()
}

#[test]
fn a_launch_with_the_skeleton_verified_in_place_re_authors_only_the_bodies_its_change_reaches() {
    let script = driver_on(
        &[reauthored()],
        &[],
        "",
        UNCLAIMED,
        &frozen(Some(published_tasks())),
    );
    let out = run(&script);
    assert!(out.get("error").is_none(), "{out}");
    let host = calls(&out, "hostCalls");
    assert_eq!(host[0], "verify-frozen-acceptance", "{out}");
    assert_eq!(host[1], "verify-frozen-skeleton", "{out}");
    assert_eq!(
        (host.iter()).filter(|c| *c == "freeze-skeleton").count(),
        1,
        "one skeleton re-author: {out}"
    );
    let landed = calls(&out, "landed");
    let targets: Vec<&str> = (landed.iter())
        .map(|l| l.split('=').next().unwrap())
        .collect();
    assert_eq!(targets, ["TASK-X-010", "TASK-X-030"], "{out}");
    assert_eq!(body_authors(&out, "TASK-X-010"), 1, "{out}");
    assert_eq!(body_authors(&out, "TASK-X-030"), 1, "{out}");
    assert_eq!(
        body_authors(&out, "TASK-X-020"),
        0,
        "a body on disk whose frozen entry is unchanged keeps its bytes: {out}"
    );
    let reauthored = host.iter().rposition(|c| c == "freeze-skeleton").unwrap();
    assert!(
        host[reauthored..].iter().any(|c| c == "task-set-lint")
            && host[reauthored..].iter().any(|c| c == "requirements-trace"),
        "both set gates judge the whole set again: {out}"
    );
    // Replayed from the start, the run makes exactly the same calls.
    assert_eq!(run(&script)["agentCalls"], out["agentCalls"]);
}

/// A launch argument without the frozen entries (a run launched before
/// they were passed) cannot tell what changed: every body is re-authored.
#[test]
fn without_the_frozen_entries_a_verified_skeleton_re_author_re_authors_every_body() {
    let out = run(&driver_on(
        &[reauthored()],
        &[],
        "",
        UNCLAIMED,
        &frozen(None),
    ));
    assert!(out.get("error").is_none(), "{out}");
    for task in ["TASK-X-010", "TASK-X-020", "TASK-X-030"] {
        assert_eq!(body_authors(&out, task), 1, "{task}: {out}");
    }
}
