//! The author calls the fixed script makes: the first acceptance prompt, the
//! body prompt's tool rule, one call per acceptance entry, and bounded
//! batches. Split from the phase-loop repair tests.

use super::{FIXED_SCRIPT_SOURCE, decl};

#[test]
fn the_first_author_receives_check_alternatives_and_the_falsifiability_standard() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prompt.mjs");
    let script = format!(
        r#"{FIXED_SCRIPT_SOURCE}
globalThis.args = {{projectRoot:"p",repositoryRoot:"r",prdPath:"p.md",prdDigest:"d",taskRoot:"tasks",gateMode:"observe",acceptanceCriteria:{{"AC-X-001":"criterion"}}}};
workflow({{agent:async(_, input)=>{{console.log(input.task);throw Error("captured");}}}}).catch(e=>{{if(e.message!=="captured") throw e;}});
"#
    );
    std::fs::write(&path, script).unwrap();
    let output = std::process::Command::new("node")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let prompt = String::from_utf8(output.stdout).unwrap();
    for required in [
        "exactly one acceptance entry",
        "exact supplied id",
        "fail when its criterion is false",
        "typed_verifier_command",
        "\"kind\":\"command\"",
        "Criterion and judgment are host-owned",
    ] {
        assert!(
            prompt.contains(required),
            "missing author guidance: {required}"
        );
    }
}

#[test]
fn mcp_obligation_body_prompt_requires_project_specific_declarations() {
    let shape = decl("BODY_SHAPE");
    assert!(!shape.contains("required_tools: []"));
    let body_policy = decl("bodyPolicy");
    assert!(body_policy.contains(".mcp.json"));
    assert!(body_policy.contains("every declared tool"));
}

#[test]
fn acceptance_entries_are_separate_calls_and_truncation_retries_only_one() {
    let dir = tempfile::tempdir().unwrap();
    let script = format!(
        "{}\n{}",
        FIXED_SCRIPT_SOURCE,
        r#"
// Scheduling fixture: the real native validator is tested by shape_tests.
globalThis.__archonValidateAcceptanceEntry = () => '[]';
// Issue 288: the host's author-context binding, its files kept nowhere.
const stubDigest = (text) => [...String(text)].reduce((h, c) => Math.imul(h ^ c.codePointAt(0), 16777619) >>> 0, 2166136261).toString(16).padStart(8, "0").repeat(8);
globalThis.__archonAuthorContext = (ext, text) => JSON.stringify({ path: "/run/author-context/" + stubDigest(text) + "." + ext, sha256: stubDigest(text) });
globalThis.args = { projectRoot:'/p', repositoryRoot:'/r', prdPath:'/p/prd', prdDigest:'x', taskRoot:'/p/tasks', gateMode:'observe', acceptanceCriteria:{'AC-X-001':'first','AC-X-002':'second'} };
let calls = [], freezes = [], failed = false;
const w = {
 finalReport: async ()=>({}),
 agent: async (id, options) => {
  calls.push(id);
  if(id.includes('AC-X-002') && !failed) { failed=true; return {status:'accepted',stopReason:'max_tokens',content:'cut'}; }
  return {status:'accepted',stopReason:'end_turn',content:JSON.stringify({id:id.includes('AC-X-002')?'AC-X-002':'AC-X-001'})};
 },
 hostCommand: async (cap, options) => {
  if(cap==='freeze-acceptance') freezes.push(JSON.parse(options.stdin));
  return {publicationReceipt:{call_id:cap},result:{data:{publicationReceipt:{call_id:cap}}},postcondition:{satisfied:true},gateEnvelope:{policy_findings:[]},subjects:[{taskId:'TASK-X-001',fileName:'TASK-X-001.md'}]};
 }
};
workflow(w).then(()=>{
 const a=calls.filter(x=>x.startsWith('acceptance-author-'));
 if(a.length!==3 || !a[0].includes('AC-X-001') || !a[1].includes('AC-X-002') || !a[2].includes('AC-X-002')) throw Error(JSON.stringify(calls));
 if(freezes[0].entries.length!==2) throw Error('not entry envelope');
}).catch(e=>{console.error(e);process.exitCode=1;});
"#
    );
    let path = dir.path().join("entry-test.cjs");
    std::fs::write(&path, script).unwrap();
    let out = std::process::Command::new("node")
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn acceptance_cost_selective_repair_and_bounded_batches() {
    let output = std::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/command/workflow_decompose_cost_test.cjs"
        ))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// N subjects produce N body author calls with at most `authorMaxParallelism`
/// in flight, frozen bodies skipped, evidence in skeleton order, and a failing
/// author surfacing only after its batch settles.
#[test]
fn body_authoring_fans_out_in_bounded_batches() {
    let output = std::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/command/workflow_decompose_body_batch_test.cjs"
        ))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
