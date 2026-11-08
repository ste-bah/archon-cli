//! The two resume cells reuse the existing provider, process isolation and
//! debug-build hold seam. All edits are confined to disposable projects.
use crate::{assertions::*, deadline, project, server};
use serde_json::{Value, json};
use server::variants::r17::Cell;
use std::{
    fs,
    time::{Duration, Instant},
};

fn wait_held(p: &project::TestProject, child: &mut std::process::Child, hold: &std::path::Path) {
    let start = Instant::now();
    while !hold.join("held").exists() {
        assert!(
            child.try_wait().unwrap().is_none(),
            "child exited before held call"
        );
        if start.elapsed() > Duration::from_secs(180) {
            let _ = child.kill();
            panic!("watchdog waiting for body boundary");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(p.path().join("tasks/PRD-X/task-skeleton.json").is_file());
}

fn write_frozen_duplicate(p: &project::TestProject) {
    let tasks = p.path().join("tasks/PRD-X");
    let path = tasks.join("task-skeleton.json");
    let mut skeleton = read(&path);
    skeleton["tasks"][0]["source_sections"]
        .as_array_mut()
        .unwrap()
        .push(json!("2"));
    let bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    let digest = archon_workflow::task_set_contract::content_digest(&bytes);
    fs::write(path, bytes).unwrap();
    let pin = pin_path(p.path());
    for (path, field) in [
        (tasks.join("task-skeleton.lock"), "digest"),
        (pin, "skeleton_digest"),
    ] {
        let mut value = read(&path);
        value[field] = digest.clone().into();
        fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
}

#[test]
fn v7_frozen_duplicate_survives_body_landing_on_resume() {
    let p = fixture(Cell::V7);
    let hold = p.path().join(".test-hold");
    fs::create_dir_all(&hold).unwrap();
    let mut child = project::spawn_decompose(
        p.path(),
        &[
            ("ARCHON_TEST_HOLD_CALL", "land-task-body".into()),
            ("ARCHON_TEST_HOLD_DIR", hold.display().to_string()),
        ],
    );
    wait_held(&p, &mut child, &hold);
    let id = project::run_id(p.path());
    // Crash at the existing host-call boundary, then resume the durable loop.
    // No authored pause is needed for this fixture.
    child.kill().unwrap();
    child.wait().unwrap();
    write_frozen_duplicate(&p);
    let frozen = read(&p.path().join("tasks/PRD-X/task-skeleton.json"));
    assert_eq!(
        frozen["tasks"][0]["source_sections"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| **s == json!("2"))
            .count(),
        2
    );
    success(&project::resume(p.path(), &id));
    // The interrupted body author is replayed from the result store.
    shared(&p, 1, [1, 1], 0);
    let landings = gates(&p, "land-task-body");
    assert!(!landings.is_empty());
    for landing in landings {
        assert!(
            findings(&landing)
                .iter()
                .all(|f| !f["text"].as_str().unwrap().contains("frozen")),
            "frozen-field mismatch: {landing}"
        );
    }
    assert_clean_set_rounds(&p, 1);
}

#[test]
fn v8_no_progress_pause_then_different_build_restores_skeleton_loop() {
    let p = fixture(Cell::V8);
    let mut child = project::spawn_decompose(
        p.path(),
        &[("ARCHON_TEST_BINARY_REVISION", "previous-build".into())],
    );
    let harness = deadline::harness_root(p.path());
    assert!(
        deadline::wait_for_child(
            &mut child,
            &harness.join("decompose.stdout"),
            &harness.join("decompose.stderr"),
            "V8 no-progress triple"
        )
        .success()
    );
    let run = project::one_run(p.path());
    assert_eq!(read(&run.join("state.json"))["status"], "paused");
    let paused = events(&p);
    assert!(
        paused
            .iter()
            .any(|e| e.to_string().contains("no_progress") && e.to_string().contains("skeleton"))
    );
    let before = p
        .provider
        .counts
        .snapshot()
        .get("skeleton")
        .copied()
        .unwrap_or_default();
    // Baseline plus three non-improving attempts exhaust the existing window.
    // Issue 630: the baseline lands and replies (2 requests). Each repair
    // call lands no changed task, and the runner refuses its reply in-session
    // until the incomplete-reply bound ends the call: 3 replies, 3 requests.
    assert_eq!(before, 2 + 3 * 3);
    p.provider
        .set_variant_preserving_history(server::Variant::R17(Cell::V0));
    success(&project::resume(p.path(), &project::run_id(p.path())));
    // The fresh fifth author lands and replies: 2 more skeleton requests.
    shared_with_skeleton_exchanges(&p, 5, [1, 1], 1, before + 2);
    let resumed = events(&p);
    assert!(resumed.iter().any(|e| e["kind"] == "binary_revision_drift"));
    // A build-only drift replays durable calls; a script change uses a
    // phase seed. Prove restoration by the exact reused ordinals and the
    // next fresh author, rather than requiring the other route's event.
    let resume_seq = resumed.iter().find(|e| e["kind"] == "resumed").unwrap()["seq"]
        .as_u64()
        .unwrap();
    for ordinal in 1..=4 {
        assert!(
            resumed
                .iter()
                .any(|e| e["seq"].as_u64().unwrap() > resume_seq
                    && e["detail"]["event"] == "call_reused"
                    && e["detail"]["call_id"] == format!("skeleton-author-{ordinal}"))
        );
    }
    assert!(
        resumed
            .iter()
            .any(|e| e["detail"]["event"] == "author_attempt_started"
                && e["detail"]["call_id"] == "skeleton-author-5"
                && e["detail"]["logical_attempt"] == 5)
    );
    assert_clean_set_rounds(&p, 1);
    let baseline = completed(Cell::V0);
    shared(&baseline, 1, [1, 1], 0);
    let actual = project::final_snapshot(&p, &[]);
    let expected = project::final_snapshot(&baseline, &[]);
    assert_eq!(actual.acceptance, expected.acceptance);
    assert_eq!(actual.skeleton, expected.skeleton);
    assert_eq!(actual.task_bodies, expected.task_bodies);
    // Compare retained final records after removing only the extra skeleton
    // attempts and the resume metadata using the established normalizer.
    let normalize = |snapshot: project::FinalSnapshot| {
        let mut value = serde_json::to_value(snapshot).unwrap();
        value["records"].as_array_mut().unwrap().retain(|r| {
            let call = &r[1]["call"];
            let id = call["id"].as_str().unwrap_or_default();
            !(id.starts_with("skeleton-author-")
                || call["method"] == "checkpoint"
                || call
                    .pointer("/options/host_command/commandId")
                    .and_then(Value::as_str)
                    == Some("freeze-skeleton"))
        });
        // Observe mode publishes even needs_review skeletons (decision.rs).
        // V8's four unsuccessful freeze attempts therefore leave a skeleton
        // and lock before the clean freeze; V0 has no such earlier writes.
        // Only their prior digests differ in these two finalReport copies.
        for record in value["records"].as_array_mut().unwrap() {
            if record[1]["call"]["method"] != "finalReport"
                || record[1]["call"]["id"] != "fixed-decomposition-final"
            {
                continue;
            }
            for path in [
                "/call/options/extra/inputs/1/publicationReceipt/entries",
                "/call/options/extra/inputs/1/result/data/publicationReceipt/entries",
            ] {
                if let Some(entries) = record[1].pointer_mut(path).and_then(Value::as_array_mut) {
                    for entry in entries {
                        if matches!(
                            entry["relative_path"].as_str(),
                            Some("task-skeleton.json" | "task-skeleton.lock")
                        ) {
                            entry.as_object_mut().unwrap().remove("prior_blake3");
                        }
                    }
                }
            }
        }
        project::keyed(value)
    };
    project::assert_same_json(
        "V8 final snapshot minus skeleton history",
        &normalize(actual),
        &normalize(expected),
    );
}
