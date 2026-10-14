use super::*;
use serde_json::json;

fn resolve(raw: &str) -> String {
    let mut entry = json!({"id":"EVIDENCE","check":{"kind":"command","command_block":true}});
    assert_eq!(resolve_command_block(&mut entry, raw), None);
    entry["check"]["command"].as_str().unwrap().to_owned()
}

fn evidence(case: &str, before: &str, after: &str, verdict: &str) {
    eprintln!(
        "EVIDENCE #423 {case} BEFORE: raw_block_bytes={before:?} AFTER: frozen_bytes={after:?} {verdict}"
    );
}

#[test]
fn evidence_423_a_crlf_is_frozen_as_byte_equal_lf_and_matches_javascript() {
    let raw = "```check\r\nset -e\r\nprintf ok\r\n```";
    let expected = "set -e\nprintf ok";
    let stored = resolve(raw);
    let js_source = include_str!("workflow_decompose_reply_blocks.js");
    let script = format!(
        "{js_source}\nconst reply = {}; const entry = {{id:'EVIDENCE',check:{{kind:'command',command_block:true}}}}; const refusal = resolveCommandBlock(entry, reply); if (refusal) throw new Error(refusal); process.stdout.write(entry.check.command);",
        serde_json::to_string(raw).unwrap()
    );
    let output = std::process::Command::new("node")
        .arg("-e")
        .arg(script)
        .output()
        .expect("Node is the repository's JS parser harness and must run the resolver");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let js_stored = String::from_utf8(output.stdout).unwrap();
    evidence("a", raw, &stored, "rust_equals_lf=true js_equals_rust=true");
    assert_eq!(stored.as_bytes(), expected.as_bytes());
    assert_eq!(js_stored.as_bytes(), expected.as_bytes());
}

#[test]
fn evidence_423_b_lone_cr_and_mixed_endings_normalize_without_changing_backslash_r() {
    let raw = "```check\r\nset -e\r\nfirst\nsecond\rprintf 'literal\\r'\n```";
    let expected = "set -e\nfirst\nsecond\nprintf 'literal\\r'";
    let stored = resolve(raw);
    evidence(
        "b",
        raw,
        &stored,
        "all_line_endings_lf=true quoted_backslash_r_preserved=true",
    );
    assert_eq!(stored.as_bytes(), expected.as_bytes());
    assert!(!stored.as_bytes().windows(2).any(|pair| pair == b"\r\n"));
    assert!(!stored.as_bytes().contains(&b'\r'));
}
