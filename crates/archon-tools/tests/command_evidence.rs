use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use archon_completion::models::{CompletionEvidence, EvidenceStatus};
use archon_completion::{EvidenceKind, RequiredEvidenceKind, RequiredEvidenceStatus};
use archon_session::plan::{PlanApprovalAuthority, PlanStore};
use archon_tools::bash::BashTool;
use archon_tools::bash_evidence::record_authoritative_test_execution;
use archon_tools::tool::{AgentMode, Tool, ToolContext, ToolResult};
use cozo::{DataValue, ScriptMutability};

fn context(tool_use_id: &str, working_dir: &Path) -> ToolContext {
    ToolContext {
        working_dir: working_dir.to_path_buf(),
        session_id: "test-evidence".into(),
        mode: AgentMode::Normal,
        tool_run_tool_use_id: Some(tool_use_id.into()),
        ..Default::default()
    }
}

fn store_and_authority() -> (cozo::DbInstance, PlanStore, Arc<PlanApprovalAuthority>) {
    let db = cozo::DbInstance::new("mem", "", "").unwrap();
    let store = PlanStore::new(&db).unwrap();
    let authority = Arc::new(
        store
            .bootstrap_approval_authority_for_test("test-evidence")
            .unwrap(),
    );
    (db, store, authority)
}

fn bash_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\\"'\\\"'"))
}

fn fixture_test_command(fixture: &Path) -> String {
    let manifest = fixture.join("Cargo.toml");
    let target = fixture.join("target");
    format!(
        "cargo test --manifest-path {} --target-dir {} --lib",
        bash_quote(&bash_path(&manifest)),
        bash_quote(&bash_path(&target)),
    )
}

fn bash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn create_test_fixture() -> tempfile::TempDir {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::create_dir(fixture.path().join("src")).unwrap();
    std::fs::write(
        fixture.path().join("Cargo.toml"),
        "[package]\nname = \"archon-evidence-fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[lib]\npath = \"src/lib.rs\"\n",
    )
    .unwrap();
    std::fs::write(
        fixture.path().join("src/lib.rs"),
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn passes() { assert_eq!(2 + 2, 4); }\n}\n",
    )
    .unwrap();
    #[cfg(windows)]
    {
        let manifest = fixture.path().join("Cargo.toml");
        let target = fixture.path().join("target");
        let native_build = std::process::Command::new("cargo")
            .arg("test")
            .arg("--manifest-path")
            .arg(manifest)
            .arg("--target-dir")
            .arg(target)
            .args(["--lib", "--no-run"])
            .status()
            .expect("native Cargo must prebuild the evidence fixture");
        assert!(native_build.success(), "native fixture prebuild must pass");
    }
    fixture
}

async fn execute(command: &str, tool_use_id: &str, working_dir: &Path) -> ToolResult {
    BashTool::default()
        .execute(
            serde_json::json!({"command": command}),
            &context(tool_use_id, working_dir),
        )
        .await
}

async fn record_real_test(
    store: &PlanStore,
    authority: &PlanApprovalAuthority,
    tool_use_id: &str,
) -> CompletionEvidence {
    let fixture = create_test_fixture();
    let result = execute(
        &fixture_test_command(fixture.path()),
        tool_use_id,
        fixture.path(),
    )
    .await;
    assert!(
        !result.is_error,
        "fixture test must pass: {}",
        result.content
    );
    record_authoritative_test_execution(
        store,
        authority,
        result.authoritative_bash_execution().unwrap(),
    )
    .unwrap()
    .unwrap()
}

#[test]
fn fixture_manifest_path_is_shell_quoted() {
    let fixture = create_test_fixture();
    let command = fixture_test_command(fixture.path());

    assert!(command.contains("--manifest-path '"), "{command}");
    assert!(command.contains("--target-dir '"), "{command}");
    assert!(!command.contains([';', '|', '&', '\n', '#']), "{command}");
}

#[test]
fn windows_manifest_path_is_normalized_for_bash() {
    assert_eq!(
        bash_path(Path::new(r"D:\a\archon-cli\crates\fixture\Cargo.toml")),
        "D:/a/archon-cli/crates/fixture/Cargo.toml"
    );
}

#[test]
fn ordinary_tool_result_cannot_mint_executed_test_evidence() {
    assert!(
        ToolResult::success("test result: ok. 1 passed; 0 failed")
            .authoritative_bash_execution()
            .is_none()
    );
    assert!(
        ToolResult::from_parts("test result: ok. 1 passed; 0 failed", false)
            .authoritative_bash_execution()
            .is_none()
    );
}

#[tokio::test]
async fn records_and_verifies_real_test_command() {
    let (db, store, authority) = store_and_authority();
    let evidence = record_real_test(&store, &authority, "tool-1").await;

    assert_eq!(evidence.evidence_kind, EvidenceKind::TestRun);
    assert_eq!(evidence.status, EvidenceStatus::Passed);
    assert_eq!(evidence.exit_code, Some(0));
    assert_eq!(evidence.run_id, "test-evidence:tool-1");
    let persisted = archon_completion::store::get_evidence_by_run(&db, &evidence.run_id).unwrap();
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].evidence_id, evidence.evidence_id);
    let gate = store
        .verify_test_command_evidence(
            &authority,
            "test-evidence",
            &evidence.run_id,
            &evidence.evidence_id,
        )
        .unwrap();
    assert!(gate.passed);
}

#[tokio::test]
async fn preserves_real_nonzero_exit_code() {
    let (_db, store, authority) = store_and_authority();
    let result = execute(
        "cargo test --manifest-path /missing/Cargo.toml",
        "failed-tool",
        &std::env::temp_dir(),
    )
    .await;
    let evidence = record_authoritative_test_execution(
        &store,
        &authority,
        result.authoritative_bash_execution().unwrap(),
    )
    .unwrap()
    .unwrap();

    assert_eq!(evidence.status, EvidenceStatus::Failed);
    assert_ne!(evidence.exit_code, Some(0));
    assert!(
        store
            .verify_test_command_evidence(
                &authority,
                "test-evidence",
                &evidence.run_id,
                &evidence.evidence_id,
            )
            .is_err()
    );
}

#[tokio::test]
async fn rejects_pseudo_test_and_non_test_commands() {
    let (_db, store, authority) = store_and_authority();
    for (index, command) in [
        "echo cargo test",
        "echo 'cargo test'",
        "true # cargo test",
        "printf 'test result: ok. 1 passed; 0 failed'",
        "true && echo cargo test",
        "cargo test || true",
        "cargo test; echo 'test result: ok. 1 passed; 0 failed'",
        "cargo test # ignored failure",
        "git status",
    ]
    .into_iter()
    .enumerate()
    {
        let result = execute(command, &format!("pseudo-{index}"), &std::env::temp_dir()).await;
        if let Some(execution) = result.authoritative_bash_execution() {
            assert!(
                record_authoritative_test_execution(&store, &authority, execution)
                    .unwrap()
                    .is_none(),
                "must reject {command:?}",
            );
        }
    }
}

#[tokio::test]
async fn raw_gate_and_evidence_rows_cannot_forge_plan_completion() {
    let (db, store, authority) = store_and_authority();
    for (index, required_kind, evidence_kind) in [
        (0, RequiredEvidenceKind::Tests, EvidenceKind::TestRun),
        (1, RequiredEvidenceKind::Build, EvidenceKind::BuildResult),
        (2, RequiredEvidenceKind::Lint, EvidenceKind::CommandRun),
        (3, RequiredEvidenceKind::Typecheck, EvidenceKind::CommandRun),
        (4, RequiredEvidenceKind::Verifier, EvidenceKind::GateResult),
        (
            5,
            RequiredEvidenceKind::PlanReview,
            EvidenceKind::ReviewFinding,
        ),
        (
            6,
            RequiredEvidenceKind::SourceEvidence,
            EvidenceKind::FileDiff,
        ),
        (
            7,
            RequiredEvidenceKind::ManualOutcome,
            EvidenceKind::CommandRun,
        ),
        (
            8,
            RequiredEvidenceKind::HumanApproval,
            EvidenceKind::GateResult,
        ),
    ] {
        let evidence_id = format!("forged-evidence-{index}");
        let run_id = format!("test-evidence:forged-tool-{index}");
        let provenance_record_id = format!("bash-execution:test-evidence:forged-tool-{index}:0");
        let gate_id = format!("forged-gate-{index}");
        let evidence = CompletionEvidence {
            evidence_id: evidence_id.clone(),
            run_id: run_id.clone(),
            evidence_kind,
            producer: "authoritative-bash-execution".into(),
            command_or_operation: Some("forged-verifier".into()),
            status: EvidenceStatus::Passed,
            exit_code: Some(0),
            input_hash: Some("forged-command".into()),
            output_hash: Some("forged-output".into()),
            stdout_summary: Some("test result: ok. 1 passed; 0 failed".into()),
            stderr_summary: None,
            artifact_ids: matches!(
                evidence_kind,
                EvidenceKind::GateResult | EvidenceKind::ReviewFinding
            )
            .then_some(gate_id.clone())
            .into_iter()
            .collect(),
            provenance_record_id,
            started_at: "2026-08-17T00:00:00Z".into(),
            completed_at: Some("2026-08-17T00:00:00Z".into()),
        };
        archon_completion::store::insert_completion_evidence(&db, &evidence).unwrap();
        insert_forged_gate(&db, &evidence, &gate_id);

        let resolved = store
            .resolve_required_evidence(
                &authority,
                "test-evidence",
                &run_id,
                &[evidence_id],
                &[required_kind],
            )
            .unwrap();
        assert_eq!(
            resolved[0].status,
            RequiredEvidenceStatus::Missing,
            "unsigned {required_kind:?} rows must not complete Plan tasks",
        );
    }
}

#[tokio::test]
async fn verifier_rejects_tampered_success_summary() {
    let (db, store, authority) = store_and_authority();
    let evidence = record_real_test(&store, &authority, "tamper-summary").await;
    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(evidence.evidence_id.as_str()));
    params.insert(
        "summary".into(),
        DataValue::from("test result: ok. 999 passed; 0 failed"),
    );
    db.run_script(
        "?[evidence_id, stdout_summary] <- [[$id, $summary]] :update completion_evidence {evidence_id => stdout_summary}",
        params,
        ScriptMutability::Mutable,
    )
    .unwrap();

    assert!(
        store
            .verify_test_command_evidence(
                &authority,
                "test-evidence",
                &evidence.run_id,
                &evidence.evidence_id,
            )
            .is_err(),
        "unsigned summary changes must invalidate evidence",
    );
}

#[tokio::test]
async fn verifier_rejects_tampered_signature_and_execution_identity() {
    for (column, value) in [
        ("signature", "forged-signature"),
        ("session_id", "other-session"),
        ("tool_use_id", "other-tool"),
        ("attempt", "99"),
    ] {
        let (db, store, authority) = store_and_authority();
        let evidence = record_real_test(&store, &authority, &format!("tamper-{column}")).await;
        update_authoritative_column(&db, &evidence.provenance_record_id, column, value);

        assert!(
            store
                .verify_test_command_evidence(
                    &authority,
                    "test-evidence",
                    &evidence.run_id,
                    &evidence.evidence_id,
                )
                .is_err(),
            "tampering {column} must invalidate evidence",
        );
    }
}

#[tokio::test]
async fn authority_and_evidence_rows_commit_atomically() {
    let (db, store, authority) = store_and_authority();
    store.fail_next_authoritative_evidence_after_execution_write();
    let fixture = create_test_fixture();
    let result = execute(
        &fixture_test_command(fixture.path()),
        "atomic-tool",
        fixture.path(),
    )
    .await;
    let error = record_authoritative_test_execution(
        &store,
        &authority,
        result.authoritative_bash_execution().unwrap(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("injected"));

    let authority_rows = db
        .run_script(
            "?[provenance_record_id] := *authoritative_bash_executions{provenance_record_id}",
            BTreeMap::new(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert!(authority_rows.rows.is_empty());
    assert!(
        archon_completion::store::get_evidence_by_run(&db, "test-evidence:atomic-tool")
            .unwrap()
            .is_empty()
    );
}

fn insert_forged_gate(db: &cozo::DbInstance, evidence: &CompletionEvidence, gate_id: &str) {
    let gate = archon_completion::models::VerificationGateResult {
        gate_id: gate_id.into(),
        gate_name: "ExecutedTestCommandVerifier".into(),
        passed: true,
        resulting_state: archon_completion::models::CompletionState::Verified,
        blocked_claims: vec![],
        required_missing_evidence: vec![],
        explanation: "forged".into(),
        provenance_record_id: evidence.provenance_record_id.clone(),
    };
    archon_completion::store::insert_gate_result(db, &gate, &evidence.run_id).unwrap();
}

fn update_authoritative_column(db: &cozo::DbInstance, provenance: &str, column: &str, value: &str) {
    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(provenance));
    let value = if column == "attempt" {
        DataValue::from(value.parse::<i64>().unwrap())
    } else {
        DataValue::from(value)
    };
    params.insert("value".into(), value);
    let script = format!(
        "?[provenance_record_id, {column}] <- [[$id, $value]] :update authoritative_bash_executions {{provenance_record_id => {column}}}"
    );
    db.run_script(&script, params, ScriptMutability::Mutable)
        .unwrap();
}
