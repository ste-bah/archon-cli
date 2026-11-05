use super::*;
use serde_json::json;

#[test]
fn native_author_gate_refuses_unset_reads_before_publication() {
    for check in [
        json!({"kind":"command", "command":"set -eu\ngrep -q x \"$UNSETVAR-x\" || true", "cwd":"project_root"}),
        json!({"kind":"floor", "contract":{"kind":"json", "artifact_path":"output.json", "typed_verifier_command":"set -u\necho ${UNSETVAR}"}}),
    ] {
        let entry = json!({"id":"A", "criterion":"output is valid", "check":check});
        let defects = entry_defects(&entry.to_string(), &[]).unwrap();
        let found = defects
            .iter()
            .find(|d| d.identity.code == "unset_shell_variable")
            .expect("native gate must refuse the read");
        assert!(found.message.contains("line 2"));
        assert!(found.message.contains("UNSETVAR"));
        assert!(refusal_text("A", found).contains("acceptance entry 'A' was refused"));
    }
}

#[test]
fn native_author_gate_allows_bound_reads() {
    let entry = json!({"id":"A", "criterion":"output is valid", "check":{
        "kind":"command", "command":"set -eu\nUNSETVAR=output\ntest -f \"$UNSETVAR\"", "cwd":"project_root"
    }});
    assert!(
        entry_defects(&entry.to_string(), &[])
            .unwrap()
            .iter()
            .all(|d| d.identity.code != "unset_shell_variable")
    );
}

#[test]
fn native_author_gate_uses_the_supplied_check_policy_names() {
    let entry = json!({"id":"A", "criterion":"output is valid", "check":{
        "kind":"command", "command":"set -eu\ntest -s \"$INPUT_DATA\"", "cwd":"project_root"
    }});
    let text = entry.to_string();
    assert!(
        entry_defects(&text, &[])
            .unwrap()
            .iter()
            .any(|d| d.identity.code == "unset_shell_variable")
    );
    assert!(
        entry_defects_with_bindings(&text, &[], &["INPUT_DATA"])
            .unwrap()
            .iter()
            .all(|d| d.identity.code != "unset_shell_variable")
    );
}
