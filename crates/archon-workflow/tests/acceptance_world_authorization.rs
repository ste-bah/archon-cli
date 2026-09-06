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
    FrozenCommandRef { acceptance_id:"AC-X-001".into(), kind:AcceptanceCommandKind::Command,
        chain_digest:"bound-chain".into(), command_digest:content_digest(b"./verify-output") }
}
#[test]
fn command_is_selected_from_validated_contract_not_caller_text() {
    let authorized = resolve_command(&contract(), "bound-chain", &reference()).unwrap();
    assert_eq!(authorized.bytes(), b"./verify-output");
}
#[test]
fn mismatched_digest_unknown_id_and_refuted_verdict_refuse() {
    let c=contract();
    let mut r=reference();r.command_digest=content_digest(b"other");
    assert!(resolve_command(&c,"bound-chain",&r).is_err());
    r=reference();r.acceptance_id="AC-X-002".into();
    assert!(resolve_command(&c,"bound-chain",&r).is_err());
    assert!(resolve_command(&c,"changed-chain",&reference()).is_err());
    let mut c=c;c.acceptance[0].judgment.verdict=archon_workflow::task_set_contract::JudgeDecision::Refuted;
    assert!(resolve_command(&c,"bound-chain",&reference()).is_err());
}
#[test]
fn nested_verifier_is_authorized_but_unbound_residual_is_not() {
    let mut c=contract();c.acceptance[0].check=serde_json::from_value(serde_json::json!({
        "kind":"floor","contract":{"kind":"artifact","artifact_path":"out.json","typed_verifier_command":"./verify-output"}
    })).unwrap();
    let mut r=reference();r.kind=AcceptanceCommandKind::NestedVerifier;
    assert_eq!(resolve_command(&c,"bound-chain",&r).unwrap().bytes(),b"./verify-output");
    r.kind=AcceptanceCommandKind::ResidualFailClosed;
    assert!(resolve_command(&c,"bound-chain",&r).is_err());
}
