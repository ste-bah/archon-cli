use super::*;

#[test]
fn skeleton_freeze_binds_acceptance_and_extends_the_existing_pin() {
    use archon_workflow::task_set_contract::{TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE};
    use archon_workflow::task_skeleton::{TaskSkeleton, TaskSkeletonLock};

    let temp = tempfile::tempdir().unwrap();
    let (tasks, acceptance_pin) = seed_frozen_acceptance(&temp);
    let raw = r#"{
      "schema_version":1,
      "acceptance_digest":"untrusted-draft-link",
      "tasks":[{
        "task_id":"TASK-X-010",
        "file_name":"TASK-X-010-body.md",
        "depends_on":[],
        "blocks":[],
        "implements":["AC-X-001"],
        "deliverable_contracts":[]
      }]
    }"#;
    std::fs::write(tasks.join(TASK_SKELETON_FILE), raw).unwrap();

    let result = freeze_skeleton(temp.path(), &tasks, &temp.path().join("prds/PRD-X.md")).unwrap();
    let skeleton: TaskSkeleton =
        serde_json::from_slice(&std::fs::read(tasks.join(TASK_SKELETON_FILE)).unwrap()).unwrap();
    assert_eq!(skeleton.acceptance_digest, acceptance_pin.acceptance_digest);
    let lock: TaskSkeletonLock =
        serde_json::from_slice(&std::fs::read(tasks.join(TASK_SKELETON_LOCK_FILE)).unwrap())
            .unwrap();
    assert_eq!(lock.acceptance_digest, acceptance_pin.acceptance_digest);
    assert_eq!(lock.digest, result.skeleton_digest);
    let pin: AcceptancePin =
        serde_json::from_slice(&std::fs::read(acceptance_pin_path(temp.path(), &tasks)).unwrap())
            .unwrap();
    assert_eq!(
        pin.skeleton_digest.as_deref(),
        Some(result.skeleton_digest.as_str())
    );
    assert_eq!(pin.acceptance_digest, acceptance_pin.acceptance_digest);
    assert_eq!(pin.freeze_event_id, acceptance_pin.freeze_event_id);
}

#[test]
fn skeleton_freeze_refuses_an_unfrozen_acceptance_without_partial_outputs() {
    use archon_workflow::task_set_contract::{TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE};

    let temp = tempfile::tempdir().unwrap();
    let (tasks, _prd, _) = seed(&temp);
    let original = br#"{"schema_version":1,"acceptance_digest":"draft","tasks":[]}"#;
    std::fs::write(tasks.join(TASK_SKELETON_FILE), original).unwrap();
    let error = freeze_skeleton(temp.path(), &tasks, &temp.path().join("prds/PRD-X.md"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("acceptance pin"), "{error}");
    assert!(error.contains("freeze-acceptance"), "{error}");
    assert_eq!(
        std::fs::read(tasks.join(TASK_SKELETON_FILE)).unwrap(),
        original
    );
    assert!(!tasks.join(TASK_SKELETON_LOCK_FILE).exists());
}

#[test]
fn skeleton_freeze_rechecks_the_prd_digest() {
    use archon_workflow::task_set_contract::TASK_SKELETON_FILE;

    let temp = tempfile::tempdir().unwrap();
    let (tasks, pin) = seed_frozen_acceptance(&temp);
    let contract: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap())
            .unwrap();
    std::fs::write(
        temp.path().join(&contract.prd.path),
        "## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-X-999 | changed |\n",
    )
    .unwrap();
    let original = br#"{"schema_version":1,"acceptance_digest":"draft","tasks":[]}"#;
    std::fs::write(tasks.join(TASK_SKELETON_FILE), original).unwrap();
    let error = freeze_skeleton(temp.path(), &tasks, &temp.path().join("prds/PRD-X.md"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("PRD digest mismatch"), "{error}");
    assert!(error.contains("restore the frozen PRD"), "{error}");
    let after_pin: AcceptancePin =
        serde_json::from_slice(&std::fs::read(acceptance_pin_path(temp.path(), &tasks)).unwrap())
            .unwrap();
    assert_eq!(after_pin, pin);
    assert_eq!(
        std::fs::read(tasks.join(TASK_SKELETON_FILE)).unwrap(),
        original
    );
}

#[test]
fn skeleton_freeze_refuses_an_unowned_prd_obligation() {
    use archon_workflow::task_set_contract::{TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE};

    let temp = tempfile::tempdir().unwrap();
    let (tasks, pin) = seed_frozen_acceptance(&temp);
    let raw = r#"{
      "schema_version":1,
      "acceptance_digest":"draft",
      "tasks":[{
        "task_id":"TASK-X-010",
        "file_name":"TASK-X-010-body.md",
        "depends_on":[],
        "blocks":[],
        "implements":[],
        "deliverable_contracts":[]
      }]
    }"#;
    std::fs::write(tasks.join(TASK_SKELETON_FILE), raw).unwrap();
    let error = freeze_skeleton(temp.path(), &tasks, &temp.path().join("prds/PRD-X.md"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("AC-X-001"), "{error}");
    assert!(error.contains("add it to at least one task"), "{error}");
    assert!(!tasks.join(TASK_SKELETON_LOCK_FILE).exists());
    let after: AcceptancePin =
        serde_json::from_slice(&std::fs::read(acceptance_pin_path(temp.path(), &tasks)).unwrap())
            .unwrap();
    assert_eq!(after, pin);
}

#[test]
fn failed_temp_staging_removes_every_transaction_file() {
    let temp = tempfile::tempdir().unwrap();
    let good = temp.path().join("first.json");
    let impossible = temp.path().join(format!("{}.json", "x".repeat(300)));
    let anchor = tempfile::tempdir().unwrap();
    let error = publish_files_atomically(
        &anchor.path().join("pin.json"),
        temp.path(),
        &[
            (good.clone(), b"first".to_vec()),
            (impossible, b"second".to_vec()),
        ],
        "workflow freeze-skeleton",
    )
    .unwrap_err()
    .to_string();
    assert!(
        !good.exists(),
        "nothing is published before all temps stage"
    );
    let leftovers: Vec<_> = std::fs::read_dir(temp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        leftovers.is_empty(),
        "leftover transaction files after {error}: {leftovers:?}"
    );
}

#[test]
fn skeleton_freeze_refuses_a_dependency_that_declares_no_data_or_ordering() {
    use archon_workflow::task_set_contract::{TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE};

    let temp = tempfile::tempdir().unwrap();
    let (tasks, pin) = seed_frozen_acceptance(&temp);
    let raw = r#"{
      "schema_version":1,
      "acceptance_digest":"draft",
      "tasks":[
        {
          "task_id":"TASK-X-001",
          "file_name":"TASK-X-001-base.md",
          "depends_on":[],
          "blocks":[],
          "implements":[],
          "deliverable_contracts":[]
        },
        {
          "task_id":"TASK-X-010",
          "file_name":"TASK-X-010-body.md",
          "depends_on":[{"task_id":"TASK-X-001"}],
          "blocks":[],
          "implements":["AC-X-001"],
          "deliverable_contracts":[]
        }
      ]
    }"#;
    std::fs::write(tasks.join(TASK_SKELETON_FILE), raw).unwrap();
    let error = freeze_skeleton(temp.path(), &tasks, &temp.path().join("prds/PRD-X.md"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("TASK-X-010"), "{error}");
    assert!(error.contains("TASK-X-001"), "{error}");
    assert!(
        error.contains("add a non-empty consumes list or set ordering_only: true"),
        "{error}"
    );
    assert!(!tasks.join(TASK_SKELETON_LOCK_FILE).exists());
    let after: AcceptancePin =
        serde_json::from_slice(&std::fs::read(acceptance_pin_path(temp.path(), &tasks)).unwrap())
            .unwrap();
    assert_eq!(after, pin);
}

#[test]
fn committed_backup_cleanup_failure_is_reported_without_failing_publication() {
    let backups = vec![(
        std::path::PathBuf::from("target.json"),
        std::path::PathBuf::from(".target.json.old"),
    )];
    let warnings = cleanup_committed_backups(&backups, |_path| {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "read-only directory",
        ))
    });

    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("freeze is already committed"));
    assert!(warnings[0].contains(".target.json.old"));
    assert!(warnings[0].contains("remove the stale backup manually"));
}

#[test]
fn skeleton_candidate_prepares_exact_staged_bytes_without_reading_or_writing_live_draft() {
    use archon_workflow::task_set_contract::{TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE};

    let temp = tempfile::tempdir().unwrap();
    let (tasks, original_pin) = seed_frozen_acceptance(&temp);
    let pin_path = acceptance_pin_path(temp.path(), &tasks);
    let pin_before = std::fs::read(&pin_path).unwrap();
    std::fs::write(tasks.join(TASK_SKELETON_FILE), b"malformed live sentinel").unwrap();
    let candidate = br#"{
      "schema_version":1,
      "acceptance_digest":"untrusted-draft-link",
      "tasks":[{
        "task_id":"TASK-X-010",
        "file_name":"TASK-X-010-body.md",
        "depends_on":[],
        "blocks":[],
        "implements":["AC-X-001"],
        "deliverable_contracts":[]
      }]
    }"#
    .to_vec();

    let prepared = prepare_skeleton_freeze_from_candidate(
        temp.path(),
        &tasks,
        &temp.path().join("prds/PRD-X.md"),
        archon_core::config::GateMode::Observe,
        candidate,
    )
    .unwrap();
    let (evaluation, outputs) = prepared.into_staged_parts();

    assert_eq!(
        std::fs::read(tasks.join(TASK_SKELETON_FILE)).unwrap(),
        b"malformed live sentinel"
    );
    assert!(!tasks.join(TASK_SKELETON_LOCK_FILE).exists());
    assert_eq!(std::fs::read(&pin_path).unwrap(), pin_before);
    assert!(evaluation.findings.is_empty());
    assert_eq!(
        outputs
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>(),
        [
            "task-skeleton.json",
            "task-skeleton.lock",
            "acceptance-pin.json"
        ]
    );
    let skeleton: archon_workflow::task_skeleton::TaskSkeleton =
        serde_json::from_slice(&outputs[0].1).unwrap();
    assert_eq!(skeleton.acceptance_digest, original_pin.acceptance_digest);
}

#[test]
fn a_skeleton_candidate_the_host_rejects_is_tagged_as_the_authors_mistake() {
    // A malformed task id is a defect in what the model wrote, not a host
    // malfunction. Tagging it routes the reason back to the author as a
    // finding; untagged, it becomes an operational error carrying no findings
    // and the author retries blind until its attempts run out.
    let temp = tempfile::tempdir().unwrap();
    let (tasks, _pin) = seed_frozen_acceptance(&temp);
    let candidate = br#"{
      "schema_version":1,
      "acceptance_digest":"untrusted-draft-link",
      "tasks":[{
        "task_id":"TASK-X-0010",
        "file_name":"TASK-X-0010-body.md",
        "depends_on":[],
        "blocks":[],
        "implements":["AC-X-001"],
        "deliverable_contracts":[]
      }]
    }"#
    .to_vec();

    let error = prepare_skeleton_freeze_from_candidate(
        temp.path(),
        &tasks,
        &temp.path().join("prds/PRD-X.md"),
        archon_core::config::GateMode::Observe,
        candidate,
    )
    .expect_err("a malformed task id must not freeze");

    assert!(
        crate::command::workflow_task_set::CandidateRejected::caused(&error),
        "{error:#}"
    );
    assert!(format!("{error:#}").contains("TASK-X-0010"), "{error:#}");
}

#[test]
fn a_skeleton_candidate_that_is_not_json_is_tagged_as_the_authors_mistake() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, _pin) = seed_frozen_acceptance(&temp);

    let error = prepare_skeleton_freeze_from_candidate(
        temp.path(),
        &tasks,
        &temp.path().join("prds/PRD-X.md"),
        archon_core::config::GateMode::Observe,
        b"{\"schema_version\":1,\"tasks\":\"not-a-list\"}".to_vec(),
    )
    .expect_err("a candidate of the wrong shape must not freeze");

    assert!(
        crate::command::workflow_task_set::CandidateRejected::caused(&error),
        "{error:#}"
    );
}

/// Issue 312: once the precheck accepts what the skeleton reader accepts, the
/// rest of the freeze must too. Fields the reader ignores hold values a strict
/// `Value` read refuses; the freeze stages without them, never failing.
#[test]
fn freeze_shape_ignored_fields_the_reader_accepts_reach_staging() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, _pin) = seed_frozen_acceptance(&temp);
    let deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
    let candidate = [
        br#"{"schema_version":1,"acceptance_digest":"d","zz":"#.as_slice(),
        deep.as_bytes(),
        br#","tasks":[{"task_id":"TASK-X-010","file_name":"TASK-X-010-body.md","#,
        br#""implements":["AC-X-001"],"z1":"\ud800","z2":1e400,"z3":""#,
        &[0xFF, 0xFE],
        br#""}]}"#,
    ]
    .concat();
    let prepared = prepare_skeleton_freeze_from_candidate(
        temp.path(),
        &tasks,
        &temp.path().join("prds/PRD-X.md"),
        archon_core::config::GateMode::Observe,
        candidate,
    )
    .expect("a candidate the reader accepts must stage");
    let (_, outputs) = prepared.into_staged_parts();
    let staged: serde_json::Value = serde_json::from_slice(&outputs[0].1).unwrap();
    assert_eq!(staged["tasks"][0]["task_id"], "TASK-X-010");
    assert!(staged.get("zz").is_none() && staged["tasks"][0].get("z1").is_none());
}

fn prepare_candidate(candidate: &[u8]) -> Result<PreparedSkeletonFreeze> {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, _pin) = seed_frozen_acceptance(&temp);
    prepare_skeleton_freeze_from_candidate(
        temp.path(),
        &tasks,
        &temp.path().join("prds/PRD-X.md"),
        archon_core::config::GateMode::Observe,
        candidate.to_vec(),
    )
}

/// Issue 312: the marker scan reads the skeleton document, and still refuses
/// a marker in a field the freeze stages. The ignored lone surrogate beside it
/// proves the scan read it that way: a strict `Value` read would fail untagged.
#[test]
fn freeze_shape_a_marker_in_a_read_field_is_still_refused() {
    for (file_name, artifact_path) in [
        ("<redacted>", "out/a.json"),
        ("TASK-X-010-body.md", "out/ <redacted>"),
    ] {
        let candidate = format!(
            r#"{{"schema_version":1,"acceptance_digest":"d","tasks":[{{
              "task_id":"TASK-X-010","file_name":"{file_name}","implements":["AC-X-001"],
              "zz":"\ud800","deliverable_contracts":[{{"kind":"file","artifact_path":"{artifact_path}"}}]}}]}}"#
        );
        let error = prepare_candidate(candidate.as_bytes())
            .expect_err("a marker in a staged field must not freeze");
        let text = format!("{error:#}");
        assert!(
            crate::command::workflow_task_set::CandidateRejected::caused(&error),
            "{text}"
        );
        assert!(text.contains("log-redaction marker"), "{text}");
    }
}

/// Issue 312: a marker only in a field the reader ignores is not staged, so
/// the freeze accepts it and the staged skeleton does not hold it.
#[test]
fn freeze_shape_a_marker_only_in_an_ignored_field_is_not_staged() {
    let candidate = br#"{"schema_version":1,"acceptance_digest":"d","tasks":[
      {"task_id":"TASK-X-001","file_name":"TASK-X-001-base.md"},
      {"task_id":"TASK-X-010","file_name":"TASK-X-010-body.md","implements":["AC-X-001"],
       "depends_on":[{"task_id":"TASK-X-001","ordering_only":true,"zz":"<redacted>"}]}]}"#;
    let prepared = prepare_candidate(candidate).expect("an ignored marker must stage");
    let (_, outputs) = prepared.into_staged_parts();
    let staged = &outputs[0].1;
    let dependency =
        &serde_json::from_slice::<serde_json::Value>(staged).unwrap()["tasks"][1]["depends_on"][0];
    assert_eq!(dependency["task_id"], "TASK-X-001");
    let text = String::from_utf8_lossy(staged);
    assert!(
        !text.contains("<redacted>") && !text.contains("zz"),
        "{text}"
    );
}
