use super::*;
use std::path::Path;

fn corrupt_owned_entry(root: &Path) {
    let tasks = root.join("tasks/PRD-X");
    let path = tasks.join("task-skeleton.json");
    let mut skeleton: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    skeleton["tasks"][0]["file_name"] = "../TASK-X-010.md".into();
    let bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    let digest = archon_workflow::task_set_contract::content_digest(&bytes);
    fs::write(path, bytes).unwrap();
    let pins = fs::read_dir(root.join(".archon/task-set-pins")).unwrap();
    let pin = pins
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "json"))
        .unwrap();
    for (path, field) in [
        (tasks.join("task-skeleton.lock"), "digest"),
        (pin, "skeleton_digest"),
    ] {
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value[field] = digest.clone().into();
        fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
}

#[test]
fn frozen_chain_authored_launch_repairs_through_real_host_and_keeps_unaffected_body() {
    let project = project();
    let root = project.path();
    let first = run(root, &decompose_args());
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let kept = root.join("tasks/PRD-X/TASK-X-020.md");
    let bytes = fs::read(&kept).unwrap();
    corrupt_owned_entry(root);
    project.provider.set_variant(server::Variant::Golden);
    let repaired = run(root, &decompose_args());
    assert!(
        repaired.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&repaired.stdout),
        String::from_utf8_lossy(&repaired.stderr)
    );
    assert_eq!(fs::read(kept).unwrap(), bytes);
    let counts = project.provider.counts.snapshot();
    assert_eq!(
        counts.get("skeleton").copied().unwrap_or_default(),
        1,
        "{counts:?}"
    );
    assert_eq!(
        counts.get("body").copied().unwrap_or_default(),
        1,
        "{counts:?}"
    );
    assert_no_unknown_prompts(&project.provider);
}

fn await_held(root: &Path, hold: &Path, child: &mut std::process::Child) {
    let started = std::time::Instant::now();
    while !hold.join("held").exists() {
        assert!(
            child.try_wait().unwrap().is_none(),
            "child exited: {}",
            tail(&deadline::harness_root(root).join("decompose.stderr"))
        );
        if started.elapsed() > std::time::Duration::from_secs(120) {
            let _ = child.kill();
            panic!("test watchdog waiting for held call");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn pause_held(root: &Path, run_id: &str, hold: &Path, child: &mut std::process::Child) {
    assert!(run(root, &["workflow", "pause", run_id]).status.success());
    fs::write(hold.join("release"), b"").unwrap();
    let harness = deadline::harness_root(root);
    assert!(
        wait_for_child(
            child,
            &harness.join("decompose.stdout"),
            &harness.join("decompose.stderr"),
            "held child"
        )
        .success()
    );
}

fn refresh_older_receipts(root: &Path, run_dir: &Path) {
    let canonical_root = root.canonicalize().unwrap();
    for entry in fs::read_dir(run_dir.join("v2/results")).unwrap() {
        let path = entry.unwrap().path();
        let mut record: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        if let Some(version) = record["result"]["data"]["logicVersion"].as_u64() {
            record["result"]["data"]["logicVersion"] = (version - 1).into();
        }
        if record["call"]["options"]["host_command"]["commandId"] != "freeze-skeleton" {
            fs::write(&path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
            continue;
        }
        if let Some(entries) = record
            .get_mut("result")
            .and_then(|r| r.get_mut("data"))
            .and_then(|d| d.get_mut("publicationReceipt"))
            .and_then(|r| r.get_mut("entries"))
            .and_then(serde_json::Value::as_array_mut)
        {
            let mut refreshed = 0;
            for entry in entries {
                let destination = Path::new(entry["destination_path"].as_str().unwrap());
                // Receipts use plain forward-slash paths; Windows canonicalize
                // returns a verbatim root. Compare their filesystem identities.
                if archon_shell::paths::path_starts_with(destination, &canonical_root)
                    && destination.exists()
                {
                    let bytes = fs::read(destination).unwrap();
                    refreshed += 1;
                    entry["byte_len"] = bytes.len().into();
                    entry["blake3"] =
                        archon_workflow::task_set_contract::content_digest(&bytes).into();
                }
            }
            assert!(
                refreshed > 0,
                "freeze-skeleton receipt must name project artifacts"
            );
        }
        fs::write(path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    }
}

#[test]
fn frozen_chain_interrupted_resume_reuses_unaffected_body_after_repaired_publication() {
    let project = project();
    let root = project.path();
    let hold = root.join(".test-hold");
    fs::create_dir_all(&hold).unwrap();
    let mut child = spawn_decompose(
        root,
        &[
            ("ARCHON_TEST_HOLD_CALL", "fixed-decomposition-final".into()),
            ("ARCHON_TEST_HOLD_DIR", hold.display().to_string()),
        ],
    );
    await_held(root, &hold, &mut child);
    let run_dir = one_run(root);
    let run_id = run_dir.file_name().unwrap().to_str().unwrap();
    pause_held(root, run_id, &hold, &mut child);
    let kept = root.join("tasks/PRD-X/TASK-X-020.md");
    let bytes = fs::read(&kept).unwrap();
    corrupt_owned_entry(root);
    refresh_older_receipts(root, &run_dir);
    fs::remove_file(hold.join("held")).unwrap();
    fs::remove_file(hold.join("release")).unwrap();
    project.provider.set_variant(server::Variant::Golden);
    let harness = deadline::harness_root(root);
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_archon"));
    deadline::isolate_child(&mut command, root, &harness);
    let mut child = command
        .current_dir(root)
        .args(["workflow", "resume", "--live", "--yes", run_id])
        .env("ARCHON_TEST_HOLD_CALL", "land-task-body")
        .env("ARCHON_TEST_HOLD_DIR", &hold)
        .stdout(fs::File::create(harness.join("decompose.stdout")).unwrap())
        .stderr(fs::File::create(harness.join("decompose.stderr")).unwrap())
        .spawn()
        .unwrap();
    await_held(root, &hold, &mut child);
    let published: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("tasks/PRD-X/task-skeleton.json")).unwrap())
            .unwrap();
    assert_eq!(
        published["tasks"][0]["file_name"], "TASK-X-010.md",
        "repair must publish before interruption"
    );
    pause_held(root, run_id, &hold, &mut child);
    let resumed = resume(root, run_id);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert_eq!(fs::read(kept).unwrap(), bytes);
    let accepted = |id: &str| {
        fs::read_to_string(run_dir.join("events.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .filter(|event| {
                event["kind"] == "subject_accepted"
                    && event["detail"]["phase"] == "bodies"
                    && event["detail"]["subject"] == id
                    && event["detail"]["reused"] != true
            })
            .count()
    };
    assert_eq!(
        accepted("TASK-X-020"),
        1,
        "unaffected body must keep its original accepted landing through both resumes"
    );
    assert_eq!(
        accepted("TASK-X-010"),
        2,
        "the changed body must have exactly one repaired landing"
    );
}
