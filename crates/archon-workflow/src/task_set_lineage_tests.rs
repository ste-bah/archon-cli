use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::*;
use crate::task_set_contract::{FreezeGateMode, empty_gate_findings_digest};

fn entry(id: &str, command: &str, verdict: &str) -> Value {
    json!({
        "id": id,
        "criterion": format!("criterion of {id}"),
        "check": {"kind": "command", "command": command, "cwd": "project_root"},
        "gap_permitted": false,
        "judgment": {"verdict": verdict, "counterexample": "", "reason": "", "host_call_id": format!("judge:{id}")}
    })
}

fn contract(entries: Vec<Value>) -> Value {
    json!({
        "schema_version": 1,
        "prd": {"path": "prd.md", "digest": "prd-digest"},
        "gap_policy": {},
        "acceptance": entries,
    })
}

fn bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec_pretty(value).unwrap()
}

fn skeleton(acceptance_digest: &str, extra: &str) -> Value {
    json!({"acceptance_digest": acceptance_digest, "tasks": [extra]})
}

fn stamp() -> FreezeGateStamp {
    FreezeGateStamp {
        mode: FreezeGateMode::Observe,
        finding_count: 0,
        findings_digest: empty_gate_findings_digest(),
        binary_commit: "abc".into(),
        evaluated_at: "2026-01-01T00:00:00Z".into(),
    }
}

/// One published version of a chain: its contract, skeleton and pin.
struct Version {
    contract: Vec<u8>,
    skeleton: Vec<u8>,
    pin: AcceptancePin,
}

fn version(root: &Path, entries: Vec<Value>, extra: &str) -> Version {
    let contract = bytes(&contract(entries));
    let digest = content_digest(&contract);
    let skeleton = bytes(&skeleton(&digest, extra));
    Version {
        pin: AcceptancePin {
            task_root: root.display().to_string(),
            acceptance_digest: digest.clone(),
            freeze_event_id: format!("acceptance-freeze-{}", &digest[..12]),
            acceptance_gate: stamp(),
            skeleton_digest: Some(content_digest(&skeleton)),
            skeleton_gate: Some(stamp()),
            fidelity_waivers: Vec::new(),
            lineage: Vec::new(),
            lineage_recording: None,
        },
        contract,
        skeleton,
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    tasks: PathBuf,
    history: ChainHistory,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let tasks = dir.path().join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    let history = ChainHistory::for_pin(&dir.path().join("pins").join("key.json"));
    Fixture {
        _dir: dir,
        tasks,
        history,
    }
}

impl Fixture {
    fn publish(&self, version: &Version) {
        std::fs::write(self.tasks.join(ACCEPTANCE_CONTRACT_FILE), &version.contract).unwrap();
        std::fs::write(self.tasks.join(TASK_SKELETON_FILE), &version.skeleton).unwrap();
    }

    fn archive(&self, version: &Version) {
        self.history.put(&version.contract).unwrap();
        self.history.put(&version.skeleton).unwrap();
    }

    /// The check for a run launched before lineage recording.
    fn verify(&self, launch: &Version, current: &Version) -> Result<ChainProof, ChainRefusal> {
        self.verify_as(LaunchLineage::Predates, launch, current)
    }

    fn verify_as(
        &self,
        launch_lineage: LaunchLineage,
        launch: &Version,
        current: &Version,
    ) -> Result<ChainProof, ChainRefusal> {
        verify_reached_from(
            &launch.pin.identity(),
            launch_lineage,
            &current.pin,
            &self.tasks,
            &self.history,
        )
    }
}

fn launch_and_repair(fixture: &Fixture) -> (Version, Version) {
    let launch = version(
        &fixture.tasks,
        vec![
            entry("A", "true", "accepted"),
            entry("B", "false", "refuted"),
        ],
        "same",
    );
    let current = version(
        &fixture.tasks,
        vec![
            entry("A", "true", "accepted"),
            entry("B", "exit 0", "accepted"),
        ],
        "same",
    );
    fixture.publish(&current);
    (launch, current)
}

fn check_of(result: Result<ChainProof, ChainRefusal>) -> ChainCheck {
    result.expect_err("the chain check must refuse").check
}

#[test]
fn the_launch_pin_itself_is_identical() {
    let fixture = fixture();
    let (launch, _) = launch_and_repair(&fixture);
    fixture.publish(&launch);
    let proof = fixture.verify(&launch, &launch).unwrap();
    assert!(proof.identical);
}

#[test]
fn a_moved_pin_with_no_record_is_an_unrecorded_change_naming_the_history() {
    let fixture = fixture();
    let (launch, current) = launch_and_repair(&fixture);
    let refusal = fixture.verify(&launch, &current).unwrap_err();
    assert_eq!(refusal.check, ChainCheck::UnrecordedChange);
    assert!(
        refusal
            .to_string()
            .starts_with("chain check unrecorded_change failed:")
    );
    assert!(refusal.detail.contains(&launch.pin.acceptance_digest));
}

#[test]
fn an_archived_launch_proves_a_change_limited_to_one_accepted_check() {
    let fixture = fixture();
    let (launch, current) = launch_and_repair(&fixture);
    fixture.archive(&launch);
    let proof = fixture.verify(&launch, &current).unwrap();
    assert!(!proof.identical);
    assert_eq!(proof.recorded_hops, None);
    assert_eq!(proof.changed_ids, BTreeSet::from(["B".to_string()]));
}

#[test]
fn every_other_kind_of_change_is_refused_by_name() {
    type Case = (&'static str, Vec<Value>, &'static str, ChainCheck);
    let mut criterion = entry("B", "exit 0", "accepted");
    criterion["criterion"] = json!("a weaker criterion");
    let mut gap = entry("B", "exit 0", "accepted");
    gap["gap_permitted"] = json!(true);
    let cases: Vec<Case> = vec![
        (
            "criterion",
            vec![entry("A", "true", "accepted"), criterion],
            "same",
            ChainCheck::CriterionChanged,
        ),
        (
            "gap",
            vec![entry("A", "true", "accepted"), gap],
            "same",
            ChainCheck::GapPermittedChanged,
        ),
        (
            "added",
            vec![
                entry("A", "true", "accepted"),
                entry("B", "exit 0", "accepted"),
                entry("C", "true", "accepted"),
            ],
            "same",
            ChainCheck::CheckSetChanged,
        ),
        (
            "reordered",
            vec![
                entry("B", "exit 0", "accepted"),
                entry("A", "true", "accepted"),
            ],
            "same",
            ChainCheck::CheckSetChanged,
        ),
        (
            "refuted",
            vec![
                entry("A", "true", "accepted"),
                entry("B", "exit 0", "refuted"),
            ],
            "same",
            ChainCheck::NotJudgeAccepted,
        ),
        (
            "skeleton",
            vec![
                entry("A", "true", "accepted"),
                entry("B", "exit 0", "accepted"),
            ],
            "other",
            ChainCheck::SkeletonChanged,
        ),
    ];
    for (label, entries, extra, expected) in cases {
        let fixture = fixture();
        let (launch, _) = launch_and_repair(&fixture);
        fixture.archive(&launch);
        let current = version(&fixture.tasks, entries, extra);
        fixture.publish(&current);
        assert_eq!(
            check_of(fixture.verify(&launch, &current)),
            expected,
            "{label}"
        );
    }
}

#[test]
fn a_changed_contract_field_is_refused() {
    let fixture = fixture();
    let (launch, _) = launch_and_repair(&fixture);
    fixture.archive(&launch);
    let mut changed = contract(vec![
        entry("A", "true", "accepted"),
        entry("B", "exit 0", "accepted"),
    ]);
    changed["prd"]["digest"] = json!("another prd");
    let mut current = version(&fixture.tasks, Vec::new(), "same");
    current.contract = bytes(&changed);
    current.pin.acceptance_digest = content_digest(&current.contract);
    current.skeleton = bytes(&skeleton(&current.pin.acceptance_digest, "same"));
    current.pin.skeleton_digest = Some(content_digest(&current.skeleton));
    fixture.publish(&current);
    assert_eq!(
        check_of(fixture.verify(&launch, &current)),
        ChainCheck::ContractFieldChanged
    );
}

#[test]
fn bytes_on_disk_the_pin_does_not_bind_are_refused() {
    let fixture = fixture();
    let (launch, current) = launch_and_repair(&fixture);
    fixture.archive(&launch);
    std::fs::write(fixture.tasks.join(ACCEPTANCE_CONTRACT_FILE), b"{}").unwrap();
    assert_eq!(
        check_of(fixture.verify(&launch, &current)),
        ChainCheck::CurrentUnbound
    );
}

fn link(lineage: &[PinTransition], from: &Version, to: &Version, ids: &[&str]) -> PinTransition {
    PinTransition::extending(
        lineage,
        from.pin.identity(),
        to.pin.identity(),
        ids.iter().map(|id| id.to_string()).collect(),
        "test",
    )
}

#[test]
fn a_recorded_lineage_names_the_checks_that_may_change() {
    let fixture = fixture();
    let (launch, mut current) = launch_and_repair(&fixture);
    fixture.archive(&launch);
    current.pin.lineage = vec![link(&[], &launch, &current, &["A"])];
    assert_eq!(
        check_of(fixture.verify(&launch, &current)),
        ChainCheck::UnnamedCheckChanged
    );
    current.pin.lineage = vec![link(&[], &launch, &current, &["B"])];
    let proof = fixture.verify(&launch, &current).unwrap();
    assert_eq!(proof.recorded_hops, Some(1));
}

#[test]
fn a_multi_hop_lineage_is_followed_from_the_launch_pin() {
    let fixture = fixture();
    let (launch, mut current) = launch_and_repair(&fixture);
    fixture.archive(&launch);
    let middle = version(
        &fixture.tasks,
        vec![
            entry("A", "true", "accepted"),
            entry("B", "exit 1", "accepted"),
        ],
        "same",
    );
    let first = link(&[], &launch, &middle, &["B"]);
    let second = link(std::slice::from_ref(&first), &middle, &current, &["B"]);
    current.pin.lineage = vec![first, second];
    assert_eq!(
        fixture.verify(&launch, &current).unwrap().recorded_hops,
        Some(2)
    );
}

#[test]
fn a_lineage_that_does_not_link_up_or_end_at_the_pin_is_broken() {
    let fixture = fixture();
    let (launch, mut current) = launch_and_repair(&fixture);
    fixture.archive(&launch);
    let middle = version(
        &fixture.tasks,
        vec![
            entry("A", "true", "accepted"),
            entry("B", "exit 1", "accepted"),
        ],
        "same",
    );
    let first = link(&[], &launch, &middle, &["B"]);
    let mut forged = link(std::slice::from_ref(&first), &middle, &current, &["B"]);
    forged.prior_link_digest = Some("forged".into());
    current.pin.lineage = vec![first, forged];
    assert_eq!(
        check_of(fixture.verify(&launch, &current)),
        ChainCheck::LineageBroken
    );
    current.pin.lineage = vec![link(&[], &launch, &launch, &["B"])];
    assert_eq!(
        check_of(fixture.verify(&launch, &current)),
        ChainCheck::LineageBroken
    );
}

#[test]
fn an_import_files_only_digests_the_launch_pin_or_lineage_names() {
    let fixture = fixture();
    let (launch, current) = launch_and_repair(&fixture);
    let named = named_digests(&launch.pin.identity(), &current.pin);
    let refusal = fixture
        .history
        .import(&current.contract, &named)
        .unwrap_err();
    assert_eq!(refusal.check, ChainCheck::UnnamedDigest);
    assert_eq!(
        fixture.history.import(&launch.contract, &named).unwrap().1,
        true
    );
    assert_eq!(
        fixture.history.import(&launch.contract, &named).unwrap().1,
        false
    );
    assert_eq!(
        fixture.history.import(&launch.skeleton, &named).unwrap().1,
        true
    );
    assert!(fixture.verify(&launch, &current).is_ok());
}

#[test]
fn a_stored_version_that_no_longer_hashes_to_its_name_is_corrupt_until_refiled() {
    let fixture = fixture();
    let (launch, current) = launch_and_repair(&fixture);
    fixture.archive(&launch);
    std::fs::write(
        fixture.history.path(&launch.pin.acceptance_digest),
        b"tampered",
    )
    .unwrap();
    assert_eq!(
        check_of(fixture.verify(&launch, &current)),
        ChainCheck::PreimageCorrupt
    );
    assert_eq!(fixture.history.put(&launch.contract).unwrap().1, true);
    assert!(fixture.verify(&launch, &current).is_ok());
}

#[test]
fn a_digest_that_is_not_a_digest_names_no_file() {
    let fixture = fixture();
    let refusal = fixture.history.get("../../etc/passwd").unwrap_err();
    assert_eq!(refusal.check, ChainCheck::HistoryUnavailable);
}

#[test]
fn an_unrecorded_change_must_carry_a_renewed_judgment() {
    let fixture = fixture();
    let launch = version(
        &fixture.tasks,
        vec![
            entry("A", "true", "accepted"),
            entry("B", "true", "accepted"),
        ],
        "same",
    );
    fixture.archive(&launch);
    let mut current = version(
        &fixture.tasks,
        vec![
            entry("A", "exit 0", "accepted"),
            entry("B", "true", "accepted"),
        ],
        "same",
    );
    fixture.publish(&current);
    assert_eq!(
        check_of(fixture.verify(&launch, &current)),
        ChainCheck::JudgmentNotRenewed
    );
    current.pin.lineage = vec![link(&[], &launch, &current, &["A"])];
    assert!(fixture.verify(&launch, &current).is_ok());
}

#[test]
fn links_before_the_launch_pin_are_not_read() {
    let fixture = fixture();
    let (launch, mut current) = launch_and_repair(&fixture);
    fixture.archive(&launch);
    let earlier = version(&fixture.tasks, vec![entry("A", "old", "accepted")], "old");
    let mut stale = link(&[], &earlier, &earlier, &["A"]);
    stale.prior_link_digest = Some("not this run's".into());
    let own = link(std::slice::from_ref(&stale), &launch, &current, &["B"]);
    current.pin.lineage = vec![stale, own];
    assert_eq!(
        fixture.verify(&launch, &current).unwrap().recorded_hops,
        Some(1)
    );
}

#[path = "task_set_lineage_launch_tests.rs"]
mod launch_marker;
