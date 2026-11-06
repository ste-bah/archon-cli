use archon_workflow::acceptance_world::{AcceptanceCommandKind, FrozenCommandRef, resolve_command};
use archon_workflow::task_set_contract::{AcceptanceContract, content_digest};

fn contract() -> AcceptanceContract {
    serde_json::from_value(serde_json::json!({
        "schema_version":1,"prd":{"path":"p.md","digest":"d"},
        "gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},
        "acceptance":[{"id":"AC-X-001","criterion":"output is correct",
        "check":{"kind":"command","command":"./verify-output","cwd":"project_root"},
        "judgment":{"verdict":"accepted","counterexample":"missing output","reason":"rejects missing output","host_call_id":"judge-1"}}]
    })).unwrap()
}
fn reference() -> FrozenCommandRef {
    FrozenCommandRef {
        acceptance_id: "AC-X-001".into(),
        kind: AcceptanceCommandKind::Command,
        chain_digest: "bound-chain".into(),
        command_digest: content_digest(b"./verify-output"),
    }
}
#[test]
fn command_is_selected_from_validated_contract_not_caller_text() {
    let authorized = resolve_command(&contract(), "bound-chain", &reference()).unwrap();
    assert_eq!(authorized.bytes(), b"./verify-output");
}
#[test]
fn mismatched_digest_unknown_id_and_refuted_verdict_refuse() {
    let c = contract();
    let mut r = reference();
    r.command_digest = content_digest(b"other");
    assert!(resolve_command(&c, "bound-chain", &r).is_err());
    r = reference();
    r.acceptance_id = "AC-X-002".into();
    assert!(resolve_command(&c, "bound-chain", &r).is_err());
    assert!(resolve_command(&c, "changed-chain", &reference()).is_err());
    let mut c = c;
    c.acceptance[0].judgment.verdict = archon_workflow::task_set_contract::JudgeDecision::Refuted;
    assert!(resolve_command(&c, "bound-chain", &reference()).is_err());
}
#[test]
fn nested_verifier_is_authorized_but_unbound_residual_is_not() {
    let mut c = contract();
    c.acceptance[0].check=serde_json::from_value(serde_json::json!({
        "kind":"floor","contract":{"kind":"artifact","artifact_path":"out.json","typed_verifier_command":"./verify-output"}
    })).unwrap();
    let mut r = reference();
    r.kind = AcceptanceCommandKind::NestedVerifier;
    assert_eq!(
        resolve_command(&c, "bound-chain", &r).unwrap().bytes(),
        b"./verify-output"
    );
    r.kind = AcceptanceCommandKind::ResidualFailClosed;
    assert!(resolve_command(&c, "bound-chain", &r).is_err());
}

#[test]
fn execution_gate_shares_policy_and_run_bindings() {
    use archon_workflow::acceptance_world::{
        execution_policy_defects, resolve_command_with_bindings,
    };
    use archon_workflow::task_set_contract::{
        AcceptanceCheck, TrustedCwd, acceptance_policy_findings,
    };
    for bound in [&[][..], &["INPUT_DATA"][..]] {
        let mut c = contract();
        let command = "set -u; test -f \"$INPUT_DATA\"";
        c.acceptance[0].check = AcceptanceCheck::Command {
            command: command.into(),
            cwd: TrustedCwd::ProjectRoot,
        };
        let mut r = reference();
        r.command_digest = content_digest(command.as_bytes());
        // The accepted check induces a judgment contradiction, but the host
        // still runs it so expansion defects can be repaired in the round.
        assert!(acceptance_policy_findings(&c).iter().any(|f| {
            f.identity
                .as_ref()
                .is_some_and(|i| i.code == "contradictory_judgment")
        }));
        let findings =
            archon_workflow::task_set_contract::acceptance_entry_policy_findings_with_bindings(
                &c.acceptance[0],
                bound,
            );
        assert_eq!(
            findings.iter().any(|f| f
                .identity
                .as_ref()
                .is_some_and(|i| i.code == "unset_shell_variable")),
            bound.is_empty()
        );
        assert!(execution_policy_defects(&c.acceptance[0], bound).is_empty());
        assert!(resolve_command_with_bindings(&c, "bound-chain", &r, bound).is_ok());
    }
    let mut c = contract();
    c.acceptance[0].check = serde_json::from_value(serde_json::json!({
        "kind":"floor","contract":{"kind":"artifact","artifact_path":"out.json"}
    }))
    .unwrap();
    assert!(!execution_policy_defects(&c.acceptance[0], &["INPUT_DATA"]).is_empty());
    let mut r = reference();
    r.kind = AcceptanceCommandKind::NestedVerifier;
    assert!(resolve_command_with_bindings(&c, "bound-chain", &r, &["INPUT_DATA"]).is_err());
}
