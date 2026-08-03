/// Without a declared calendar the check must stay silent -- it is opt-in, and
/// a 24/7 venue trades every day.
#[test]
fn a_venue_with_no_declared_calendar_keeps_every_session() {
    let rows: Vec<_> = (0..40)
        .map(|index| {
            let jitter = ((index * 7919) % 977) as f64 / 100.0;
            serde_json::json!({"timestamp": 1_704_067_200i64 + index * 86_400,
                               "value": 100.0 + index as f64 * 0.4 + jitter,
                               "measure": 50.0 + index as f64 * 0.9 + jitter})
        })
        .collect();
    let (project, mut contract) = series_project(rows);
    contract["observed_time_field"] = serde_json::json!("timestamp");
    let stdout = verifier_stdout(&project, &contract);
    assert!(!stdout.contains("when the venue was closed"), "{stdout}");
}

/// A markdown deliverable declared as `text` must pass on existence rather
/// than being parsed as JSON. Without this the verifier demoted correct work
/// permanently: no remediation can make prose parse, so the task looped to its
/// round cap and blocked on a defect that was in the contract, not the work.
#[test]
fn a_textual_deliverable_is_checked_for_presence_not_parsed_as_json() {
    let project = tempfile::tempdir().expect("project");
    let artifact = project.path().join(".archon/demo/inventory.md");
    std::fs::create_dir_all(artifact.parent().expect("parent")).expect("dir");
    std::fs::write(&artifact, "# Inventory\n\nRules are cited.\n").expect("artifact");
    let contract = serde_json::json!({
        "kind": "rule_inventory",
        "artifact_path": ".archon/demo/inventory.md",
        "artifact_format": "text"
    });
    let command = super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );
    let out = run_verifier(&command);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(
        stdout.contains("declared_text_deliverable_present"),
        "{stdout}"
    );
}

/// Still fail-closed: declaring `text` buys presence, not a free pass.
#[test]
fn a_missing_textual_deliverable_still_fails() {
    let project = tempfile::tempdir().expect("project");
    let contract = serde_json::json!({
        "kind": "rule_inventory",
        "artifact_path": ".archon/demo/absent.md",
        "artifact_format": "text"
    });
    let command = super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );
    let out = run_verifier(&command);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("missing or empty"),
        "status={:?}
stdout:
{}
stderr:
{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Inference must not weaken JSON validation: a `.json` deliverable with no
/// declared format is still strictly parsed and still fails when malformed.
#[test]
fn an_undeclared_json_extension_is_still_strictly_parsed() {
    let project = tempfile::tempdir().expect("project");
    let artifact = project.path().join(".archon/demo/thing.json");
    std::fs::create_dir_all(artifact.parent().expect("parent")).expect("dir");
    std::fs::write(&artifact, "# not json\n").expect("artifact");
    let contract = serde_json::json!({
        "kind": "thing",
        "artifact_path": ".archon/demo/thing.json"
    });
    let command = super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );
    let out = run_verifier(&command);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("not valid JSON"),
        "status={:?}
stdout:
{}
stderr:
{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Site 2: the PARAMETERIZED/instance path. A markdown deliverable declared
/// with a `<placeholder>` segment goes down the instance branch, which loaded
/// every instance as JSON independently of the single-artifact branch. Fixing
/// only the single-artifact site left this one broken — and it is the one a
/// per-run report artifact actually travels through, so a helper-level test
/// would have reported the fix working while it still failed live.
#[test]
fn a_parameterized_markdown_instance_is_not_parsed_as_json() {
    let project = tempfile::tempdir().expect("project");
    let report = project.path().join(".archon/demo/runs/run-1/review.md");
    std::fs::create_dir_all(report.parent().expect("parent")).expect("dir");
    std::fs::write(&report, "# Adversarial review\n\nNo blocking issues.\n").expect("report");
    write_json(
        &project.path().join(".archon/demo/runs.json"),
        &serde_json::json!({"records": {"run-1": {"report_path": ".archon/demo/runs/run-1/review.md"}}}),
    );
    let contract = serde_json::json!({
        "kind": "run_review",
        "artifact_path": ".archon/demo/runs/<run-id>/review.md",
        "instance_source_path": ".archon/demo/runs.json",
        "instance_source_records_field": "records",
        "instance_artifact_field": "report_path"
    });
    let command = super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );
    let out = run_verifier(&command);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "markdown instance must not be JSON-parsed: {stdout}"
    );
    assert!(!stdout.contains("not valid JSON"), "{stdout}");
}

/// D3 / prior-run finding F4. A contract naming `<dataset-id>` and binding
/// nothing to it can never pass (joined literally, that path cannot exist) and
/// can never fail (globbed with no floor, zero matches satisfies it). Eight of
/// the seventeen real tasks are shaped exactly like this, and F4 is a run in
/// which one of them was reported present against the wildcard. The gate must
/// refuse the contract and name the token the author has to bind.
#[test]
fn an_unbound_templated_contract_fails_closed_naming_the_token() {
    let project = tempfile::tempdir().expect("project");
    let contract = serde_json::json!({
        "kind": "native_dataset_manifest",
        "artifact_path": ".archon/trading-lab/data/datasets/<dataset-id>/<version>/manifest.json"
    });
    let command = super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );
    let out = run_verifier(&command);
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !out.status.success(),
        "an unbound template must not be satisfiable: {report}"
    );
    assert!(
        report.contains("<dataset-id>") && report.contains("<version>"),
        "every unexpanded token is named: {report}"
    );
    assert!(
        report.contains("no instance binding"),
        "and the reason is the missing binding, not a missing file: {report}"
    );
    assert!(
        report.contains("min_instances"),
        "and the message says what to declare: {report}"
    );
}

/// The other half of D3: once the template *is* bound, it resolves to the
/// instances the binding names and the gate reports how many it checked. Both
/// declared forms are exercised — a source collection, and a glob with a
/// declared floor — because the floor form is the one the fail-closed rule
/// turns on.
#[test]
fn a_bound_template_resolves_to_its_instances() {
    let project = tempfile::tempdir().expect("project");
    for dataset in ["spy-1d", "qqq-1d"] {
        let manifest = project
            .path()
            .join(format!(".archon/demo/datasets/{dataset}/v1/manifest.json"));
        std::fs::create_dir_all(manifest.parent().expect("parent")).expect("dir");
        write_json(&manifest, &serde_json::json!({"dataset_id": dataset}));
    }
    write_json(
        &project.path().join(".archon/demo/registry.json"),
        &serde_json::json!({"records": {
            "spy-1d": {"manifest_path": ".archon/demo/datasets/spy-1d/v1/manifest.json"},
            "qqq-1d": {"manifest_path": ".archon/demo/datasets/qqq-1d/v1/manifest.json"},
        }}),
    );

    let source_bound = serde_json::json!({
        "kind": "native_dataset_manifest",
        "artifact_path": ".archon/demo/datasets/<dataset-id>/<version>/manifest.json",
        "instance_source_path": ".archon/demo/registry.json",
        "instance_source_records_field": "records",
        "instance_artifact_field": "manifest_path",
        "min_instances": 2
    });
    let glob_bound = serde_json::json!({
        "kind": "native_dataset_manifest",
        "artifact_path": ".archon/demo/datasets/<dataset-id>/<version>/manifest.json",
        "min_instances": 2
    });
    for contract in [source_bound, glob_bound] {
        let command = super::workflow_live_v2_deliverable_contract::verification_command(
            project.path().to_str().expect("project path"),
            &contract,
        );
        let out = run_verifier(&command);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "a bound template resolves: {stdout}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            stdout.contains("declared_deliverable_instances_present"),
            "{stdout}"
        );
        assert!(
            stdout.contains("\"instance_count\": 2"),
            "both instances are checked, not the literal template: {stdout}"
        );
    }
}

/// A declared floor is what makes the glob form falsifiable. Drop one instance
/// and it must fail rather than reporting fewer.
#[test]
fn a_bound_template_below_its_declared_floor_fails() {
    let project = tempfile::tempdir().expect("project");
    let manifest = project
        .path()
        .join(".archon/demo/datasets/spy-1d/v1/manifest.json");
    std::fs::create_dir_all(manifest.parent().expect("parent")).expect("dir");
    write_json(&manifest, &serde_json::json!({"dataset_id": "spy-1d"}));
    let contract = serde_json::json!({
        "kind": "native_dataset_manifest",
        "artifact_path": ".archon/demo/datasets/<dataset-id>/<version>/manifest.json",
        "min_instances": 2
    });
    let command = super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );
    let out = run_verifier(&command);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "{stdout}");
    assert!(
        stdout.contains("requires >= 2 instance(s), found 1"),
        "{stdout}"
    );
}

/// Still fail-closed on the instance path: a declared instance that is missing
/// fails regardless of its extension.
#[test]
fn a_missing_parameterized_instance_still_fails() {
    let project = tempfile::tempdir().expect("project");
    write_json(
        &project.path().join(".archon/demo/runs.json"),
        &serde_json::json!({"records": {"run-1": {"report_path": ".archon/demo/runs/run-1/review.md"}}}),
    );
    let contract = serde_json::json!({
        "kind": "run_review",
        "artifact_path": ".archon/demo/runs/<run-id>/review.md",
        "instance_source_path": ".archon/demo/runs.json",
        "instance_source_records_field": "records",
        "instance_artifact_field": "report_path"
    });
    let command = super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );
    let out = run_verifier(&command);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("missing or empty"),
        "status={:?}
stdout:
{}
stderr:
{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
