//! Issue 554: a check the host could not prove, on an entry the author loop
//! authored, never ends the acceptance phase or stalls it unseen by its
//! author: the same candidate is frozen again, and a repeat on the entry's
//! unchanged text goes to that entry's author.
use super::shape_tests::script_source;
use super::*;
use serde_json::{Value, json};

const UNPROVEN: &str = "check 'A': operational: the host could not prove it after repairing its own environment (the tree it ran on did not build); it is not published until the host can";

/// Drive the acceptance author loop over `ids` against freeze outcomes
/// `freeze(n)` (a JS function body over `n`, the 1-based freeze number).
fn drive(ids: &[&str], freeze: &str) -> Value {
    let criteria: serde_json::Map<String, Value> = (ids.iter())
        .map(|id| (id.to_string(), json!(id.to_lowercase())))
        .collect();
    let criteria = Value::Object(criteria);
    let runtime = rquickjs::Runtime::new().unwrap();
    let context = rquickjs::Context::full(&runtime).unwrap();
    context.with(|ctx| {
        install_entry_validator(&ctx).unwrap();
        archon_workflow::v2::script::author_context::install_preview(&ctx).unwrap();
        let script = format!(
            r#"const args = {{repositoryRoot:'/archon-test-roots/repo',projectRoot:'/archon-test-roots/project',taskRoot:'/archon-test-roots/project/tasks',acceptanceCriteria:{criteria},authorMaxParallelism:1,gateMode:'enforce'}};
            {source}
            (async () => {{
                let freezes = 0;
                const authored = [];
                const prompts = [];
                const w = {{
                    agent: async (id, options) => {{
                        if (authored.length > 12) throw Error('bounded test exhausted');
                        const entry = Object.keys(args.acceptanceCriteria).find(key => id.includes(`-${{key}}-`));
                        authored.push(entry);
                        prompts.push(String(options.task));
                        return {{status:'accepted',stopReason:'end_turn',content:JSON.stringify({{
                            id: entry, check:{{kind:'command',command:'test -f output',cwd:'project_root'}},
                            gap_permitted:false, covers:[]
                        }})}};
                    }},
                    hostCommand: async () => {{
                        freezes++;
                        const n = freezes;
                        {freeze}
                    }},
                    pause: async (id) => {{throw Error('unexpected pause ' + id);}}
                }};
                let error = null;
                try {{
                    await authorCandidate(w, {{phase:'acceptance',prompt:()=> 'author',
                        author:authorAcceptanceEntries,capability:'freeze-acceptance',
                        retryScopes:new Set(['candidate_artifact'])}});
                }} catch (e) {{ error = String(e.message); }}
                return JSON.stringify({{authored,freezes,error,prompts}});
            }})()"#,
            source = script_source(),
        );
        let promise: rquickjs::Promise = ctx.eval(script).unwrap();
        let result: String = promise.finish().unwrap();
        serde_json::from_str(&result).unwrap()
    })
}

fn unproven_finding() -> String {
    json!({"text": UNPROVEN, "subject": "A", "remediation_scope": "operational",
        "source_path": "/archon-test-roots/project/tasks/acceptance-contract.json",
        "deterministic_defect": {"code": "host_permanent", "subject": "A", "kind": "check"}})
    .to_string()
}

const ACCEPTED: &str = "return {publicationReceipt:{call_id:'freeze'},postcondition:{satisfied:true},gateEnvelope:{policy_findings:[]}};";

fn assert_reached_its_author(result: &Value, authored: Value) {
    assert_eq!(result["error"], Value::Null, "{result}");
    assert_eq!(result["authored"], authored, "{result}");
    let last = result["prompts"].as_array().unwrap().last().unwrap();
    let last = last.as_str().unwrap();
    assert!(
        last.contains("two freezes of its unchanged text") && last.contains("did not build"),
        "{last}"
    );
}

/// A staged freeze that stalls on the unproven check alone: the first stall
/// re-freezes the same candidate, and the repeat goes to the entry's author.
#[test]
fn a_stalled_freeze_unproven_on_an_unchanged_entry_twice_goes_to_its_author_554() {
    let freeze = format!(
        "if (n <= 2) return {{gateEnvelope:{{policy_findings:[{}],operational_error:{{kind:'permanent_host_fault',text:'operational: the host could not prove 1 check(s)'}}}}}};\n{ACCEPTED}",
        unproven_finding()
    );
    let result = drive(&["A"], &freeze);
    assert_eq!(result["freezes"], 3, "{result}");
    assert_reached_its_author(&result, json!(["A", "A"]));
}

/// An unstaged freeze returns the unproven check as its only finding: that
/// never ends the phase (it used to stop it, terminally).
#[test]
fn an_unproven_entry_as_the_only_finding_never_ends_the_phase_554() {
    let freeze = format!(
        "if (n <= 2) return {{publicationReceipt:{{call_id:'freeze'}},postcondition:{{satisfied:true}},gateEnvelope:{{policy_findings:[{}]}}}};\n{ACCEPTED}",
        unproven_finding()
    );
    let result = drive(&["A"], &freeze);
    assert_eq!(result["freezes"], 3, "{result}");
    assert_reached_its_author(&result, json!(["A", "A"]));
}

/// The R11 shape: beside a finding its author repairs, the unproven entry
/// is carried unchanged; the next freeze reports it again, so it goes to
/// its own author too.
#[test]
fn an_unproven_entry_beside_authored_findings_reaches_its_author_on_the_repeat_554() {
    let refuted = json!({"text": "check 'B' was refuted: repair", "subject": "B",
        "remediation_scope": "candidate_artifact"});
    let freeze = format!(
        "if (n === 1) return {{publicationReceipt:{{call_id:'freeze'}},postcondition:{{satisfied:true}},gateEnvelope:{{policy_findings:[{refuted},{unproven}]}}}};\nif (n === 2) return {{publicationReceipt:{{call_id:'freeze'}},postcondition:{{satisfied:true}},gateEnvelope:{{policy_findings:[{unproven}]}}}};\n{ACCEPTED}",
        unproven = unproven_finding()
    );
    let result = drive(&["A", "B"], &freeze);
    assert_eq!(result["freezes"], 3, "{result}");
    assert_reached_its_author(&result, json!(["A", "B", "B", "A"]));
}
