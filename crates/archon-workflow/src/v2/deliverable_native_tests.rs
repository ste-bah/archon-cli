//! Library tests inspect emitted text and evaluate its declared inputs natively.
use super::deliverable_contract::{ContractRoots, set_verifier_program};
use serde_json::Value;

pub(super) fn initialize() {
    set_verifier_program(Ok("native-test-program".into()));
}

pub(super) fn command(roots: &ContractRoots, contract: &Value) -> String {
    initialize();
    super::deliverable_contract::verification_command(roots, contract)
}

pub(super) fn run(command: &str) -> std::process::Output {
    if !command.contains("'verify-deliverable'") {
        // Contract defects are the existing fail-closed shell form.
        return std::process::Command::new(archon_shell::resolve_posix_shell())
            .arg("-c")
            .arg(command)
            .output()
            .unwrap();
    }
    let (line, after) = command.split_once('\n').unwrap_or((command, ""));
    let line = line.strip_suffix(" || exit $?").unwrap_or(line);
    let words = words(line);
    assert_eq!(&words[..2], &["printf", "%s"]);
    assert_eq!(
        &words[3..7],
        &["|", "native-test-program", "workflow", "verify-deliverable"]
    );
    let mut roots = Vec::new();
    let mut index = 7;
    while words[index] == "--root" {
        roots.push(words[index + 1].clone());
        index += 2;
    }
    assert_eq!(&words[index..], &["--contract-json", "-"]);
    let raw: Value = serde_json::from_str(&words[2]).unwrap();
    let contract = match serde_json::from_value(raw) {
        Ok(contract) => contract,
        Err(error) => {
            return output(
                false,
                serde_json::json!({"status":"failed","failure_count":1,"failures":[format!("declared deliverable contract does not parse: {error}")]}),
                Vec::new(),
            );
        }
    };
    let roots = ContractRoots::from_ordered(roots).unwrap();
    let facts = crate::collect_declarative_floor_facts(&roots, &contract).unwrap();
    let verdict = crate::evaluate_floor_predicates(&contract, &facts);
    let passed = matches!(verdict, crate::FloorVerdict::Passed);
    let value = super::declarative_floor_output::verdict_value(&contract, &facts, verdict);
    let mut verdict = output(passed, value, Vec::new());
    if passed && !after.is_empty() {
        let typed = std::process::Command::new(archon_shell::resolve_posix_shell())
            .arg("-c")
            .arg(after)
            .output()
            .unwrap();
        verdict.stdout.push(b'\n');
        verdict.stdout.extend(typed.stdout);
        verdict.stderr.extend(typed.stderr);
        verdict.status = typed.status;
    }
    verdict
}

fn output(passed: bool, value: Value, mut stdout: Vec<u8>) -> std::process::Output {
    let status = std::process::Command::new(archon_shell::resolve_posix_shell())
        .arg("-c")
        .arg(if passed { "exit 0" } else { "exit 1" })
        .status()
        .unwrap();
    stdout.extend(serde_json::to_vec_pretty(&value).unwrap());
    std::process::Output {
        status,
        stdout,
        stderr: Vec::new(),
    }
}

fn words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    for ch in line.chars() {
        match (quote, ch) {
            (Some(q), ch) if q == ch => quote = None,
            (None, '\'' | '"') => quote = Some(ch),
            (None, ch) if ch.is_whitespace() => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            _ => word.push(ch),
        }
    }
    assert!(quote.is_none());
    if !word.is_empty() {
        words.push(word);
    }
    words
}

#[test]
fn adapter_keeps_missing_kind_and_executes_typed_commands() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("out.json"), "{}").unwrap();
    let roots = ContractRoots::project_only(directory.path().to_string_lossy());
    let missing = run(&command(
        &roots,
        &serde_json::json!({"artifact_path":"out.json"}),
    ));
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stdout).contains("missing field `kind`"));
    let marker = directory.path().join("typed-marker");
    let typed = format!(
        "printf ran > {}; exit 3",
        crate::acceptance_scratch::shell_arg(&marker)
    );
    let failed = run(&command(
        &roots,
        &serde_json::json!({"kind":"evidence","artifact_path":"out.json","typed_verifier_command":typed}),
    ));
    assert_eq!(failed.status.code(), Some(3));
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "ran");
}

#[test]
fn presence_metadata_is_excluded_but_unknown_declarations_still_fail() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("out.json"), "{}").unwrap();
    let roots = ContractRoots::project_only(directory.path().to_string_lossy());
    let mut contract = serde_json::json!({
        "kind":"evidence", "artifact_path":"out.json",
        "artifact_status":{"exists":false,"bytes":0}
    });
    let passed = run(&command(&roots, &contract));
    assert!(passed.status.success(), "{passed:?}");
    contract["unexpected_predicate"] = Value::Bool(true);
    let failed = run(&command(&roots, &contract));
    assert!(!failed.status.success());
    assert!(
        String::from_utf8_lossy(&failed.stdout).contains("unknown field `unexpected_predicate`")
    );
}

#[test]
fn presence_metadata_cannot_make_an_empty_deliverable_pass() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("out.json"), "").unwrap();
    let roots = ContractRoots::project_only(directory.path().to_string_lossy());
    let contract = serde_json::json!({
        "kind":"evidence", "artifact_path":"out.json",
        "artifact_status":{"exists":true,"bytes":123}
    });
    let failed = run(&command(&roots, &contract));
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stdout).contains("missing or empty"));
}
