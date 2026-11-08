use super::*;
use archon_workflow::task_set_contract::{
    AcceptanceCriterion, GapPolicy, JudgeVerdict, PrdIdentity,
};
use archon_workflow::task_skeleton::FrozenTask;
use archon_workflow::task_universe::WorkflowV2DeliverableContract;

fn contract(command: &str) -> AcceptanceContract {
    AcceptanceContract {
        schema_version: 1,
        prd: PrdIdentity {
            path: "prd.md".into(),
            digest: String::new(),
        },
        gap_policy: GapPolicy {
            permitted_acceptance_ids: BTreeSet::new(),
            forbidden_phrases: vec![],
            required_fields: vec![],
        },
        acceptance: vec![AcceptanceCriterion {
            id: "AC-X-001".into(),
            criterion: "output".into(),
            check: AcceptanceCheck::Command {
                command: command.into(),
                cwd: TrustedCwd::RepoRoot,
            },
            covers: vec!["REQ-X-001".into()],
            gap_permitted: false,
            judgment: JudgeVerdict {
                verdict: archon_workflow::task_set_contract::JudgeDecision::Accepted,
                counterexample: "none".into(),
                reason: "checked".into(),
                host_call_id: "fixture".into(),
                sampling: None,
            },
        }],
        supplementary: vec![],
    }
}

fn skeleton(artifact: &str) -> TaskSkeleton {
    TaskSkeleton {
        schema_version: 1,
        acceptance_digest: String::new(),
        tasks: vec![FrozenTask {
            status: None,
            task_id: "TASK-X-001".into(),
            file_name: "TASK-X-001.md".into(),
            depends_on: vec![],
            blocks: vec![],
            implements: vec!["REQ-X-001".into()],
            source_sections: vec!["0".into()],
            deliverable_contracts: vec![WorkflowV2DeliverableContract {
                kind: "result".into(),
                min_instances: 1,
                artifact_path: artifact.into(),
                ..Default::default()
            }],
        }],
    }
}

#[test]
fn membership_only_and_deliverable_reads_including_directories() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let contract = contract("inspect data/out.json");
    let reads = BTreeMap::from([(
        "AC-X-001".into(),
        BTreeSet::from([root.join("data").join("out.json")]),
    )]);
    for artifact in ["data/out.json", "data"] {
        let d = evaluate(&skeleton(artifact), &contract, root, &reads, None);
        assert!(d.links[0].membership && d.links[0].reads_deliverable);
        assert!(d.membership_only.is_empty());
    }
    let d = evaluate(&skeleton("other.json"), &contract, root, &reads, None);
    assert_eq!(d.membership_only, ["TASK-X-001"]);
    assert!(!d.links[0].reads_deliverable);
    let mut s = skeleton("data");
    s.tasks[0].implements.clear();
    let d = evaluate(&s, &contract, root, &reads, None);
    assert!(!d.links[0].membership && d.links[0].reads_deliverable);
}

#[test]
fn directory_read_links_every_contained_artifact() {
    let repo = repository();
    let root = repo.path();
    std::fs::create_dir(root.join("data")).unwrap();
    let mut s = skeleton("data/first.json");
    let mut second = s.tasks[0].clone();
    second.task_id = "TASK-X-002".into();
    second.implements.clear();
    second.deliverable_contracts[0].artifact_path = "data/second.json".into();
    s.tasks.push(second);
    let d = discover(&s, &contract("inspect data"), root, Some(root), root);
    assert_eq!(d.links.len(), 2);
    assert!(d.links.iter().all(|link| link.reads_deliverable));
    assert!(d.membership_only.is_empty());
}

#[test]
fn absent_unowned_owned_present_at_base_and_unresolvable_base() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repository();
    let repository = repo.path();
    let c = contract("inspect data/missing.json data/owned.json present.txt");
    let s = skeleton("data/owned.json");
    let d = discover(&s, &c, repository, Some(repository), temp.path());
    assert!(d.base_unavailable.is_none());
    assert_eq!(
        d.unowned,
        [(
            "AC-X-001".into(),
            repository.join("data").join("missing.json")
        )]
    );
    let d = discover(&s, &c, temp.path(), Some(temp.path()), temp.path());
    assert!(d.base_unavailable.is_some());
    assert!(d.unowned.is_empty());
}

#[test]
fn directory_operand_and_manifest_are_owned_without_movability_rules() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repository();
    let repository = repo.path();
    let c = contract("inspect data/input Cargo.toml");
    let d = discover(
        &skeleton("data/input/file.json"),
        &c,
        repository,
        Some(repository),
        temp.path(),
    );
    assert!(d.unowned.is_empty(), "{d:?}");
    assert!(d.links[0].reads_deliverable);
    let c = contract("inspect data/Cargo.toml");
    let d = discover(
        &skeleton("data/Cargo.toml"),
        &c,
        repository,
        Some(repository),
        temp.path(),
    );
    assert!(d.base_unavailable.is_none());
    assert!(d.unowned.is_empty(), "{d:?}");
    assert!(d.links[0].reads_deliverable, "{d:?}");
}

#[test]
fn freeze_findings_stamp_lock_and_all_staged_bytes_are_unchanged() {
    let temp = repository();
    let root = temp.path();
    let tasks = root.join("tasks");
    std::fs::create_dir(&tasks).unwrap();
    let prd = root.join("prd.md");
    let text = "- REQ-X-001: output exists.\n\n## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-X-001 | output |\n";
    std::fs::write(&prd, text).unwrap();
    let mut c = contract("inspect data/missing.json");
    c.prd.digest = content_digest(text.as_bytes());
    let bytes = serde_json::to_vec_pretty(&c).unwrap();
    let (lock, pin) =
        findings::acceptance_lock_and_pin(&tasks, FreezeGateMode::Observe, &[], &bytes, None);
    std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), bytes).unwrap();
    std::fs::write(
        tasks.join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE),
        serde_json::to_vec(&lock).unwrap(),
    )
    .unwrap();
    let pin_path = acceptance_pin_path(root, &tasks);
    std::fs::create_dir_all(pin_path.parent().unwrap()).unwrap();
    std::fs::write(pin_path, serde_json::to_vec(&pin).unwrap()).unwrap();
    let mut s = skeleton("data/own.json");
    s.tasks[0].implements.push("AC-X-001".into());
    let candidate = serde_json::to_vec(&s).unwrap();
    // Hold the clock constant while comparing complete publication bytes.
    fn fixed_stamp(
        mode: FreezeGateMode,
        findings: &[GateFinding],
    ) -> archon_workflow::task_set_contract::FreezeGateStamp {
        let mut stamp = gate_stamp(mode, findings);
        stamp.evaluated_at = "2026-11-08T00:00:00+00:00".into();
        stamp
    }
    let files_before = snapshot(root);
    let before = super::super::skeleton_freeze::prepare_with_diagnostics(
        root,
        &tasks,
        &prd,
        GateMode::Observe,
        candidate.clone(),
        |_, _, _, _| {},
        fixed_stamp,
    )
    .unwrap();
    let after = super::super::skeleton_freeze::prepare_with_diagnostics(
        root,
        &tasks,
        &prd,
        GateMode::Observe,
        candidate,
        child,
        fixed_stamp,
    )
    .unwrap();
    assert_eq!(
        format!("{:?}", before.findings),
        format!("{:?}", after.findings)
    );
    assert_eq!(
        serde_json::to_vec(&before.lock).unwrap(),
        serde_json::to_vec(&after.lock).unwrap()
    );
    assert_eq!(
        serde_json::to_vec(&before.pin).unwrap(),
        serde_json::to_vec(&after.pin).unwrap()
    );
    assert_eq!(before.skeleton_bytes, after.skeleton_bytes);
    assert_eq!(format!("{before:?}"), format!("{after:?}"));
    assert_eq!(files_before, snapshot(root));
    let log = tasks.join(".decompose.log");
    let first = std::fs::read_to_string(&log).unwrap();
    assert!(first.contains("unowned_check_input"));
    super::super::skeleton_freeze::prepare_with_diagnostics(
        root,
        &tasks,
        &prd,
        GateMode::Observe,
        serde_json::to_vec(&s).unwrap(),
        child,
        fixed_stamp,
    )
    .unwrap();
    assert_eq!(first, std::fs::read_to_string(log).unwrap());
}

#[test]
fn live_only_inside_is_unowned_and_outside_is_separate() {
    let repo = repository();
    let root = repo.path();
    let outside = tempfile::tempdir().unwrap();
    let c = contract("inspect data/out.json ../outside/out.json");
    std::fs::create_dir(root.join("data")).unwrap();
    std::fs::write(root.join("data").join("out.json"), "live only").unwrap();
    let d = discover(
        &skeleton("other.json"),
        &c,
        root,
        Some(root),
        outside.path(),
    );
    assert_eq!(
        d.unowned,
        [("AC-X-001".into(), root.join("data").join("out.json"))]
    );
    assert_eq!(d.outside.len(), 1);
    assert!(d.outside.contains(&(
        "AC-X-001".into(),
        root.parent().unwrap().join("outside").join("out.json"),
    )));
}

#[cfg(unix)]
#[test]
fn absolute_operand_outside_is_separate() {
    let repo = repository();
    let outside = tempfile::tempdir().unwrap();
    let path = outside.path().join("input.json");
    let c = contract(&format!("inspect ../outside/out.json {}", path.display()));
    let d = discover(
        &skeleton("other.json"),
        &c,
        repo.path(),
        Some(repo.path()),
        repo.path(),
    );
    assert_eq!(d.outside.len(), 2, "{d:?}");
    assert!(d.outside.contains(&(
        "AC-X-001".into(),
        absolute(repo.path(), &path.to_string_lossy())
    )));
}

pub(super) fn repository() -> tempfile::TempDir {
    repository_with(&[("present.txt", "base"), ("Cargo.toml", "[package]\n")])
}

fn repository_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let repo = tempfile::tempdir_in(archon_shell::paths::plain(
        std::env::current_dir().unwrap().canonicalize().unwrap(),
    ))
    .unwrap();
    for (name, text) in files {
        let path = repo.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    for args in [
        vec!["init"],
        vec!["add", "."],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "base",
        ],
    ] {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    repo
}

pub(crate) fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() == ".decompose.log" {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                walk(root, &entry.path(), files);
            } else {
                files.insert(
                    entry.path().strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(root, root, &mut files);
    files
}

#[test]
fn mechanics_are_reads_messages_are_not_and_bare_missing_is_diagnostic_only() {
    let repo = repository();
    let root = repo.path();
    for (command, expected) in [
        ("bash scripts/absent.sh", vec!["scripts/absent.sh"]),
        ("bash absent.sh", vec!["absent.sh"]),
        (
            "bash scripts/verify.sh || echo \"scripts/verify.sh failed\"",
            vec!["scripts/verify.sh"],
        ),
        (
            "test -f data/in.json || { echo \"data/in.json missing\"; exit 1; }",
            vec!["data/in.json"],
        ),
        ("mkdir -p build/out && run > build/out/r.json", vec![]),
        ("echo \"data/x.json missing\"", vec![]),
        ("inspect report.json", vec!["report.json"]),
    ] {
        let c = contract(command);
        let discovery = input_discovery_for_check(&c.acceptance[0], root, root, None);
        if command == "inspect report.json" {
            assert!(discovery.candidates.is_empty());
            assert!(discovery.eligible.is_empty());
            assert_eq!(
                discovery
                    .unshaped_missing
                    .iter()
                    .map(Path::new)
                    .collect::<Vec<_>>(),
                [Path::new("report.json")]
            );
        }
        if command == "bash scripts/absent.sh" {
            assert!(discovery.eligible.is_empty());
            assert_eq!(discovery.candidates.len(), 1);
            assert_eq!(
                Path::new(&discovery.candidates[0].path),
                Path::new("scripts/absent.sh")
            );
            assert_eq!(
                discovery.candidates[0].exclusion,
                Some(ExclusionReason::CheckMechanics)
            );
        }
        let d = discover(&skeleton("owned.json"), &c, root, Some(root), root);
        assert_eq!(
            d.unowned.iter().map(|(_, p)| p.clone()).collect::<Vec<_>>(),
            expected
                .iter()
                .map(|p| p.split('/').fold(root.to_path_buf(), |a, c| a.join(c)))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn unavailable_selection_skips_base_and_log_failure_is_nonfatal() {
    let repo = repository();
    let root = repo.path();
    let d = discover(
        &skeleton("owned.json"),
        &contract("inspect absent/file.json"),
        root,
        None,
        root,
    );
    assert!(d.base_unavailable.is_some());
    assert!(d.unowned.is_empty());
    std::fs::create_dir(root.join(".decompose.log")).unwrap();
    persist(root, &lines(&d));
}

#[test]
fn quoted_patterns_and_embedded_expressions_are_observations_only() {
    let repo = repository();
    let root = repo.path();
    let c = contract(
        "inspect 'strategy.entry' 'raw_bound_version(&request.created_at,' 'assert!(result.is_err())' --threshold 0.95",
    );
    let d = discover(&skeleton("owned.json"), &c, root, Some(root), root);
    assert!(d.unowned.is_empty(), "{d:?}");
    assert_eq!(
        d.unconfirmed,
        [("AC-X-001".into(), root.join("strategy.entry"))]
    );
    assert!(
        lines(&d)
            .iter()
            .any(|line| line.contains("unconfirmed_read"))
    );
    std::fs::write(root.join("strategy.entry"), "live only").unwrap();
    let live = discover(&skeleton("owned.json"), &c, root, Some(root), root);
    assert!(live.unowned.is_empty(), "{live:?}");
    assert_eq!(live.unconfirmed, d.unconfirmed);
}

#[cfg(windows)]
#[test]
fn rooted_and_backslash_reads_are_observed_without_bare_fragments() {
    let repo = repository();
    let root = repo.path();
    let c = contract("inspect /etc/x 'data\\x.json'");
    let reads = input_discovery_for_check(&c.acceptance[0], root, root, None);
    assert_eq!(
        reads
            .absolute_reads
            .iter()
            .map(Path::new)
            .collect::<Vec<_>>(),
        [Path::new("/etc/x")]
    );
    assert!(reads.unshaped_missing.is_empty());
    let d = discover(&skeleton("owned.json"), &c, root, Some(root), root);
    assert_eq!(d.outside.len(), 1, "{d:?}");
    assert!(d.unowned.is_empty(), "{d:?}");
    assert!(d.unconfirmed.is_empty(), "{d:?}");
}

#[path = "workflow_acceptance_link_diagnostics_round4_tests.rs"]
mod round4;
