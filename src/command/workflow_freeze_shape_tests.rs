//! Real refusal envelopes exercise field-by-field convergence.
use super::tests::{assert_author_loop_keeps_running, envelope};
use super::*;
use serde_json::{Value, json};

fn subjects(value: &Value) -> std::collections::BTreeSet<String> {
    value["policy_findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| {
            assert_eq!(
                finding["deterministic_defect"]["code"],
                "invalid_candidate_shape"
            );
            let subject = &finding["deterministic_defect"]["subject"];
            subject.as_str().expect("subject").to_string()
        })
        .collect()
}

/// Runs `repairs` one per attempt on `candidate`: the defect count must fall
/// by one each time, the first envelope must name `first` exactly, and the
/// fixed script's author loop must keep running on the real envelopes.
fn assert_converges(
    shape: &ElementShape,
    mut candidate: Value,
    first: &[&str],
    repairs: &[(&str, Value)],
) {
    let dir = tempfile::tempdir().expect("fixture");
    let mut envelopes = Vec::new();
    for step in 0..=repairs.len() {
        let value = envelope(dir.path(), &format!("step-{step}"), &candidate, shape);
        let named = subjects(&value);
        assert_eq!(named.len(), repairs.len() - step, "step {step}: {value}");
        if step == 0 {
            let expected: std::collections::BTreeSet<_> =
                first.iter().map(|pointer| pointer.to_string()).collect();
            assert_eq!(named, expected, "{value}");
        }
        envelopes.push(value);
        if let Some((pointer, fill)) = repairs.get(step) {
            let (parent, key) = pointer.rsplit_once('/').expect("pointer");
            let target = candidate.pointer_mut(parent).expect("parent");
            target[key] = fill.clone();
        }
    }
    assert_author_loop_keeps_running(&envelopes);
}

#[test]
fn workflow_host_command_each_missing_consumed_artifact_path_is_its_own_identity() {
    let candidate = json!({ "schema_version": 1, "acceptance_digest": "d", "tasks": [{
        "task_id": "TASK-X-002", "file_name": "TASK-X-002.md",
        "depends_on": [{ "task_id": "TASK-X-001", "consumes": [{}, {}, {}] }] }] });
    let at = "/tasks/0/depends_on/0/consumes";
    let repairs: Vec<_> = (0..3)
        .map(|n| {
            (
                format!("{at}/{n}/artifact_path"),
                json!(format!("out{n}.json")),
            )
        })
        .collect();
    let repairs: Vec<_> = repairs
        .iter()
        .map(|(p, v)| (p.as_str(), v.clone()))
        .collect();
    let first: Vec<_> = repairs.iter().map(|(p, _)| &p[1..]).collect();
    assert_converges(&TASK_SHAPE, candidate, &first, &repairs);
}

/// Entries and supplementary entries alike: a command check's missing or
/// invalid field, each count. Missing tags are covered by the round 11 corpus.
#[test]
fn workflow_host_command_each_command_check_field_is_its_own_identity() {
    let candidate = json!({
        "entries": [{ "id": "AC-X-001", "criterion": "c", "check": { "kind": "command" } }],
        "supplementary": [{ "id": "SUP-X-001", "criterion": "c",
            "check": { "kind": "command", "command": "true", "cwd": "elsewhere" } }] });
    let repairs = [
        ("/entries/0/check/command", json!("true")),
        ("/entries/0/check/cwd", json!("project_root")),
        ("/supplementary/0/check/cwd", json!("repo_root")),
    ];
    let first: Vec<_> = repairs.iter().map(|(p, _)| &p[1..]).collect();
    assert_converges(&ENTRY_SHAPE, candidate, &first, &repairs);
}

/// A floor check's contract, missing or empty: each required contract field
/// is its own identity.
#[test]
fn workflow_host_command_each_floor_contract_field_is_its_own_identity() {
    let candidate = json!({
        "entries": [{ "id": "AC-X-001", "criterion": "c", "check": { "kind": "floor" } }],
        "supplementary": [{ "id": "SUP-X-001", "criterion": "c",
            "check": { "kind": "floor", "contract": {} } }] });
    let repairs = [
        ("/entries/0/check/contract", json!({})),
        ("/entries/0/check/contract/kind", json!("file")),
        ("/entries/0/check/contract/artifact_path", json!("a.json")),
        ("/supplementary/0/check/contract/kind", json!("file")),
        (
            "/supplementary/0/check/contract/artifact_path",
            json!("b.json"),
        ),
    ];
    let first = [
        "entries/0/check/contract",
        "entries/0/check/contract/kind",
        "entries/0/check/contract/artifact_path",
        "supplementary/0/check/contract/kind",
        "supplementary/0/check/contract/artifact_path",
    ];
    assert_converges(&ENTRY_SHAPE, candidate, &first, &repairs);
}

// The real native binding and the embedded author, not a Node mock. Allow
// replaying the old JS to demonstrate these regressions fail before the fix.
pub(super) fn script_source() -> String {
    if let Ok(root) = std::env::var("ARCHON_TEST_SCRIPT_ROOT") {
        [
            "workflow_decompose_v1.js",
            "workflow_decompose_v1_acceptance.js",
            "workflow_decompose_v1_set_gate.js",
            "workflow_decompose_v1_progress.js",
        ]
        .iter()
        .map(|name| std::fs::read_to_string(std::path::Path::new(&root).join(name)).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
    } else {
        crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE.to_string()
    }
}

fn assert_author_shape_refusal(entry: Value) {
    let source = script_source();
    let runtime = rquickjs::Runtime::new().unwrap();
    let context = rquickjs::Context::full(&runtime).unwrap();
    context.with(|ctx| {
        install_entry_validator(&ctx).unwrap();
        ctx.eval::<(), _>(format!("const args = {{repositoryRoot:'/archon-test-roots/repo',projectRoot:'/archon-test-roots/project',}};\n{source}")).unwrap();
        let candidate = serde_json::to_vec(&json!({"entries": [entry.clone()]})).unwrap();
        // Each refusal names the entry (Issue 357 round 4), not `entries/0`.
        let expected: Vec<_> = element_shape_defects(&candidate, &ENTRY_SHAPE)
            .iter().map(|defect| super::entry_validator::refusal_text("A", defect)).collect();
        assert!(!expected.is_empty(), "test entry must be invalid");
        let script = format!(r#"(async () => {{
            const entry = {entry};
            const w = {{agent: async () => ({{status: 'accepted', stopReason: 'end_turn', content: JSON.stringify(entry)}})}};
            const refused = await authorOne(w, 'author', 1, 'A', 'a', [], {{A:'a'}}, {{roundCalls:0, roundAnswered:0}});
            entry.check = {{kind:'command', command:'test -f output', cwd:'project_root'}};
            const repaired = await authorOne(w, 'repair', 2, 'A', 'a', [], {{A:'a'}}, {{roundCalls:0, roundAnswered:0}});
            return JSON.stringify({{refused, repaired}});
        }})()"#);
        let promise: rquickjs::Promise = ctx.eval(script).unwrap();
        let result: String = promise.finish().unwrap();
        let result: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result["refused"]["failure"]["malformed"], true, "{result}");
        // Its repair is measured in the frontier of the entry it names.
        assert_eq!(result["refused"]["failure"]["entryId"], "A", "{result}");
        let summary = result["refused"]["failure"]["summary"].as_str().unwrap();
        for message in expected { assert!(summary.contains(&message), "{summary}"); }
        // The host-owned criterion is stamped, never repaired (round 5).
        assert_eq!(result["repaired"]["entry"]["criterion"], "a", "{result}");
    });
}

// Issue 357 round 5: criterion is host-owned (stamped before validation), so
// these drive an author-owned field; the expected texts use the host value.
#[test]
fn acceptance_author_refuses_missing_command_with_freeze_validator() {
    assert_author_shape_refusal(
        json!({"id":"A", "criterion":"a", "check":{"kind":"command", "cwd":"project_root"}}),
    );
}

#[test]
fn acceptance_author_refuses_null_command_with_freeze_validator() {
    assert_author_shape_refusal(
        json!({"id":"A", "criterion":"a", "check":{"kind":"command", "command":null, "cwd":"project_root"}}),
    );
}

#[test]
fn acceptance_author_refuses_numeric_command_with_freeze_validator() {
    assert_author_shape_refusal(
        json!({"id":"A", "criterion":"a", "check":{"kind":"command", "command":42, "cwd":"project_root"}}),
    );
}

// Issue 357 round 3: after the judge refutes a shape-valid entry, the native
// validator's 5 -> 4 -> 3 -> 2 -> 1 -> 0 repair completes without a pause.
#[test]
fn native_refuted_entry_shape_repairs_decrease_five_to_zero_without_pause() {
    let runtime = rquickjs::Runtime::new().unwrap();
    let context = rquickjs::Context::full(&runtime).unwrap();
    context.with(|ctx| {
        install_entry_validator(&ctx).unwrap();
        let script = format!(
            r#"const args = {{repositoryRoot:'/archon-test-roots/repo',projectRoot:'/archon-test-roots/project',acceptanceCriteria:{{A:'a'}},authorMaxParallelism:1,gateMode:'enforce'}};
            {source}
            (async () => {{
                let calls = 0, freezes = 0;
                const w = {{
                    agent: async () => {{
                        if (++calls > 12) throw Error('bounded test exhausted');
                        const r = calls === 1 ? 6 : calls - 1;
                        return {{status:'accepted',stopReason:'end_turn',content:JSON.stringify({{
                            id:'A',
                            check:{{kind:'command',command:r >= 2 ? 'test -f output' : 42,
                                cwd:r >= 3 ? 'project_root' : null}},
                            gap_permitted:r >= 4 ? false : 'false',
                            covers:r >= 6 ? [] : r >= 5 ? ['REQ-Y', null] : [42, null]
                        }})}};
                    }},
                    hostCommand: async () => {{
                        freezes++;
                        const findings = freezes > 1 ? [] : [{{text:"check 'A' was refuted: repair",
                            subject:'A',remediation_scope:'candidate_artifact'}}];
                        return {{publicationReceipt:{{call_id:'freeze'}},postcondition:{{satisfied:true}},
                            gateEnvelope:{{policy_findings:findings}}}};
                    }},
                    pause: async () => {{throw Error('unexpected pause');}}
                }};
                let error = null;
                try {{
                    await authorCandidate(w, {{phase:'acceptance',prompt:()=> 'author',
                        author:authorAcceptanceEntries,capability:'freeze-acceptance',
                        retryScopes:new Set(['candidate_artifact'])}});
                }} catch (e) {{ error = String(e.message); }}
                return JSON.stringify({{calls,freezes,error}});
            }})()"#,
            source = script_source(),
        );
        let promise: rquickjs::Promise = ctx.eval(script).unwrap();
        let result: String = promise.finish().unwrap();
        let result: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result, json!({"calls":7,"freezes":2,"error":null}));
    });
}
