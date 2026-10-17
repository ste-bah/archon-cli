//! Issue 383: the acceptance author's refusal text uses the host's shared
//! public-copy redactor, held equal to `sanitize_value` in QuickJS.
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
    "sk-short",
    "ghp_example",
    "tokens_in=12",
    "Secrets",
    "bearer tokens",
    "for token in a b; do echo \"$token\"; done",
    "{\"a\":\"token=abc\"} run token=abc",
    "tab\tsep\u{a0}nbsp\u{85}nel\u{3000}ideo token=a\u{2028}token=b",
    "\u{feff}token=bom-is-not-whitespace-in-rust",
    "Bearer\u{feff}secret",
    concat!("é", "sk", "-ant-abcdefghijklmnopqrst0000", "中"),
    concat!(
        "é",
        "sk",
        "-abcdefghijklmnopqrst0000",
        "中 ",
        "é",
        "sk",
        "-ant-abcdefghijklmnopqrst0000",
        "中 éauthorization:abc中 éBearer abc中 é",
        "gh",
        "p_abcdefghijklmnopqrstuvwxyz0123456789",
        "中 é",
        "gi",
        "thub_pat_abcdefghijklmnopqrstuvwxyz0123456789",
        "中 éAK",
        "IA0123456789ABCDEF中 éAS",
        "IA0123456789ABCDEF中 éx",
        "oxb-1234567890abcdef中 éeyJabc.def.ghi中"
    ),
    concat!(
        "é{\"type\":\"service_account\",\"private_key\":\"value\"}中 é",
        "-----BEGIN ",
        "PRIVATE KEY-----\\nvalue\\n-----END PRIVATE KEY-----中 é",
        "sk",
        "_live_abcdefghijklmnopqrstuvwx中 é",
        "pk",
        "_test_abcdefghijklmnopqrstuvwx中 ér",
        "k_live_abcdefghijklmnopqrstuvwx中"
    ),
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
fn reverify_event_ids_paths_and_sequence_survive_held_secret_sanitizing() {
    let secret = "held-event-secret-444";
    archon_observability::secret_values::register_values([secret]);
    let before = serde_json::json!({
        "report_id": "reverify-123-report",
        "report_path": "decomposition/reverify/reverify-123-report.json",
        "sequence": 123,
        "message": format!("provider failed with {secret}")
    });
    let after = archon_workflow::events::sanitize_value(before.clone());
    assert_eq!(after["report_id"], before["report_id"]);
    assert_eq!(after["report_path"], before["report_path"]);
    assert_eq!(after["sequence"], before["sequence"]);
    assert_eq!(after["message"], "provider failed with [REDACTED]");
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

#[test]
fn a_held_secret_in_malformed_reply_is_redacted_before_refusal_storage_and_prompt_reuse() {
    let secret = "held-workflow-secret-444";
    archon_observability::secret_values::register_values([secret]);
    let reply = format!(r#"{{"id":"A","check":{{"command":"{secret} \q"}}}}"#);
    let body = format!(
        "(() => {{ const source = {reply}; try {{ JSON.parse(source); return 'unexpected parse'; }} catch (error) {{ const raw = acceptanceParseRefusal('A', source, error); return malformedAcceptanceFailure('A', raw, 'parse-error').failure.findings[0].text; }} }})()",
        reply = serde_json::to_string(&reply).unwrap()
    );
    let stored_refusal: String = {
        let source = script_source();
        let runtime = rquickjs::Runtime::new().unwrap();
        let context = rquickjs::Context::full(&runtime).unwrap();
        context.with(|ctx| {
            super::install_entry_validator(&ctx).unwrap();
            let script = [
                "const args = {authorMaxParallelism:1, gateMode:'enforce'};",
                source.as_str(),
                body.as_str(),
            ]
            .join("\n");
            ctx.eval(script)
                .unwrap_or_else(|error| panic!("held-secret refusal evaluation failed: {error:?}"))
        })
    };
    assert!(!stored_refusal.contains(secret), "{stored_refusal}");
    assert!(stored_refusal.contains("REDACTED"), "{stored_refusal}");
    let prompt = format!("Repair this refusal: {stored_refusal}");
    assert!(!prompt.contains(secret), "{prompt}");
}

#[test]
fn a_held_secret_in_validator_refusal_is_redacted_before_state_and_return() {
    let secret = "held-validator-secret-444";
    archon_observability::secret_values::register_values([secret]);
    let body = format!(
        "args.acceptanceCriteria = {{A:'criterion'}}; catalogueText = () => ''; priorText = () => ''; __archonValidateAcceptanceEntry = () => JSON.stringify([{{text:{secret}, subject:'A'}}]); const state = {{entries:new Map(), retryIds:new Set(['A'])}}; const w = {{agent:async () => ({{status:'accepted', stopReason:'end_turn', content:JSON.stringify({{id:'A'}})}})}}; try {{ const out = await authorAcceptanceEntries(w, 'author', 1, state); return JSON.stringify({{out, stored:state.refusals.get('A')}}); }} catch (error) {{ return JSON.stringify({{error:String(error), stack:error.stack}}); }}",
        secret = serde_json::to_string(secret).unwrap()
    );
    let found: Value = {
        let source = script_source();
        let runtime = rquickjs::Runtime::new().unwrap();
        let context = rquickjs::Context::full(&runtime).unwrap();
        context.with(|ctx| {
            super::install_entry_validator(&ctx).unwrap();
            let script = [
                "const args = {authorMaxParallelism:1, gateMode:'enforce'};",
                &source,
                "(async () => {",
                &body,
                "})()",
            ]
            .join("\n");
            let promise: rquickjs::Promise = ctx.eval(script).unwrap();
            serde_json::from_str(&promise.finish::<String>().unwrap()).unwrap()
        })
    };
    let output = found.to_string();
    assert!(!output.contains(secret), "{output}");
    assert!(output.contains("[REDACTED]"), "{output}");
    assert!(
        found["out"]["findings"][0]["text"]
            .as_str()
            .unwrap()
            .contains("[REDACTED]")
    );
    assert!(found["stored"][0].as_str().unwrap().contains("[REDACTED]"));
}

#[test]
fn the_script_fails_clearly_when_the_host_redactor_binding_is_missing() {
    let found = eval_after_script(
        "(() => { __archonRedactPublicText = null; try { redactSecretShapedText('text'); return JSON.stringify('did not throw'); } catch (error) { return JSON.stringify(String(error.message)); } })()",
    );
    assert!(
        found
            .as_str()
            .unwrap()
            .contains("host public-text redactor binding __archonRedactPublicText is required"),
        "{found}"
    );
}
