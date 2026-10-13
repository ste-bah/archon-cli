//! Issue 383: the acceptance author's parse refusal redacts its excerpt and
//! the engine message with the host's public-copy rule. The script's rule
//! (`redactSecretShapedText` in `workflow_decompose_v1_acceptance.js`) is held
//! equal to `archon_workflow::events::sanitize_value` here, in the engine the
//! host runs the script in.
use super::shape_tests::script_source;
use serde_json::Value;

/// Runs `body` (an expression returning a JSON string) after the fixed
/// script, in QuickJS as the host runs it.
fn eval_after_script(body: &str) -> Value {
    let source = script_source();
    let runtime = rquickjs::Runtime::new().unwrap();
    let context = rquickjs::Context::full(&runtime).unwrap();
    context.with(|ctx| {
        super::install_entry_validator(&ctx).unwrap();
        let script = [
            "const args = {authorMaxParallelism:1, gateMode:'enforce'};",
            &source,
            body,
        ]
        .join("\n");
        let result: String = ctx.eval(script).unwrap();
        serde_json::from_str(&result).unwrap()
    })
}

const CORPUS: &[&str] = &[
    "",
    "plain words stay as they are",
    "Authorization: Bearer abc def",
    "authorization:x BEARER \"tok\" next",
    "(sk-ant-x) \"password=p\" api_key=1, apikey=2; token=3 access_token=4 refresh_token=5 secret=6",
    "sk-0123456789abcdef sk-0123456789abcdefg",
    "sk-ééééééééé sk-éé",
    "SK-ANT-UPPER Token=Mixed PASSWORD=x",
    "tokens=5 sk-short secretive=1 xtoken=2",
    "{\"a\":\"token=abc\"} run token=abc",
    "tab\tsep\u{a0}nbsp\u{85}nel\u{3000}ideo token=a\u{2028}token=b",
    "\u{feff}token=bom-is-not-whitespace-in-rust",
    "bearer",
    "Bearer",
    "`Bearer` x 'bearer' y",
    "multi\n\nline token=z\r\nend",
];

#[test]
fn the_script_redaction_is_the_host_public_copy_rule_word_for_word() {
    let found = eval_after_script(&format!(
        "JSON.stringify({}.map(text => redactSecretShapedText(text)))",
        serde_json::to_string(CORPUS).unwrap()
    ));
    let found: Vec<String> = serde_json::from_value(found).unwrap();
    for (text, script) in CORPUS.iter().zip(found) {
        let host = archon_workflow::events::sanitize_value(Value::String((*text).to_string()));
        assert_eq!(Value::String(script), host, "{text:?}");
    }
}

#[test]
fn a_quickjs_parse_refusal_carries_no_secret_shaped_word_from_the_reply() {
    let key = format!("sk-ant-api03-{}", "K".repeat(40));
    let reply = format!(
        "{{\"id\":\"A\",\"check\":{{\"kind\":\"command\",\"command\":\"curl -H x-api-key {key} \\q\"}}}}"
    );
    let body = format!(
        "(() => {{ const source = {reply}; try {{ JSON.parse(source); return JSON.stringify('parsed'); }} catch (error) {{ return JSON.stringify({{message: String(error.message), refusal: acceptanceParseRefusal('A', source, error)}}); }} }})()",
        reply = serde_json::to_string(&reply).unwrap()
    );
    let found = eval_after_script(&body);
    let refusal = found["refusal"].as_str().unwrap();
    println!("quickjs message: {}", found["message"]);
    println!("refusal: {refusal}");
    assert!(!refusal.contains(&key), "{refusal}");
    assert!(!refusal.contains("KKKKK"), "{refusal}");
    assert!(refusal.contains("x-api-key <redacted>"), "{refusal}");
    assert!(
        refusal.starts_with("acceptance entry A returned no complete entry: JSON parse error"),
        "{refusal}"
    );
}
