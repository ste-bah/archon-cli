use super::*;

#[test]
fn reverify_remediate_repairs_with_open_findings_then_reverifies_to_accepted() {
    let project = project_with(Plan {
        variant: Variant::SiblingCheck,
        block: None,
    });
    let root = project.path();
    let initial = run(root, &decompose_args());
    assert!(
        initial.status.success(),
        "{}",
        String::from_utf8_lossy(&initial.stderr)
    );
    let run_dir = one_run(root);
    let run_id = run_dir.file_name().unwrap().to_str().unwrap();
    let body = root.join("tasks/PRD-X/TASK-X-010.md");
    fs::write(&body, "not a task body\n").unwrap();
    let old_records = record_bytes(&run_dir);
    let requests_before = project.provider.requests.snapshot().len();
    let repaired = run(root, &["workflow", "reverify", run_id, "--remediate"]);
    let stdout = String::from_utf8_lossy(&repaired.stdout);
    let stderr = String::from_utf8_lossy(&repaired.stderr);
    assert!(repaired.status.success(), "stdout={stdout} stderr={stderr}");
    assert!(stdout.contains("Outcome: accepted"), "{stdout}");
    assert!(!stdout.contains("Resume:"), "{stdout}");
    assert!(!stderr.contains("Resume:"), "{stderr}");
    let reports = fs::read_dir(run_dir.join("decomposition/reverify/reports"))
        .unwrap()
        .map(|entry| {
            serde_json::from_slice::<serde_json::Value>(&fs::read(entry.unwrap().path()).unwrap())
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(reports.len(), 2);
    let finding = reports
        .iter()
        .find(|report| report["outcome"] == "needs_remediation")
        .unwrap();
    let accepted = reports
        .iter()
        .find(|report| report["outcome"] == "accepted")
        .unwrap();
    let findings = finding["subjects"][0]["findings"].as_array().unwrap();
    assert!(!findings.is_empty());
    let requests = project.provider.requests.snapshot();
    let repair_requests = &requests[requests_before..];
    let author = repair_requests
        .iter()
        .find(|request| {
            request
                .prompt
                .contains("Author the complete TASK body for host-frozen task_id TASK-X-010")
        })
        .unwrap();
    for finding in findings {
        let quoted = serde_json::to_string(finding.as_str().unwrap()).unwrap();
        assert!(
            author.prompt.contains(&quoted[1..quoted.len() - 1]),
            "author did not receive {finding}: {}",
            author.prompt
        );
    }
    assert!(
        repair_requests
            .iter()
            .any(|request| request.prompt.contains("TASK-X-010") && request.role != author.role)
    );
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(run_dir.join("state.json")).unwrap()).unwrap();
    assert_eq!(state["status"], "completed");
    assert!(
        fs::read_to_string(&body)
            .unwrap()
            .contains("task_id: TASK-X-010")
    );
    let retained = tree_bytes(&run_dir.join("v2/results"));
    for (path, bytes) in old_records {
        assert!(
            retained.values().any(|retained| retained == &bytes),
            "prior record was not kept in its slot or archive: {}",
            path.display()
        );
    }
    let consumed =
        fs::read_to_string(run_dir.join("decomposition/reverify/consumed.jsonl")).unwrap();
    assert!(consumed.contains(finding["id"].as_str().unwrap()));
    let events = fs::read_to_string(run_dir.join("events.jsonl")).unwrap();
    assert_eq!(events.matches("decomposition_reverify_round").count(), 2);
    assert!(events.contains("Repair"));
    assert!(events.contains("Accepted"));
    assert_eq!(accepted["supersedes_id"], "fixed-decomposition-final");

    let clean = run(root, &["workflow", "reverify", run_id, "--remediate"]);
    assert!(
        clean.status.success(),
        "{}",
        String::from_utf8_lossy(&clean.stderr)
    );
    let text = String::from_utf8_lossy(&clean.stdout);
    assert!(text.contains("Outcome: accepted"), "{text}");
    assert!(!text.contains("Resume:"), "{text}");
}

#[test]
fn reverify_remediate_reports_real_resume_author_no_progress_pause() {
    let project = project_with(Plan {
        variant: Variant::Golden,
        block: None,
    });
    let root = project.path();
    let initial = run(root, &decompose_args());
    assert!(
        initial.status.success(),
        "{}",
        String::from_utf8_lossy(&initial.stderr)
    );
    let run_dir = one_run(root);
    let run_id = run_dir.file_name().unwrap().to_str().unwrap();
    fs::write(root.join("tasks/PRD-X/TASK-X-010.md"), "not a task body\n").unwrap();
    project
        .provider
        .set_variant_preserving_history(Variant::StalledBody);
    let before = project.provider.requests.snapshot().len();
    let output = run(root, &["workflow", "reverify", run_id, "--remediate"]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "{text}");
    assert!(!text.contains("control or ownership changed"), "{text}");
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(run_dir.join("state.json")).unwrap()).unwrap();
    assert_eq!(state["status"], "paused", "{text}");
    let events = fs::read_to_string(run_dir.join("events.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(events.iter().any(|event| event["kind"] == "resumed"));
    let pause = events
        .iter()
        .rev()
        .find(|event| event["kind"] == "paused")
        .unwrap();
    assert_ne!(pause["detail"]["cause_kind"], "operator_pause");
    let cause = pause["detail"]["cause_reason"].as_str().unwrap();
    assert_eq!(cause, "authored_pending", "{pause}");
    assert!(text.contains(cause), "{text}");
    let stopped = events
        .iter()
        .find(|event| event["detail"]["decision"] == "ResumeStopped")
        .unwrap();
    assert_eq!(stopped["detail"]["cause"], cause);
    assert_eq!(stopped["detail"]["status"], "paused");
    assert!(
        text.contains("Fixed decomposition paused:"),
        "resume output missing: {text}"
    );
    let requests = project.provider.requests.snapshot();
    let attempts = requests[before..]
        .iter()
        .filter(|request| request.role == "body")
        .collect::<Vec<_>>();
    assert!(
        attempts.len() >= 4,
        "no-progress window did not run: {attempts:?}"
    );
    assert!(
        attempts
            .iter()
            .all(|request| request.answer == "not a task body\n")
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["kind"] == "resumed")
            .count(),
        1
    );
}
