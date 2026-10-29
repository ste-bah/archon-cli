//! O1 and O3 through the whole embedded script under `node`: a skeleton
//! re-author re-authors only the bodies its change reaches, and a skeleton
//! the host refuses for dropping a frozen task goes back to its author.
//!
//! The scripted host reads each skeleton candidate it is sent and answers
//! with that candidate's subjects, as the real freeze does.

use super::FIXED_SCRIPT_SOURCE;

const TASK_ROOT: &str = "/p/tasks";

/// One skeleton task entry implementing `implements`.
fn task(id: &str, implements: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "task_id": id, "file_name": format!("{id}.md"), "depends_on": [], "blocks": [],
        "implements": implements, "deliverable_contracts": []
    })
}

pub(crate) fn skeleton(tasks: Vec<serde_json::Value>) -> String {
    serde_json::json!({"schema_version": 1, "acceptance_digest": "", "tasks": tasks}).to_string()
}

/// `skeletons`: the skeleton author's replies in order (the last repeats);
/// `refused`: freeze-skeleton call ordinals the host refuses with `refusal`;
/// `trace`: requirements-trace finding rounds (the last repeats).
fn driver(skeletons: &[String], refused: &[usize], refusal: &str, trace: &str) -> String {
    driver_on(skeletons, refused, refusal, trace, "undefined")
}

/// [`driver`] on a task root the launcher found frozen: `frozen` is the
/// `frozenChain` argument (a JS expression), and the host verifies each
/// frozen stage in place, answering with the launch-bound subjects.
pub(crate) fn driver_on(
    skeletons: &[String],
    refused: &[usize],
    refusal: &str,
    trace: &str,
    frozen: &str,
) -> String {
    let skeletons = serde_json::to_string(skeletons).unwrap();
    let refused = serde_json::to_string(refused).unwrap();
    format!(
        r##"{FIXED_SCRIPT_SOURCE}
globalThis.__archonValidateAcceptanceEntry = () => "[]";
globalThis.__archonRedactPublicText = text => String(text);
const stubDigest = (text) => [...String(text)].reduce((h, c) => Math.imul(h ^ c.codePointAt(0), 16777619) >>> 0, 2166136261).toString(16).padStart(8, "0").repeat(8);
globalThis.__archonAuthorContext = (ext, text) => JSON.stringify({{ path: "/run/author-context/" + stubDigest(text) + "." + ext, sha256: stubDigest(text) }});
globalThis.args = {{
  projectRoot: "/p", repositoryRoot: "/r", prdPath: "/p/prd.md", prdDigest: "d", taskRoot: "{TASK_ROOT}",
  gateMode: "enforce", acceptanceCriteria: {{ "AC-X-001": "criterion" }}, frozenChain: {frozen},
}};
const skeletons = {skeletons};
const refused = {refused};
const traceRounds = {trace};
const agentCalls = [];
const hostCalls = [];
const landed = [];
const prompts = {{}};
let skeletonReplies = 0, skeletonFreezes = 0, traceCalls = 0;
let finalInputs = null;
const w = {{ checkpoint: async () => {{}},
  pause: async (id) => {{ throw new Error("workflow paused by run control: script requested pause " + id); }},
  agent: async (id, options) => {{
    agentCalls.push(id);
    (prompts[id] = prompts[id] || []).push(options.task);
    let content = "# body " + id;
    if (id.startsWith("acceptance-author-")) content = JSON.stringify({{ id: "AC-X-001" }});
    if (id.startsWith("skeleton-author-")) content = skeletons[Math.min(skeletonReplies++, skeletons.length - 1)];
    return {{ status: "accepted", stopReason: "end_turn", content }};
  }},
  hostCommand: async (capability, options) => {{
    hostCalls.push(capability);
    const callId = capability + "-" + hostCalls.length;
    const committed = {{
      publicationReceipt: {{ call_id: callId }},
      result: {{ data: {{ publicationReceipt: {{ call_id: callId }} }} }},
      postcondition: {{ satisfied: true }},
      gateEnvelope: {{ policy_findings: [] }},
      subjects: [],
    }};
    if (capability === "freeze-skeleton") {{
      skeletonFreezes += 1;
      if (refused.includes(skeletonFreezes)) {{
        return {{ gateEnvelope: {{ policy_findings: [{{ text: {refusal:?}, subject: "skeleton", remediation_scope: "candidate_artifact" }}] }} }};
      }}
      committed.subjects = JSON.parse(options.stdin).tasks.map((t) => ({{ taskId: t.task_id, fileName: t.file_name }}));
    }}
    if (capability === "verify-frozen-skeleton") committed.subjects = args.frozenChain.subjects;
    if (capability === "land-task-body") landed.push(options.target.taskId + "=" + callId);
    if (capability === "requirements-trace") {{
      committed.gateEnvelope.policy_findings = traceRounds[Math.min(traceCalls++, traceRounds.length - 1)];
    }}
    return committed;
  }},
  finalReport: async (_id, options) => {{ finalInputs = options.inputs; return {{}}; }},
}};
workflow(w).then(
  () => console.log(JSON.stringify({{ agentCalls, hostCalls, landed, prompts, evidence: finalInputs.map((o) => o.publicationReceipt.call_id) }})),
  (error) => console.log(JSON.stringify({{ agentCalls, hostCalls, landed, prompts, error: String(error && error.message) }})),
);
"##
    )
}

pub(crate) fn run(script: &str) -> serde_json::Value {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("set-gate-skeleton.cjs");
    std::fs::write(&path, script).expect("write driver");
    let out = std::process::Command::new("node")
        .arg(&path)
        .output()
        .expect("node must be available");
    assert!(
        out.status.success(),
        "driver failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("driver json")
}

pub(crate) const UNCLAIMED: &str = r#"[[{ "text": "PRD obligation 'REQ-X-009' is claimed by no task; add it to at least one TASK file's implements list", "subject": "REQ-X-009", "source_path": "/p/prd.md", "remediation_scope": "skeleton" }], []]"#;

pub(crate) fn calls(out: &serde_json::Value, key: &str) -> Vec<String> {
    (out[key].as_array().expect(key).iter())
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

/// O3: the re-authored skeleton changes TASK-X-020's frozen fields and adds
/// TASK-X-030; TASK-X-010 keeps its frozen entry, so its body keeps its
/// published bytes (never landed again) and the set gates judge the whole
/// set again. The author call ids are the replay identity on resume.
#[test]
fn a_skeleton_re_author_re_authors_only_the_bodies_its_change_reaches() {
    let first = skeleton(vec![
        task("TASK-X-010", &["AC-X-001"]),
        task("TASK-X-020", &["AC-X-001"]),
    ]);
    let second = skeleton(vec![
        task("TASK-X-010", &["AC-X-001"]),
        task("TASK-X-020", &["AC-X-001", "REQ-X-009"]),
        task("TASK-X-030", &["REQ-X-009"]),
    ]);
    let script = driver(&[first, second], &[], "", UNCLAIMED);
    let out = run(&script);
    assert!(out.get("error").is_none(), "{out}");
    let landed = calls(&out, "landed");
    let targets: Vec<&str> = landed
        .iter()
        .map(|l| l.split('=').next().unwrap())
        .collect();
    assert_eq!(
        targets,
        ["TASK-X-010", "TASK-X-020", "TASK-X-020", "TASK-X-030"],
        "{out}"
    );
    let agents = calls(&out, "agentCalls");
    assert!(
        agents.contains(&"body-TASK-X-020-author-2".to_string()),
        "{out}"
    );
    assert!(
        agents.contains(&"body-TASK-X-030-author-1".to_string()),
        "{out}"
    );
    assert!(
        !agents.contains(&"body-TASK-X-010-author-2".to_string()),
        "{out}"
    );
    // The kept body's evidence is its first landing, unchanged.
    let evidence = calls(&out, "evidence");
    assert!(
        evidence.contains(&landed[0].split('=').nth(1).unwrap().to_string()),
        "{out}"
    );
    let host = calls(&out, "hostCalls");
    let reauthored = host.iter().rposition(|c| c == "freeze-skeleton").unwrap();
    assert!(
        host[reauthored..].iter().any(|c| c == "task-set-lint")
            && host[reauthored..].iter().any(|c| c == "requirements-trace"),
        "both set gates judge the whole set again: {out}"
    );
    // Replayed from the start, the run makes exactly the same calls.
    assert_eq!(run(&script)["agentCalls"], out["agentCalls"]);
}

/// O1: the host refuses a re-authored skeleton that drops a frozen task
/// before publishing it, as a candidate finding: the script hands it to
/// the skeleton author, whose next skeleton is frozen; the run goes on.
#[test]
fn a_skeleton_the_host_refuses_for_dropping_a_frozen_task_is_re_authored_with_that_finding() {
    let first = skeleton(vec![
        task("TASK-X-010", &["AC-X-001"]),
        task("TASK-X-020", &["AC-X-001"]),
    ]);
    let second = skeleton(vec![
        task("TASK-X-010", &["AC-X-001", "REQ-X-009"]),
        task("TASK-X-020", &["AC-X-001"]),
    ]);
    let refusal = "the skeleton drops frozen task TASK-X-020 (TASK-X-020.md); a re-authored skeleton keeps every frozen task_id and file_name and may only add implements and tasks";
    let out = run(&driver(&[first, second], &[2], refusal, UNCLAIMED));
    assert!(out.get("error").is_none(), "{out}");
    let prompt = out["prompts"]["skeleton-author-3"][0]
        .as_str()
        .expect("re-authored");
    assert!(prompt.contains("drops frozen task TASK-X-020"), "{prompt}");
    assert_eq!(
        calls(&out, "hostCalls")
            .iter()
            .filter(|c| *c == "freeze-skeleton")
            .count(),
        3,
        "{out}"
    );
}

#[path = "workflow_decompose_set_gate_skeleton_launch_tests.rs"]
mod launch;
