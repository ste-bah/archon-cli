use super::*;
use archon_workflow::llm_client_port::WorkflowLlmClient;
use std::sync::Arc;

#[tokio::test]
async fn unchanged_parse_finding_key_survives_a_sibling_defect_567() {
    use crate::command::workflow_task_set::prepare_acceptance_freeze_from_candidate;
    use crate::command::workflow_task_set::republish::test_fixture::frozen_set;
    let set = frozen_set(&[
        ("AC-X-001", "./verify-output\nif then\n", true),
        ("AC-X-002", "./verify-output", true),
    ]);
    let original: serde_json::Value = serde_json::from_slice(&set.contract_bytes()).unwrap();
    // A mechanical refusal must never call this client.
    struct NoJudge;
    #[async_trait::async_trait]
    impl WorkflowLlmClient for NoJudge {
        async fn send_message(
            &self,
            _: Vec<serde_json::Value>,
            _: Vec<serde_json::Value>,
            _: Vec<serde_json::Value>,
            _: &str,
        ) -> archon_workflow::error::WorkflowResult<
            archon_workflow::llm_client_port::WorkflowAgentOutcome,
        > {
            panic!("parse-only candidate must not reach judge")
        }
    }
    let mut findings = Vec::new();
    for sibling_defect in [false, true] {
        let mut candidate = original.clone();
        // Keep both checks unparseable so this test needs no model or execution.
        candidate["acceptance"][1]["check"]["command"] = "if then".into();
        if sibling_defect {
            candidate["acceptance"][1]["check"] = serde_json::json!({"kind":"floor", "contract": {
                "kind":"artifact", "artifact_path":"out.json", "artifact_format":"", "required_true_fields":[],
                "typed_verifier_command":"if then"
            }});
        }
        let prepared = prepare_acceptance_freeze_from_candidate(
            set.project.path(),
            &set.tasks,
            &set.prd,
            archon_core::config::GateMode::Observe,
            serde_json::to_vec(&candidate).unwrap(),
            Arc::new(NoJudge),
        )
        .await;
        let finding = if sibling_defect {
            let error = prepared.unwrap_err();
            let staging = set.project.path().join("refusal");
            std::fs::create_dir_all(&staging).unwrap();
            let envelope = staging.join("gate.json");
            refuse_candidate_error(
                set.project.path(),
                StagedArgs {
                    staging_root: &staging,
                    gate_envelope: &envelope,
                    call_id: "call",
                },
                "freeze-acceptance",
                crate::command::workflow_gate::GateId::FreezeAcceptance,
                "acceptance",
                &error,
            )
            .unwrap();
            let value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(envelope).unwrap()).unwrap();
            value["policy_findings"]
                .as_array()
                .unwrap()
                .iter()
                .find(|finding| finding["deterministic_defect"]["subject"] == "AC-X-001")
                .unwrap()
                .clone()
        } else {
            let prepared = prepared.unwrap();
            let own = prepared
                .findings
                .iter()
                .find(|finding| {
                    finding.subject == "AC-X-001"
                        && finding
                            .deterministic_defect
                            .as_ref()
                            .is_some_and(|defect| defect.code == "shell_parse")
                })
                .expect("the pre-judge lint finding");
            serde_json::json!({"subject":own.subject, "deterministic_defect":own.deterministic_defect,
                "gate_stage":own.gate_stage, "judged_item":own.judged_item})
        };
        assert_eq!(finding["subject"], "AC-X-001");
        assert_eq!(finding["deterministic_defect"]["code"], "shell_parse");
        assert!(finding["gate_stage"].is_null());
        findings.push(finding);
    }
    // These are every host field openFindingKey reads. Diagnostics and paths are irrelevant.
    for field in [
        "subject",
        "deterministic_defect",
        "gate_stage",
        "judged_item",
    ] {
        assert_eq!(findings[0][field], findings[1][field], "{field}");
    }
    let output = std::process::Command::new("node")
        .args([
            "-e",
            r#"
const fs = require('node:fs'), vm = require('node:vm'), assert = require('node:assert/strict');
const ctx = vm.createContext({});
for (const name of ['workflow_decompose_v1_progress.js', 'workflow_decompose_v1_failing_set.js'])
  vm.runInContext(fs.readFileSync(process.argv[1] + '/' + name, 'utf8'), ctx);
const findings = JSON.parse(process.argv[2]);
assert.equal(ctx.openFindingKey(findings[0]), ctx.openFindingKey(findings[1]));
"#,
        ])
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/src/command"))
        .arg(serde_json::to_string(&findings).unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
