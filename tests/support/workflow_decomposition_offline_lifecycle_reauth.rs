use std::fs;
use std::path::Path;

pub fn assert_refuted_repair_judging(requests: &[super::server::RequestRecord]) {
    let repair_refutations = requests
        .iter()
        .enumerate()
        .filter(|(_, request)| {
            request.role == "judge"
                && request.key == "judge:adversarial:AC-X-001"
                && request.answer.contains("\"verdict\":\"refuted\"")
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    assert_eq!(
        repair_refutations.len(),
        3,
        "each launch repair re-judgment must refute AC-X-001: {requests:?}"
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.role == "reauthor")
            .count(),
        3,
        "the repair author must be asked once per refused judge result: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .skip(repair_refutations[2] + 1)
            .all(|request| request.role != "judge"),
        "the later byte-identical acceptance entry must reuse its prior accepted verdict: {requests:?}"
    );
}

pub fn assert_reauthored(root: &Path, log: &str) {
    assert!(
        log.contains("acceptance=reauthored published_ids=AC-X-001 pending_ids= "),
        "{log}"
    );
    assert!(!log.contains("acceptance=reauthor_failed"), "{log}");
    let contract_bytes = fs::read(root.join("tasks/PRD-X/acceptance-contract.json")).unwrap();
    let contract: serde_json::Value = serde_json::from_slice(&contract_bytes).unwrap();
    assert_eq!(
        contract["acceptance"][0]["check"]["command"],
        "python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))[\"ready\"] is True' .archon/proof/outcome.json",
        "separately read published contract must contain the re-authored check"
    );
    let digest = archon_workflow::task_set_contract::content_digest(&contract_bytes);
    let skeleton_lock: archon_workflow::task_skeleton::TaskSkeletonLock =
        serde_json::from_slice(&fs::read(root.join("tasks/PRD-X/task-skeleton.lock")).unwrap())
            .unwrap();
    assert_eq!(skeleton_lock.acceptance_digest, digest);
}
