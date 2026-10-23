//! A seeded reply overlays its published entry only while that entry is
//! unchanged since the reply was authored; every repair path that publishes
//! after the reply keeps its repair.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::command::workflow_decompose_seed::derive::tests::{entry, gate, reply};
use crate::command::workflow_decompose_seed::{PhaseSeed, SubjectSeed};

const ID: &str = "AC-1";

pub(crate) fn with_command(id: &str, command: &str) -> Value {
    let mut item = entry(id);
    item["check"]["command"] = json!(command);
    item
}

/// Publish `entries` as the contract under `root`; its digest.
pub(crate) fn publish(root: &std::path::Path, entries: &[Value]) -> String {
    let bytes = serde_json::to_vec(&json!({
        "schema_version": 1, "prd": {"path": "PRD.md", "digest": "p"},
        "gap_policy": {"permitted_acceptance_ids": [], "forbidden_phrases": [], "required_fields": []},
        "acceptance": entries, "supplementary": [],
    }))
    .unwrap();
    std::fs::write(
        root.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE),
        &bytes,
    )
    .unwrap();
    archon_workflow::task_set_contract::content_digest(&bytes)
}

/// The lock every publish path writes beside the contract it published.
pub(crate) fn lock(root: &std::path::Path, digest: &str, at: &str) {
    std::fs::write(
        root.join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE),
        serde_json::to_vec(&json!({
            "algorithm": "blake3", "digest": digest,
            "gate": {"mode": "enforce", "finding_count": 0, "findings_digest": "f",
                     "binary_commit": "b", "evaluated_at": format!("2026-10-06T{at}+00:00")},
        }))
        .unwrap(),
    )
    .unwrap();
}

/// An in-run freeze of `entries` that committed a contract with `digest`.
pub(crate) fn freeze(
    at: &str,
    entries: &[Value],
    digest: &str,
) -> archon_workflow::WorkflowV2CallRecord {
    let stdin = json!({"entries": entries, "supplementary": []}).to_string();
    let mut record = gate("freeze-acceptance", at, &stdin, Vec::new(), true);
    record.result.data["publicationReceipt"] = json!({
        "schema_version": 1, "call_id": record.call.id, "command_id": "freeze-acceptance",
        "committed_at": format!("2026-10-06T{at}+00:00"),
        "entries": [{"relative_path": "acceptance-contract.json",
                     "destination_path": "/tasks/acceptance-contract.json",
                     "byte_len": 1, "blake3": digest}],
    });
    record
}

pub(crate) fn seed(replies: Vec<(&str, &str)>, carried: Vec<Value>) -> PhaseSeed {
    let replies = replies
        .into_iter()
        .map(
            |(call_id, command)| crate::command::workflow_decompose_seed::ReplySeed {
                call_id: call_id.into(),
                id: ID.into(),
                text: with_command(ID, command).to_string(),
                criterion: Some(format!("criterion of {ID}")),
            },
        )
        .collect();
    PhaseSeed {
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
                carried: carried.len(),
                carried_entries: carried,
                refuted_ids: Vec::new(),
                invalid: BTreeMap::new(),
            },
        )]),
        acceptance_refreshed_from_contract_digest: None,
        resume_frozen_chain: None,
        pause_generation_floor: 0,
        history_attempts: BTreeMap::new(),
        event_id: None,
    }
}

/// The replies and carried commands the script would receive.
fn refreshed(
    root: &std::path::Path,
    mut seed: PhaseSeed,
    records: &[archon_workflow::WorkflowV2CallRecord],
) -> (Vec<String>, Vec<String>) {
    super::super::refresh_acceptance_carried_entries(root, root, &mut seed, records).unwrap();
    let SubjectSeed::Entries {
        replies,
        carried_entries,
        ..
    } = &seed.subjects["acceptance"]
    else {
        panic!("acceptance seed")
    };
    (
        replies.iter().map(|reply| reply.call_id.clone()).collect(),
        carried_entries
            .iter()
            .map(|entry| entry["check"]["command"].as_str().unwrap().to_string())
            .collect(),
    )
}

fn authored_reply(at: &str) -> archon_workflow::WorkflowV2CallRecord {
    reply(
        &format!("acceptance-author-{ID}-2"),
        at,
        &with_command(ID, "reply bytes").to_string(),
    )
}

#[test]
fn a_consumed_heal_with_no_resolved_row_keeps_its_repair() {
    let temp = tempfile::tempdir().unwrap();
    let first = [with_command(ID, "first")];
    let first_digest = publish(temp.path(), &first);
    // The run froze, the author replied, then a consumed-ledger heal
    // republished the entry: it rewrote the lock but wrote no resolved row.
    let healed = publish(temp.path(), &[with_command(ID, "healed")]);
    lock(temp.path(), &healed, "03:00:00");
    let records = [
        freeze("01:00:00", &first, &first_digest),
        authored_reply("02:00:00"),
    ];
    let seeded = seed(vec![(&records[1].call.id, "reply bytes")], first.to_vec());
    let (replies, carried) = refreshed(temp.path(), seeded, &records);
    assert!(
        replies.is_empty(),
        "the reply predates the heal: {replies:?}"
    );
    assert_eq!(carried, ["healed"]);
}

#[test]
fn an_operator_or_launch_republish_with_no_run_record_keeps_its_repair() {
    let temp = tempfile::tempdir().unwrap();
    // Frozen before this run; republished from outside the script later.
    let repaired = publish(temp.path(), &[with_command(ID, "repaired outside")]);
    lock(temp.path(), &repaired, "03:00:00");
    let records = [authored_reply("02:00:00")];
    let seeded = seed(
        vec![(&records[0].call.id, "reply bytes")],
        vec![with_command(ID, "older")],
    );
    let (replies, carried) = refreshed(temp.path(), seeded, &records);
    assert!(replies.is_empty(), "{replies:?}");
    assert_eq!(carried, ["repaired outside"]);
}

#[test]
fn a_previous_build_resolved_row_without_a_time_keeps_the_repair() {
    let temp = tempfile::tempdir().unwrap();
    let healed = with_command(ID, "healed");
    let digest = publish(temp.path(), std::slice::from_ref(&healed));
    lock(temp.path(), &digest, "03:00:00");
    let whole =
        archon_workflow::task_set_contract::content_digest(&serde_json::to_vec(&healed).unwrap());
    let journal = temp.path().join("decomposition/reverify");
    std::fs::create_dir_all(&journal).unwrap();
    std::fs::write(
        journal.join("resolved.jsonl"),
        format!("{}\n", json!({"report_id": "r", "subjects": {ID: whole}})),
    )
    .unwrap();
    let records = [authored_reply("02:00:00")];
    let seeded = seed(
        vec![(&records[0].call.id, "reply bytes")],
        vec![healed.clone()],
    );
    let (replies, carried) = refreshed(temp.path(), seeded, &records);
    assert!(replies.is_empty(), "{replies:?}");
    assert_eq!(carried, ["healed"]);
}

#[test]
fn a_logic_bump_that_drops_old_gates_does_not_revive_old_replies_over_a_repair() {
    let temp = tempfile::tempdir().unwrap();
    let replied = [with_command(ID, "reply bytes")];
    let replied_digest = archon_workflow::task_set_contract::content_digest(b"older contract");
    let mut old_gate = freeze("02:30:00", &replied, &replied_digest);
    old_gate.result.data["logicVersion"] = json!(1);
    let records = vec![authored_reply("02:00:00"), old_gate];
    let criteria = BTreeMap::from([(ID.to_string(), format!("criterion of {ID}"))]);
    let derived =
        crate::command::workflow_decompose_seed::derive::derive(&records, &[], &criteria).unwrap();
    let SubjectSeed::Entries { gates, replies, .. } = &derived.subjects["acceptance"] else {
        panic!("acceptance seed")
    };
    assert!(gates.is_empty(), "this build drops the stale gate");
    assert_eq!(replies.len(), 1, "so the old reply is an overlay candidate");
    let healed = publish(temp.path(), &[with_command(ID, "healed")]);
    lock(temp.path(), &healed, "03:00:00");
    let mut seeded = seed(Vec::new(), replied.to_vec());
    if let SubjectSeed::Entries { replies: kept, .. } =
        seeded.subjects.get_mut("acceptance").unwrap()
    {
        kept.clone_from(replies);
    }
    let (replies, carried) = refreshed(temp.path(), seeded, &records);
    assert!(replies.is_empty(), "{replies:?}");
    assert_eq!(carried, ["healed"]);
}

#[test]
fn a_reply_newer_than_every_publication_of_its_entry_stays_an_overlay() {
    let temp = tempfile::tempdir().unwrap();
    let first = [with_command(ID, "first"), with_command("AC-2", "other")];
    let first_digest = publish(temp.path(), &first);
    // Another entry was republished after the reply; this one was not.
    let later = publish(
        temp.path(),
        &[with_command(ID, "first"), with_command("AC-2", "fixed")],
    );
    lock(temp.path(), &later, "03:00:00");
    let records = [
        freeze("01:00:00", &first, &first_digest),
        authored_reply("02:00:00"),
    ];
    let seeded = seed(vec![(&records[1].call.id, "reply bytes")], first.to_vec());
    let (replies, carried) = refreshed(temp.path(), seeded, &records);
    assert_eq!(replies, [records[1].call.id.clone()]);
    assert_eq!(carried, ["first", "fixed"]);
    // With no time for the reply, nothing shows the entry unchanged since.
    let seeded = seed(
        vec![("acceptance-author-AC-1-9", "reply bytes")],
        first.to_vec(),
    );
    assert!(refreshed(temp.path(), seeded, &records).0.is_empty());
}
