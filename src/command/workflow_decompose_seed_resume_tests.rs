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
        "judgment": {"verdict":"accepted", "counterexample":"false case", "reason":"checked", "host_call_id":"fixture"}
    })
}

fn authored_digest(entry: &serde_json::Value) -> String {
    archon_workflow::task_set_contract::content_digest(
        &serde_json::to_vec(&json!({
            "id": entry["id"], "criterion": entry["criterion"], "check": entry["check"],
            "covers": entry["covers"], "gap_permitted": entry["gap_permitted"],
        }))
        .unwrap(),
    )
}

#[test]
fn live_shaped_candidate_filters_only_five_later_republished_replies() {
    // A real freeze candidate has complete authored entry records under
    // entries/supplementary; each committed receipt is associated with its
    // candidate by call id and committed_at.
    let repaired = ["CHK-002", "CHK-009", "CHK-010", "CHK-011", "CHK-012"];
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
    let resolved = repaired
        .iter()
        .map(|id| {
            (
                (*id).to_string(),
                json!(authored_digest(&entry(id, "repaired check"))),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    std::fs::write(temp.path().join("decomposition/reverify/resolved.jsonl"), serde_json::to_vec(&json!({"report_id":"reverify-7126-report","subjects":resolved,"published_at":"2026-10-22T01:39:00Z"})).unwrap()).unwrap();
    let lineage = super::reverify_report_publications(temp.path()).unwrap();
    let lineage_ids: std::collections::BTreeSet<_> =
        lineage.iter().map(|(id, _, _)| id.as_str()).collect();
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
        let versions = lineage
            .iter()
            .find(|(published_id, _, _)| published_id == &id)
            .map(|(_, published_at, digest)| vec![(*published_at, digest.clone())]);
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
    let versions = vec![(at("2026-10-22T01:39:00Z"), authored_digest(&current))];
    assert!(!super::reply_was_published_after(
        Some(&versions),
        at("2026-10-22T01:40:00Z"),
        &current,
    ));
}

#[test]
fn per_check_publication_time_drops_only_replies_started_before_republish() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("decomposition/reverify")).unwrap();
    std::fs::write(
        temp.path().join("events.jsonl"),
        r#"{"ts":"2026-10-22T01:00:00Z","kind":"decomposition_reverified","detail":{"outcome":"needs_remediation","report_id":"reverify-report"}}"#,
    ).unwrap();
    let published_digest = authored_digest(&entry("AC-X-001", "repaired"));
    std::fs::write(
        temp.path().join("decomposition/reverify/resolved.jsonl"),
        serde_json::to_vec(&json!({"report_id":"reverify-report","subjects":{"AC-X-001":published_digest},"published_at":"2026-10-22T03:00:00Z"})).unwrap(),
    ).unwrap();

    let current = entry("AC-X-001", "repaired");
    let publications = super::reverify_report_publications(temp.path()).unwrap();
    assert_eq!(publications.len(), 1);
    let published_at = publications[0].1;
    let published_digest = publications[0].2.clone();
    assert!(super::reply_was_published_after(
        Some(&vec![(published_at, published_digest.clone())]),
        at("2026-10-22T02:00:00Z"),
        &current,
    ));
    assert!(!super::reply_was_published_after(
        Some(&vec![(published_at, published_digest)]),
        at("2026-10-22T04:00:00Z"),
        &current,
    ));
}

#[test]
fn later_acceptance_freeze_keeps_all_seven_repaired_bytes() {
    let repairs: Vec<_> = (0..7)
        .map(|index| entry(&format!("SUP-REQ-REPAIR-{index:03}"), "per-check repaired"))
        .collect();
    let temp = tempfile::tempdir().unwrap();
    let reverify = temp.path().join("decomposition/reverify");
    std::fs::create_dir_all(&reverify).unwrap();
    let published_subjects = repairs
        .iter()
        .map(|entry| {
            (
                entry["id"].as_str().unwrap().to_string(),
                json!(authored_digest(entry)),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    std::fs::write(reverify.join("resolved.jsonl"), serde_json::to_vec(&json!({"report_id":"reverify-7126-report","subjects":published_subjects,"published_at":"2026-10-22T03:00:00Z"})).unwrap()).unwrap();
    let contract = json!({"schema_version":1,"prd":{"path":"PRD.md","digest":"p"},"gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},"acceptance":repairs,"supplementary":[]});
    std::fs::write(
        temp.path()
            .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE),
        serde_json::to_vec(&contract).unwrap(),
    )
    .unwrap();
    let replies = repairs.iter().map(|item| crate::command::workflow_decompose_seed::ReplySeed {
        call_id: format!("author-{}", item["id"].as_str().unwrap()), id: item["id"].as_str().unwrap().into(),
        text: json!({"id":item["id"],"criterion":item["criterion"],"check":{"kind":"command","command":"older candidate"}}).to_string(),
        criterion: Some(item["criterion"].as_str().unwrap().into()),
    }).collect::<Vec<_>>();
    let records = replies
        .iter()
        .map(|reply| {
            let mut record = archon_workflow::WorkflowV2CallRecord::new(
                "run",
                archon_workflow::WorkflowV2HostCall {
                    id: reply.call_id.clone(),
                    method: archon_workflow::WorkflowV2HostMethod::Agent,
                    write_mode: None,
                    options: Default::default(),
                },
                1,
                "author".into(),
                archon_workflow::WorkflowV2Result::accepted("reply"),
                Vec::new(),
            );
            record.started_at = "2026-10-22T02:00:00Z".into();
            record
        })
        .collect::<Vec<_>>();
    let mut seed = PhaseSeed {
        schema_version: 1,
        transition_index: 0,
        runtime: archon_workflow::FixedRunIdentityV1 {
            template_version: "t".into(),
            starting_binary_revision: "b".into(),
            script_digest: "s".into(),
            catalog_digest: "c".into(),
            project_root_identity: "p".into(),
            prd_identity: "p".into(),
            task_root_identity: "t".into(),
        },
        derived_at: "now".into(),
        author_ordinals: BTreeMap::new(),
        pause_ordinals: BTreeMap::new(),
        subjects: BTreeMap::from([(
            "acceptance".into(),
            SubjectSeed::Entries {
                gates: Vec::new(),
                candidate: None,
                replies,
                unreadable_replies: BTreeMap::new(),
                carried_entries: (0..7)
                    .map(|i| entry(&format!("SUP-REQ-REPAIR-{i:03}"), "older candidate"))
                    .collect(),
                refuted_ids: Vec::new(),
                invalid: BTreeMap::new(),
                carried: 7,
            },
        )]),
        acceptance_refreshed_from_contract_digest: None,
        resume_frozen_chain: None,
        pause_generation_floor: 0,
        history_attempts: BTreeMap::new(),
        event_id: None,
    };
    refresh_acceptance_carried_entries(temp.path(), temp.path(), &mut seed, &records).unwrap();
    let Some(SubjectSeed::Entries {
        replies,
        carried_entries,
        ..
    }) = seed.subjects.get("acceptance")
    else {
        panic!("acceptance seed")
    };
    assert!(replies.is_empty());
    assert_eq!(carried_entries.len(), 7);
    for (index, candidate) in carried_entries.iter().enumerate() {
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

#[test]
fn consumed_publications_filter_old_replies_but_unpublished_pending_replies_remain() {
    let repaired_consumed = ["CHK-002", "CHK-009", "CHK-010", "CHK-011", "CHK-012"];
    let pending_latest = [
        "CHK-001", "CHK-003", "CHK-004", "CHK-005", "CHK-006", "CHK-007", "CHK-008",
    ];
    let ids = repaired_consumed
        .into_iter()
        .chain(pending_latest)
        .collect::<Vec<_>>();
    let temp = tempfile::tempdir().unwrap();
    let reverify = temp.path().join("decomposition/reverify");
    std::fs::create_dir_all(&reverify).unwrap();
    std::fs::write(temp.path().join("events.jsonl"), concat!(
        "{\"ts\":\"2026-10-22T01:39:00Z\",\"kind\":\"decomposition_reverified\",\"detail\":{\"outcome\":\"needs_remediation\",\"report_id\":\"reverify-7126-report\"}}\n",
        "{\"ts\":\"2026-10-22T01:42:00Z\",\"kind\":\"decomposition_reverified\",\"detail\":{\"outcome\":\"needs_remediation\",\"report_id\":\"reverify-7281-report\"}}\n"
    )).unwrap();
    std::fs::write(
        reverify.join("consumed.jsonl"),
        "{\"report_id\":\"reverify-7126-report\"}\n",
    )
    .unwrap();
    let published = repaired_consumed
        .iter()
        .map(|id| {
            (
                (*id).to_string(),
                json!(authored_digest(&entry(id, "repaired check"))),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    std::fs::write(reverify.join("resolved.jsonl"), serde_json::to_vec(&json!({"report_id":"reverify-7126-report","subjects":published,"published_at":"2026-10-22T01:39:00Z"})).unwrap()).unwrap();
    let contract = json!({
        "schema_version": 1, "prd":{"path":"PRD.md","digest":"prd"},
        "gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},
        "acceptance":ids.iter().map(|id| entry(id, "repaired check")).collect::<Vec<_>>(), "supplementary":[]
    });
    std::fs::write(
        temp.path()
            .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE),
        serde_json::to_vec(&contract).unwrap(),
    )
    .unwrap();
    let replies = ids.iter().map(|id| crate::command::workflow_decompose_seed::ReplySeed {
        call_id: format!("acceptance-author-{id}-1"), id: id.to_string(),
        text: json!({"id":id,"criterion":format!("criterion for {id}"),"check":{"kind":"command","command":"stale check"}}).to_string(),
        criterion: Some(format!("criterion for {id}")),
    }).collect::<Vec<_>>();
    let records = ids
        .iter()
        .map(|id| {
            archon_workflow::WorkflowV2CallRecord::new(
                "run",
                archon_workflow::WorkflowV2HostCall {
                    id: format!("acceptance-author-{id}-1"),
                    method: archon_workflow::WorkflowV2HostMethod::Agent,
                    write_mode: None,
                    options: Default::default(),
                },
                1,
                "author".into(),
                archon_workflow::WorkflowV2Result::accepted("reply"),
                Vec::new(),
            )
        })
        .collect::<Vec<_>>();
    // Call timestamps are seeded before both report events.
    let records = records
        .into_iter()
        .map(|mut record| {
            record.started_at = "2026-10-21T16:00:00Z".into();
            record
        })
        .collect::<Vec<_>>();
    let mut seed = PhaseSeed {
        schema_version: 1,
        transition_index: 0,
        runtime: archon_workflow::FixedRunIdentityV1 {
            template_version: "t".into(),
            starting_binary_revision: "b".into(),
            script_digest: "s".into(),
            catalog_digest: "c".into(),
            project_root_identity: "p".into(),
            prd_identity: "prd".into(),
            task_root_identity: "tasks".into(),
        },
        derived_at: "now".into(),
        author_ordinals: BTreeMap::new(),
        pause_ordinals: BTreeMap::new(),
        subjects: BTreeMap::from([(
            "acceptance".into(),
            SubjectSeed::Entries {
                gates: Vec::new(),
                candidate: None,
                replies,
                unreadable_replies: BTreeMap::new(),
                carried_entries: ids
                    .iter()
                    .map(|id| entry(id, "stale carried check"))
                    .collect(),
                refuted_ids: Vec::new(),
                invalid: BTreeMap::new(),
                carried: 12,
            },
        )]),
        acceptance_refreshed_from_contract_digest: None,
        resume_frozen_chain: None,
        pause_generation_floor: 0,
        history_attempts: BTreeMap::new(),
        event_id: None,
    };
    refresh_acceptance_carried_entries(temp.path(), temp.path(), &mut seed, &records).unwrap();
    let Some(SubjectSeed::Entries {
        replies,
        carried_entries,
        ..
    }) = seed.subjects.get("acceptance")
    else {
        panic!("acceptance entries")
    };
    assert_eq!(
        replies.len(),
        7,
        "unpublished latest-report subjects remain eligible overlays"
    );
    assert_eq!(carried_entries.len(), 12);
}

#[path = "workflow_decompose_seed_resume_regression_tests.rs"]
mod regression_tests;
