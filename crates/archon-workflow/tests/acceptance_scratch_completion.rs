#[path = "support/native_fixture.rs"]
mod support;
use archon_workflow::acceptance_scratch::observe_commands;
use archon_workflow::acceptance_world::AcceptanceCommandKind;
use archon_workflow::task_set_contract::AcceptanceCheck;
use support::fixture;

#[tokio::test]
async fn advanced_floor_runs_shared_verifier_before_exact_nested_command() {
    for valid in [false, true] {
        let cmd = "grep -q before data/value && printf nested-ran";
        let (t, mut p, commit, mut c, mut refs) = fixture(cmd);
        p.timeout_secs = 15;
        let body = if valid {
            r#"{"cells":[{"id":"one"}]}"#
        } else {
            r#"{"cells":[]}"#
        };
        std::fs::write(p.project.join("data/manifest.json"), body).unwrap();
        std::fs::write(
            p.project.join("data/registry.json"),
            r#"{"records":{"one":{"payload":"data/rows.json"}}}"#,
        )
        .unwrap();
        std::fs::write(
            p.project.join("data/rows.json"),
            r#"[{"value":1},{"value":2}]"#,
        )
        .unwrap();
        c.acceptance[0].check = serde_json::from_value(serde_json::json!({"kind":"floor","contract":{
            "kind":"artifact","artifact_path":"data/manifest.json", "typed_verifier_command":cmd,
            "required_universe":true,"universe_fields":["ids"],"cells_field":"cells","cell_identity_fields":["id"],
            "registry_path":"data/registry.json","registry_records_field":"records","registry_key_fields":["id"],
            "data_kind":"record_series","payload_path_field":"payload","payload_format":"json",
            "required_fields":["value"],"non_constant_fields":["value"],"series_value_fields":["value"]
        }})).unwrap();
        // The required universe is declared by the artifact and must contain one cell.
        let mut manifest: serde_json::Value = serde_json::from_str(body).unwrap();
        manifest["ids"] = serde_json::json!(["one"]);
        std::fs::write(
            p.project.join("data/manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        refs[0].kind = AcceptanceCommandKind::NestedVerifier;
        let out = observe_commands(&p, &commit, &c, "chain", &refs, &t.path().join("evidence"))
            .await
            .unwrap();
        assert!(out.checks[0].operational_error.is_none(), "{out:?}");
        assert_eq!(out.passed(), valid, "{out:?}");
        assert_eq!(
            String::from_utf8_lossy(&out.checks[0].stdout).contains("nested-ran"),
            valid
        );
    }
}

#[tokio::test]
async fn replaced_target_link_voids_result_before_reuse() {
    let cmd = "test -f input && rm target && mkdir target && test -d target";
    let (t, p, commit, mut c, mut refs) = fixture(cmd);
    let mut next = c.acceptance[0].clone();
    next.id = "AC-X-002".into();
    c.acceptance.push(next);
    let mut next = refs[0].clone();
    next.acceptance_id = "AC-X-002".into();
    refs.push(next);
    let out = observe_commands(&p, &commit, &c, "chain", &refs, &t.path().join("evidence"))
        .await
        .unwrap();
    assert!(!out.passed(), "target substitution was accepted");
    assert!(
        out.checks[0]
            .operational_error
            .as_ref()
            .unwrap()
            .contains("identity")
    );
    assert!(out.checks.iter().skip(1).all(|c| c.exit_code.is_none()));
}

#[tokio::test]
async fn setup_failure_preserves_operational_record_and_live_audit() {
    let (t, mut p, commit, c, refs) = fixture("test -f input");
    p.project_inputs.push("missing".into());
    let evidence = t.path().join("evidence");
    let _ = observe_commands(&p, &commit, &c, "chain", &refs, &evidence).await;
    let raw =
        std::fs::read(evidence.join("observation.json")).expect("setup failure evidence missing");
    let record: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(record["live_roots_unchanged"], true);
    assert!(record["operational_errors"].as_array().unwrap().len() > 0);
}

#[tokio::test]
async fn after_audit_failure_preserves_nonpassing_record() {
    let (t, p, commit, mut c, mut refs) = fixture("test -f input");
    let cmd = format!(
        "test -f input && mkfifo '{}'",
        p.project.join("fifo").display()
    );
    if let AcceptanceCheck::Command { command, .. } = &mut c.acceptance[0].check {
        *command = cmd.clone();
    }
    refs[0].command_digest = archon_workflow::task_set_contract::content_digest(cmd.as_bytes());
    let evidence = t.path().join("evidence");
    let _ = observe_commands(&p, &commit, &c, "chain", &refs, &evidence).await;
    let record: serde_json::Value = serde_json::from_slice(
        &std::fs::read(evidence.join("observation.json")).expect("audit failure evidence missing"),
    )
    .unwrap();
    assert_eq!(record["live_roots_unchanged"], false);
    assert!(record["operational_errors"].as_array().unwrap().len() > 0);
}

#[tokio::test]
async fn each_check_records_cache_identity_and_project_mutations() {
    let (t, p, commit, c, refs) = fixture("test -f data/value && printf changed > data/value");
    let out = observe_commands(&p, &commit, &c, "chain", &refs, &t.path().join("evidence"))
        .await
        .unwrap();
    let record = serde_json::to_value(out).unwrap();
    assert!(record["check_evidence"][0]["before_identity"].is_object());
    assert!(
        record["check_evidence"][0]["changed_project_paths"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("data/value"))
    );
}

#[tokio::test]
async fn cancelled_observation_does_not_create_a_worktree() {
    use std::sync::{Arc,atomic::AtomicBool};
    let (t,p,commit,c,refs)=fixture("test -f input");
    let out=archon_workflow::acceptance_scratch::observe_commands_cancellable(
        &p,&commit,&c,"chain",&refs,&t.path().join("evidence"),Arc::new(AtomicBool::new(true))
    ).await.unwrap();
    assert!(!p.scratch_parent.exists(),"cancelled setup still created scratch storage");
    assert!(!out.passed());
}

#[tokio::test]
async fn hanging_checkout_is_bounded_and_records_cleanup() {
    use std::process::Command;
    let (t,mut p,_,c,refs)=fixture("test -f input");
    let git=|args:&[&str]| { let o=Command::new("git").arg("-C").arg(&p.repository).args(args).output().unwrap();assert!(o.status.success());String::from_utf8(o.stdout).unwrap().trim().to_string() };
    std::fs::write(p.repository.join(".gitattributes"),"input filter=stall\n").unwrap();
    git(&["add",".gitattributes"]);git(&["commit","-qm","checkout filter fixture"]);
    let commit=git(&["rev-parse","HEAD"]);
    git(&["config","filter.stall.smudge","sleep 30; cat"]);
    p.timeout_secs=1;
    let start=std::time::Instant::now();
    let out=observe_commands(&p,&commit,&c,"chain",&refs,&t.path().join("evidence")).await.unwrap();
    assert!(start.elapsed()<std::time::Duration::from_secs(8),"checkout exceeded observation setup bound");
    assert!(!out.passed());
    assert!(out.teardown_verified,"{:?}",out.cleanup_error);
    assert!(out.operational_errors.iter().any(|e|e.contains("deadline")),"{:?}",out.operational_errors);
}
