use super::*;
use std::fs;
use std::path::{Path, PathBuf};

fn same_canonical_path(left: &Path, right: &Path) -> std::io::Result<bool> {
    Ok(left.canonicalize()? == right.canonicalize()?)
}

fn refresh_publication_receipts(records: &[PathBuf], changed: &[PathBuf]) {
    for path in records {
        let mut record: serde_json::Value =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        let Some(entries) =
            record["result"]["data"]["publicationReceipt"]["entries"].as_array_mut()
        else {
            continue;
        };
        let mut updated = false;
        for entry in entries {
            let Some(destination) = entry["destination_path"].as_str() else {
                continue;
            };
            let destination = Path::new(destination);
            if changed
                .iter()
                .any(|path| same_canonical_path(path, destination).unwrap())
            {
                let bytes = fs::read(destination).unwrap();
                entry["byte_len"] = bytes.len().into();
                entry["blake3"] = archon_workflow::task_set_contract::content_digest(&bytes).into();
                updated = true;
            }
        }
        if updated {
            fs::write(path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        }
    }
}

#[test]
fn launch_repair_acceptance_and_refutation_paths() {
    run_launch_repair_lifecycle(server::Variant::Golden);
    run_launch_repair_lifecycle(server::Variant::RefuteFirstJudge);
}

fn run_launch_repair_lifecycle(variant: server::Variant) {
    use server::{Plan, Variant};

    progress("launch-repair", "starting real CLI and local provider");
    let project = project::project_with(Plan {
        variant: Variant::Golden,
        block: None,
    });
    let root = project.path();
    let canonical_root = root.canonicalize().unwrap();
    let output = run(root, &decompose_args());
    assert!(
        output.status.success(),
        "exit status: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let first_run = one_run(root);

    // EXPECTED: the contract remains digest-consistent, but the current
    // live-root acceptance rule refuses its check and routes it to reauthor.
    // Model a freeze produced by an older build, whose recorded logic version
    // predates the current launch repair rules.
    let result_records = fs::read_dir(first_run.join("v2/results"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    let freeze_record = result_records
        .iter()
        .find(|path| {
            fs::read(path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .is_some_and(|record| {
                    record["call"]["options"]["host_command"]["commandId"] == "freeze-acceptance"
                })
        })
        .expect("first CLI launch persisted its freeze-acceptance record");
    let mut record: serde_json::Value =
        serde_json::from_slice(&fs::read(freeze_record).unwrap()).unwrap();
    let version = record["result"]["data"]["logicVersion"]
        .as_u64()
        .expect("freeze record carries a logic version");
    record["result"]["data"]["logicVersion"] = version.saturating_sub(1).into();

    let tasks = canonical_root.join("tasks/PRD-X");
    let pin_path = fs::read_dir(canonical_root.join(".archon/task-set-pins"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            fs::read(path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .is_some_and(|pin| {
                    pin["task_root"]
                        .as_str()
                        .is_some_and(|task_root| task_root.ends_with("tasks/PRD-X"))
                })
        })
        .expect("the first CLI launch persisted its acceptance pin");
    let contract_path = tasks.join("acceptance-contract.json");
    let contract_lock_path = tasks.join("acceptance-contract.lock");
    let skeleton_path = tasks.join("task-skeleton.json");
    let skeleton_lock_path = tasks.join("task-skeleton.lock");
    let mut contract: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&fs::read(&contract_path).unwrap()).unwrap();
    let original_entry = contract
        .acceptance
        .first()
        .expect("the first freeze has AC-X-001")
        .clone();
    let absolute_root = canonical_root.to_string_lossy().into_owned();
    let criterion = contract
        .acceptance
        .first_mut()
        .expect("the authored contract has an acceptance criterion");
    let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } =
        &mut criterion.check
    else {
        panic!("the authored criterion has a command check");
    };
    *command = format!("{command} && test -d {absolute_root}");
    let contract_bytes = serde_json::to_vec_pretty(&contract).unwrap();
    fs::write(&contract_path, &contract_bytes).unwrap();
    let contract_digest = archon_workflow::task_set_contract::content_digest(&contract_bytes);

    let mut acceptance_lock: archon_workflow::task_set_contract::AcceptanceLock =
        serde_json::from_slice(&fs::read(&contract_lock_path).unwrap()).unwrap();
    acceptance_lock.digest = contract_digest.clone();
    fs::write(
        &contract_lock_path,
        serde_json::to_vec_pretty(&acceptance_lock).unwrap(),
    )
    .unwrap();

    let mut skeleton: archon_workflow::task_skeleton::TaskSkeleton =
        serde_json::from_slice(&fs::read(&skeleton_path).unwrap()).unwrap();
    skeleton.acceptance_digest = contract_digest.clone();
    let skeleton_bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    fs::write(&skeleton_path, &skeleton_bytes).unwrap();
    let skeleton_digest = archon_workflow::task_set_contract::content_digest(&skeleton_bytes);

    let mut skeleton_lock: archon_workflow::task_skeleton::TaskSkeletonLock =
        serde_json::from_slice(&fs::read(&skeleton_lock_path).unwrap()).unwrap();
    skeleton_lock.digest = skeleton_digest.clone();
    skeleton_lock.acceptance_digest = contract_digest.clone();
    fs::write(
        &skeleton_lock_path,
        serde_json::to_vec_pretty(&skeleton_lock).unwrap(),
    )
    .unwrap();

    let mut pin: archon_workflow::task_set_contract::AcceptancePin =
        serde_json::from_slice(&fs::read(&pin_path).unwrap()).unwrap();
    pin.acceptance_digest = contract_digest.clone();
    pin.skeleton_digest = Some(skeleton_digest.clone());
    fs::write(&pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
    record["result"]["data"]["logicVersion"] = version.saturating_sub(1).into();
    fs::write(freeze_record, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    refresh_publication_receipts(
        &result_records,
        &[
            contract_path.clone(),
            contract_lock_path.clone(),
            skeleton_path.clone(),
            skeleton_lock_path.clone(),
            pin_path.clone(),
        ],
    );

    // Separately read the source of truth after rewriting every binding.
    let contract_bytes_on_disk = fs::read(&contract_path).unwrap();
    let check: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&contract_bytes_on_disk).unwrap();
    assert_eq!(
        contract_bytes_on_disk,
        serde_json::to_vec_pretty(&check).unwrap()
    );
    assert!(match &check.acceptance[0].check {
        archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } => {
            command.contains(&absolute_root)
        }
        archon_workflow::task_set_contract::AcceptanceCheck::Floor { .. } => false,
    });
    let acceptance_lock_bytes = fs::read(&contract_lock_path).unwrap();
    let acceptance_lock: archon_workflow::task_set_contract::AcceptanceLock =
        serde_json::from_slice(&acceptance_lock_bytes).unwrap();
    assert_eq!(
        acceptance_lock_bytes,
        serde_json::to_vec_pretty(&acceptance_lock).unwrap()
    );
    assert_eq!(acceptance_lock.digest, contract_digest);
    let skeleton_bytes_on_disk = fs::read(&skeleton_path).unwrap();
    let skeleton: archon_workflow::task_skeleton::TaskSkeleton =
        serde_json::from_slice(&skeleton_bytes_on_disk).unwrap();
    assert_eq!(
        skeleton_bytes_on_disk,
        serde_json::to_vec_pretty(&skeleton).unwrap()
    );
    assert_eq!(skeleton.acceptance_digest, contract_digest);
    let skeleton_lock_bytes = fs::read(&skeleton_lock_path).unwrap();
    let skeleton_lock: archon_workflow::task_skeleton::TaskSkeletonLock =
        serde_json::from_slice(&skeleton_lock_bytes).unwrap();
    assert_eq!(
        skeleton_lock_bytes,
        serde_json::to_vec_pretty(&skeleton_lock).unwrap()
    );
    assert_eq!(skeleton_lock.digest, skeleton_digest);
    assert_eq!(skeleton_lock.acceptance_digest, contract_digest);
    let pin_bytes = fs::read(&pin_path).unwrap();
    let pin: archon_workflow::task_set_contract::AcceptancePin =
        serde_json::from_slice(&pin_bytes).unwrap();
    assert_eq!(pin_bytes, serde_json::to_vec_pretty(&pin).unwrap());
    assert_eq!(pin.acceptance_digest, contract_digest);
    assert_eq!(
        pin.skeleton_digest.as_deref(),
        Some(skeleton_digest.as_str())
    );
    let changed = [
        contract_path,
        contract_lock_path,
        skeleton_path,
        skeleton_lock_path,
        pin_path,
    ];
    let mut receipt_coverage = std::collections::BTreeSet::new();
    for result_path in &result_records {
        let result: serde_json::Value =
            serde_json::from_slice(&fs::read(result_path).unwrap()).unwrap();
        let Some(entries) = result["result"]["data"]["publicationReceipt"]["entries"].as_array()
        else {
            continue;
        };
        for entry in entries {
            let Some(destination) = entry["destination_path"].as_str() else {
                continue;
            };
            let destination = Path::new(destination);
            if changed
                .iter()
                .any(|path| same_canonical_path(path, destination).unwrap())
            {
                let bytes = fs::read(destination).unwrap();
                assert_eq!(entry["byte_len"], bytes.len());
                assert_eq!(
                    entry["blake3"],
                    archon_workflow::task_set_contract::content_digest(&bytes)
                );
                receipt_coverage.insert(destination.canonicalize().unwrap());
            }
        }
    }
    assert_eq!(receipt_coverage, changed.into_iter().collect());

    // The next launch is the one exercising repair. Restart request ordinals
    // so the refuting variant covers all three repair judge calls.
    // EXPECTED: Golden publishes its accepted re-authored check. In the
    // refuting variant, the script author returns the byte-identical check
    // from the first freeze, so its accepted per-entry verdict is reused.
    // set_variant clears per-role counts so RefuteFirstJudge can distinguish
    // repair judgments from subsequent provider traffic.
    project.provider.set_variant(variant);
    let output = run(root, &decompose_args());
    assert!(
        output.status.success(),
        "second launch exit status: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let run_dir = fs::read_dir(root.join(".archon/workflows"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.join("state.json").is_file()
                && path.canonicalize().unwrap() != first_run.canonicalize().unwrap()
        })
        .expect("second decompose launch result store");
    let log = fs::read_to_string(root.join("tasks/PRD-X/.decompose.log")).unwrap();
    if variant == Variant::Golden {
        reauth_lifecycle::assert_reauthored(root, &log);
        return;
    }
    assert!(log.contains("acceptance=reauthor_refuted_routed"), "{log}");
    assert!(!log.contains("acceptance=reauthor_failed"), "{log}");
    let results = fs::read_dir(run_dir.join("v2/results"))
        .unwrap()
        .map(|entry| fs::read(entry.unwrap().path()).unwrap())
        .map(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).unwrap())
        .collect::<Vec<_>>();
    assert!(
        results.iter().any(|record| {
            record["call"]["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("acceptance-author-"))
                && record["status"] == "accepted"
        }),
        "the script's acceptance author call must be persisted: {results:?}"
    );
    assert!(
        results.iter().any(|record| {
            record["call"]["options"]["host_command"]["commandId"] == "freeze-acceptance"
                && record["status"] == "accepted"
        }),
        "the authored output must reach the acceptance freeze: {results:?}"
    );
    let published_contract_bytes = fs::read(root.join("tasks/PRD-X/acceptance-contract.json"))
        .expect("the acceptance freeze must publish its contract");
    let published_contract: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&published_contract_bytes)
            .expect("the acceptance freeze must publish a readable contract");
    let frozen_entry = published_contract
        .acceptance
        .iter()
        .find(|entry| entry.id == "AC-X-001")
        .expect("the published contract must contain AC-X-001");
    assert!(
        frozen_entry.check == original_entry.check,
        "the separately read freeze must contain the same check as the first accepted judgment"
    );
    assert_eq!(frozen_entry.criterion, original_entry.criterion);
    assert_eq!(frozen_entry.covers, original_entry.covers);
    assert_eq!(frozen_entry.gap_permitted, original_entry.gap_permitted);
    assert_eq!(
        frozen_entry.judgment, original_entry.judgment,
        "the freeze must reuse the first accepted judgment for the byte-identical entry"
    );
    assert_eq!(
        frozen_entry.judgment.verdict,
        archon_workflow::task_set_contract::JudgeDecision::Accepted,
        "only an accepted verdict may be reused for the frozen entry"
    );

    let requests = project.provider.requests.snapshot();
    reauth_lifecycle::assert_refuted_repair_judging(&requests);
    let acceptance = requests
        .iter()
        .find(|request| request.role == "acceptance")
        .expect("wire-level offline provider served no acceptance-author request");
    assert_eq!(acceptance.method, "POST");
    assert_eq!(acceptance.path, "/v1/messages");
    assert!(acceptance.auth_present);
    assert!(acceptance.known_path);
    assert!(acceptance.key.starts_with("acceptance:"));
    assert!(!acceptance.prompt.is_empty());
    assert!(!acceptance.answer.is_empty());
    assert_eq!(
        project.provider.requests.per_call().get(&acceptance.key),
        Some(&1)
    );
    progress("judge-refutation", "passed");
}
