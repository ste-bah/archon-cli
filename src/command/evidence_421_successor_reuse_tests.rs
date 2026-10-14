use super::successor_reuse_tests::successor_fixture;

fn state(
    store: &archon_workflow::WorkflowV2ResultStore,
    receipt: &archon_workflow::PublicationReceiptV1,
    pin_path: &str,
) -> String {
    let pin = std::fs::read(pin_path).unwrap();
    let records = store.load_call_records().unwrap();
    let successor_publications = records
        .iter()
        .filter_map(|record| {
            let publication = record.result.data.get("publicationReceipt")?;
            Some(format!(
                "call={} command={} status={:?} invalidated_by={:?} publication={}",
                record.call.id,
                record.call.options.host_command.as_ref()?.command_id,
                record.status,
                record.invalidated_by,
                publication
            ))
        })
        .collect::<Vec<_>>();
    format!(
        "receipt_digest={} pin_disk_digest={} successor_publications={successor_publications:?}",
        receipt.entries[0].blake3,
        archon_workflow::task_set_contract::content_digest(&pin),
    )
}

fn observed_after(
    store: &archon_workflow::WorkflowV2ResultStore,
    receipt: &archon_workflow::PublicationReceiptV1,
    pin_path: &str,
    reused: bool,
) -> String {
    let pin_path = std::path::Path::new(pin_path);
    let records = store.load_call_records().unwrap();
    let successor_prior = records
        .iter()
        .find(|record| {
            record
                .call
                .options
                .host_command
                .as_ref()
                .is_some_and(|request| request.command_id == "freeze-skeleton")
        })
        .and_then(|record| record.result.data.get("publicationReceipt"))
        .and_then(|publication| publication.get("entries"))
        .and_then(serde_json::Value::as_array)
        .and_then(|entries| {
            entries.iter().find(|entry| {
                entry
                    .get("destination_path")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|destination| std::path::Path::new(destination) == pin_path)
            })
        })
        .and_then(|entry| entry.get("prior_blake3"))
        .and_then(serde_json::Value::as_str);
    let receipt_digest = receipt.entries[0].blake3.as_str();
    format!(
        "reuse_decision={} successor_prior_blake3={} successor_prior_equals_receipt_digest={}",
        if reused { "reuse" } else { "re-run" },
        successor_prior.unwrap_or("absent"),
        successor_prior == Some(receipt_digest)
    )
}

#[test]
fn evidence_421_a_ordered_freeze_skeleton_publication_reuses_acceptance_receipt() {
    let (_root, executor, original, receipt, pin_path) =
        successor_fixture("freeze-acceptance", "freeze-skeleton", true, false);
    let store = archon_workflow::WorkflowV2ResultStore::new(executor.run_root.join("v2"));
    // The fixture is rooted in a real temporary project and the executor's
    // receipt-chain predicate reads the persisted successor record.
    let before = state(&store, &receipt, &pin_path);
    let reused = executor
        .receipt_matches_live_with_successors(&original, Some(&receipt), false)
        .unwrap();
    eprintln!(
        "EVIDENCE #421 a BEFORE: {before} AFTER: {}",
        observed_after(&store, &receipt, &pin_path, reused)
    );
    assert!(reused);
}

#[test]
fn evidence_421_b_disk_edit_without_successor_publication_reruns_acceptance() {
    let (_root, executor, original, receipt, pin_path) =
        successor_fixture("freeze-acceptance", "freeze-skeleton", true, false);
    let store = archon_workflow::WorkflowV2ResultStore::new(executor.run_root.join("v2"));
    let successor = store
        .load_call_records()
        .unwrap()
        .into_iter()
        .find(|record| {
            record
                .call
                .options
                .host_command
                .as_ref()
                .is_some_and(|request| request.command_id == "freeze-skeleton")
        })
        .unwrap();
    std::fs::remove_file(store.result_path(&successor.call.id)).unwrap();
    let before = state(&store, &receipt, &pin_path);
    let reused = executor
        .receipt_matches_live_with_successors(&original, Some(&receipt), false)
        .unwrap();
    eprintln!(
        "EVIDENCE #421 b BEFORE: {before} AFTER: {}",
        observed_after(&store, &receipt, &pin_path, reused)
    );
    assert!(!reused);
}

#[test]
fn evidence_421_c_invalidated_successor_publication_does_not_reuse_receipt() {
    let (_root, executor, original, receipt, pin_path) =
        successor_fixture("freeze-acceptance", "freeze-skeleton", true, false);
    let store = archon_workflow::WorkflowV2ResultStore::new(executor.run_root.join("v2"));
    let mut successor = store
        .load_call_records()
        .unwrap()
        .into_iter()
        .find(|record| {
            record
                .call
                .options
                .host_command
                .as_ref()
                .is_some_and(|request| request.command_id == "freeze-skeleton")
        })
        .unwrap();
    successor.invalidated_by = Some("later-recorded-call".into());
    store.save_call_record(&successor).unwrap();
    let before = state(&store, &receipt, &pin_path);
    let reused = executor
        .receipt_matches_live_with_successors(&original, Some(&receipt), false)
        .unwrap();
    eprintln!(
        "EVIDENCE #421 c BEFORE: {before} AFTER: {}",
        observed_after(&store, &receipt, &pin_path, reused)
    );
    assert!(!reused);
}

#[test]
fn evidence_421_d_successor_without_prior_digest_is_not_reused() {
    let (_root, executor, original, receipt, pin_path) =
        successor_fixture("freeze-acceptance", "freeze-skeleton", true, false);
    let store = archon_workflow::WorkflowV2ResultStore::new(executor.run_root.join("v2"));
    let mut successor = store
        .load_call_records()
        .unwrap()
        .into_iter()
        .find(|record| {
            record
                .call
                .options
                .host_command
                .as_ref()
                .is_some_and(|request| request.command_id == "freeze-skeleton")
        })
        .unwrap();
    successor.result.data["publicationReceipt"]["entries"][0]
        .as_object_mut()
        .unwrap()
        .remove("prior_blake3");
    store.save_call_record(&successor).unwrap();
    let before = state(&store, &receipt, &pin_path);
    let reused = executor
        .receipt_matches_live_with_successors(&original, Some(&receipt), false)
        .unwrap();
    eprintln!(
        "EVIDENCE #421 d BEFORE: {before} AFTER: {}",
        observed_after(&store, &receipt, &pin_path, reused)
    );
    assert!(!reused);
}

#[test]
fn evidence_421_e_successor_with_wrong_prior_digest_is_not_reused() {
    let (_root, executor, original, receipt, pin_path) =
        successor_fixture("freeze-acceptance", "freeze-skeleton", true, false);
    let store = archon_workflow::WorkflowV2ResultStore::new(executor.run_root.join("v2"));
    let mut successor = store
        .load_call_records()
        .unwrap()
        .into_iter()
        .find(|record| {
            record
                .call
                .options
                .host_command
                .as_ref()
                .is_some_and(|request| request.command_id == "freeze-skeleton")
        })
        .unwrap();
    successor.result.data["publicationReceipt"]["entries"][0]["prior_blake3"] =
        "broken-chain-digest".into();
    store.save_call_record(&successor).unwrap();
    let before = state(&store, &receipt, &pin_path);
    let reused = executor
        .receipt_matches_live_with_successors(&original, Some(&receipt), false)
        .unwrap();
    eprintln!(
        "EVIDENCE #421 e BEFORE: {before} AFTER: {}",
        observed_after(&store, &receipt, &pin_path, reused)
    );
    assert!(!reused);
}
