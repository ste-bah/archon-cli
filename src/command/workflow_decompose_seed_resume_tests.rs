use super::*;
use crate::command::workflow_decompose_seed::SubjectSeed;
use serde_json::json;

fn at(value: &str) -> chrono::DateTime<chrono::FixedOffset> {
    chrono::DateTime::parse_from_rfc3339(value).unwrap()
}

fn entry(id: &str, command: &str) -> serde_json::Value {
    json!({
        "id": id,
        "criterion": format!("criterion for {id}"),
        "check": {"kind":"command", "command":command, "cwd":"project_root"},
        "covers": [],
        "gap_permitted": false,
        "judgment": {"verdict":"accepted", "counterexample":"false case", "reason":"checked"}
    })
}

#[test]
fn live_shaped_candidate_filters_only_five_later_republished_replies() {
    // A real freeze candidate has complete authored entry records under
    // entries/supplementary; each committed receipt is associated with its
    // candidate by call id and committed_at.
    let repaired = [
        "AC-DL-005",
        "SUP-REQ-BT-003",
        "SUP-REQ-DL-071",
        "SUP-REQ-DL-095",
        "SUP-REQ-DL-110",
    ];
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("decomposition/reverify")).unwrap();
    std::fs::write(
        temp.path().join("events.jsonl"),
        r#"{"ts":"2026-10-22T01:39:00Z","kind":"decomposition_reverified","detail":{"outcome":"needs_remediation","report_id":"reverify-7126-report"}}"#,
    )
    .unwrap();
    std::fs::write(
        temp.path()
            .join("decomposition/reverify/remediation-reverify-7126-report.json"),
        serde_json::to_vec(&json!({ "report_id":"reverify-7126-report", "subjects":repaired }))
            .unwrap(),
    )
    .unwrap();
    let lineage = super::reverify_report_publications(temp.path()).unwrap();
    let lineage_ids: std::collections::BTreeSet<_> =
        lineage.iter().map(|(id, _)| id.as_str()).collect();
    let mut kept = 0;
    let mut dropped = 0;
    for index in 0..80 {
        let id = repaired
            .get(index)
            .map(|id| (*id).to_string())
            .unwrap_or_else(|| format!("SUP-REQ-FIXTURE-{index:03}"));
        let prior = entry(&id, "old check");
        let current = entry(
            &id,
            if index < repaired.len() {
                "repaired check"
            } else {
                "old check"
            },
        );
        let reply_at = at("2026-10-21T01:00:00Z");
        let versions = lineage_ids
            .contains(id.as_str())
            .then(|| vec![(at("2026-10-22T01:39:00Z"), current.clone())]);
        if super::reply_was_published_after(versions.as_ref(), reply_at, &current) {
            dropped += 1;
        } else {
            kept += 1;
        }
        if lineage_ids.contains(id.as_str()) {
            assert_ne!(prior["check"], current["check"]);
            assert_eq!(current["check"]["command"], "repaired check");
        }
    }
    assert_eq!(dropped, 5);
    assert_eq!(kept, 75);
}

#[test]
fn a_reply_newer_than_the_publication_remains_an_overlay() {
    let current = entry("AC-NEWER", "published check");
    let versions = vec![(at("2026-10-22T01:39:00Z"), current.clone())];
    assert!(!super::reply_was_published_after(
        Some(&versions),
        at("2026-10-22T01:40:00Z"),
        &current,
    ));
}

#[test]
fn later_acceptance_freeze_keeps_all_seven_repaired_bytes() {
    let repairs: Vec<_> = (0..7)
        .map(|index| entry(&format!("SUP-REQ-REPAIR-{index:03}"), "per-check repaired"))
        .collect();
    let subjects: Vec<_> = repairs
        .iter()
        .map(|entry| entry["id"].as_str().unwrap())
        .collect();
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("decomposition/reverify")).unwrap();
    std::fs::write(
        temp.path().join("events.jsonl"),
        r#"{"ts":"2026-10-22T01:39:00Z","kind":"decomposition_reverified","detail":{"outcome":"needs_remediation","report_id":"reverify-7126-report"}}"#,
    )
    .unwrap();
    std::fs::write(
        temp.path()
            .join("decomposition/reverify/remediation-reverify-7126-report.json"),
        serde_json::to_vec(&json!({ "report_id":"reverify-7126-report", "subjects":subjects }))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        super::reverify_report_publications(temp.path())
            .unwrap()
            .len(),
        7
    );
    let later_freeze_candidate = json!({ "entries": repairs });
    assert_eq!(
        later_freeze_candidate["entries"].as_array().unwrap().len(),
        7
    );
    for (index, candidate) in later_freeze_candidate["entries"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        assert_eq!(candidate["check"]["command"], "per-check repaired");
        assert_eq!(candidate["id"], format!("SUP-REQ-REPAIR-{index:03}"));
    }
}

#[test]
fn acceptance_seed_refreshes_old_carried_entries_but_keeps_reauthored_reply_overlay() {
    let temp = tempfile::tempdir().unwrap();
    let current = json!({
        "schema_version": 1,
        "prd": {"path": "PRD.md", "digest": "prd"},
        "gap_policy": {"permitted_acceptance_ids": [], "forbidden_phrases": [], "required_fields": []},
        "acceptance": [{
            "id": "AC-1", "criterion": "criterion",
            "check": {"kind": "command", "command": "grep repaired out.txt", "cwd": "project_root"},
            "gap_permitted": false,
            "judgment": {"verdict": "accepted", "counterexample": "old counterexample", "reason": "repaired judgment", "host_call_id": "repair-1"}
        }],
        "supplementary": []
    });
    let bytes = serde_json::to_vec(&current).unwrap();
    std::fs::write(
        temp.path()
            .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE),
        &bytes,
    )
    .unwrap();
    let prior = json!({
        "id": "AC-1", "criterion": "criterion",
        "check": {"kind": "command", "command": "grep old out.txt", "cwd": "project_root"},
        "gap_permitted": false,
        "judgment": {"verdict": "accepted", "counterexample": "old", "reason": "old", "host_call_id": "old-1"}
    });
    let mut seed = PhaseSeed {
        schema_version: 1,
        transition_index: 0,
        runtime: archon_workflow::FixedRunIdentityV1 {
            template_version: "t".into(), starting_binary_revision: "b".into(),
            script_digest: "s".into(), catalog_digest: "c".into(),
            project_root_identity: "p".into(), prd_identity: "prd".into(), task_root_identity: "tasks".into(),
        },
        derived_at: "now".into(), author_ordinals: BTreeMap::new(), pause_ordinals: BTreeMap::new(),
        subjects: BTreeMap::from([("acceptance".into(), SubjectSeed::Entries {
            gates: Vec::new(), candidate: None,
            replies: vec![crate::command::workflow_decompose_seed::ReplySeed {
                call_id: "acceptance-author-AC-1-2".into(), id: "AC-1".into(),
                text: json!({"id":"AC-1", "check":{"kind":"command", "command":"grep authored out.txt"}}).to_string(),
                criterion: Some("criterion".into()),
            }],
            unreadable_replies: BTreeMap::new(), carried_entries: vec![prior], refuted_ids: Vec::new(),
            invalid: BTreeMap::new(), carried: 1,
        })]),
        acceptance_refreshed_from_contract_digest: None,
        resume_frozen_chain: None, pause_generation_floor: 0, history_attempts: BTreeMap::new(), event_id: None,
    };

    let digest = refresh_acceptance_carried_entries(temp.path(), temp.path(), &mut seed, &[])
        .unwrap()
        .unwrap();
    let Some(SubjectSeed::Entries {
        carried_entries,
        replies,
        ..
    }) = seed.subjects.get("acceptance")
    else {
        panic!("acceptance entries")
    };
    assert_eq!(carried_entries, &vec![current["acceptance"][0].clone()]);
    assert_eq!(carried_entries[0]["judgment"]["host_call_id"], "repair-1");
    assert!(
        replies[0].text.contains("grep authored out.txt"),
        "reply remains for the script's overlay"
    );
    assert_eq!(
        seed.acceptance_refreshed_from_contract_digest.as_deref(),
        Some(digest.as_str())
    );
}
