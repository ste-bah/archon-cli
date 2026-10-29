use super::set_gate_skeleton::{driver_on, run, skeleton};

pub(crate) fn script(stage: &str, fault: &str) -> String {
    let tasks = serde_json::json!([
        {"task_id":"TASK-X-010", "file_name":"TASK-X-010.md", "implements":["AC-X-001"]},
        {"task_id":"TASK-X-020", "file_name":"TASK-X-020.md", "implements":["AC-X-001"]}
    ]);
    let mut next = tasks.clone();
    next[0]["implements"] = serde_json::json!(["AC-X-001", "REQ-X-001"]);
    script_with(stage, fault, tasks, next)
}

fn script_with(
    stage: &str,
    fault: &str,
    tasks: serde_json::Value,
    next: serde_json::Value,
) -> String {
    let frozen = serde_json::json!({
        "acceptance":true,"skeleton":true,"tasks":tasks,
        "subjects":[{"taskId":"TASK-X-010","fileName":"TASK-X-010.md"},
                    {"taskId":"TASK-X-020","fileName":"TASK-X-020.md"}],
        "bodies":["TASK-X-010.md","TASK-X-020.md"]
    });
    let base = driver_on(
        &[skeleton(next.as_array().unwrap().clone())],
        &[],
        "",
        "[[]]",
        &frozen.to_string(),
    );
    let injected = format!(
        r#"
    await new Promise(resolve => setImmediate(resolve));
    if (capability === "verify-frozen-{stage}" && failures++ < failureCount) return {fault};
"#
    );
    base.replace("const w =", "globalThis.__archonHost = async (_method, payload) => { throw new Error(JSON.parse(payload).reason); }; let failures = 0; const failureCount = 1; const watchdog = setTimeout(() => process.exit(2), 5000); watchdog.unref();\nconst w =")
        .replace("hostCalls.push(capability);", &format!("hostCalls.push(capability);{injected}"))
}

#[test]
fn frozen_stage_authored_finding_reopens_its_author_and_keeps_unaffected_bodies() {
    let fault = r#"{gateEnvelope:{policy_findings:[{text:"repair the owning entry",subject:"TASK-X-010",source_path:"/p/tasks/task-skeleton.json",remediation_scope:"skeleton"}]}}"#;
    let out = run(&script("skeleton", fault));
    assert!(out.get("error").is_none(), "{out}");
    assert_eq!(
        out["agentCalls"],
        serde_json::json!(["skeleton-author-1", "body-TASK-X-010-author-1"]),
        "{out}"
    );
    assert_eq!(out["landed"].as_array().unwrap().len(), 1, "{out}");
    assert!(
        out["prompts"]["skeleton-author-1"][0]
            .as_str()
            .unwrap()
            .contains("repair the owning entry"),
        "{out}"
    );
}

#[test]
fn frozen_stage_typed_transient_verification_recovers_without_a_stop_or_author_call() {
    let fault = r#"{gateEnvelope:{operational_error:{kind:"transient",text:"read unavailable"}}}"#;
    let driver =
        script("skeleton", fault).replace("const failureCount = 1", "const failureCount = 5")
            // Resume once through the existing pause, then allow verification to recover.
            .replace("const agentCalls = [];", "const agentCalls = []; const pauses = [];")
            .replace("pause: async (id) => { throw new Error(\"workflow paused by run control: script requested pause \" + id); }", "pause: async (id) => { pauses.push(id); }")
            .replace("{ agentCalls, hostCalls, landed, prompts, evidence:", "{ agentCalls, hostCalls, landed, prompts, pauses, evidence:");
    let out = run(&driver);
    assert!(out.get("error").is_none(), "{out}");
    assert_eq!(out["agentCalls"], serde_json::json!([]), "{out}");
    assert_eq!(
        out["pauses"],
        serde_json::json!(["pause-verify-frozen-skeleton-1"]),
        "{out}"
    );
}

#[test]
fn frozen_stage_permanent_verification_stops_with_the_exact_remedy() {
    let fault = r#"{gateEnvelope:{operational_error:{kind:"permanent",text:"restore /p/tasks/task-skeleton.json from its authenticated publication"}}}"#;
    let out = run(&script("skeleton", fault));
    assert_eq!(
        out["error"],
        "verify-frozen-skeleton stopped: restore /p/tasks/task-skeleton.json from its authenticated publication",
        "{out}"
    );
    assert_eq!(out["agentCalls"], serde_json::json!([]), "{out}");
}

#[test]
fn frozen_stage_acceptance_repair_retains_other_checks_and_independent_bodies() {
    let contract = serde_json::json!({"acceptance":[
        {"id":"AC-X-001","check":{"kind":"command","command":"test -f out/a"}},
        {"id":"AC-X-002","check":{"kind":"command","command":"test -f out/b"}}
    ], "supplementary":[]});
    let fault = serde_json::json!({"gateEnvelope":{
        "report":contract.to_string(),
        "policy_findings":[{"text":"repair this check", "subject":"AC-X-001",
            "source_path":"/p/tasks/acceptance-contract.json", "remediation_scope":"candidate_artifact"}]
    }}).to_string();
    let tasks = serde_json::json!([
        {"task_id":"TASK-X-010","file_name":"TASK-X-010.md","implements":["AC-X-001"]},
        {"task_id":"TASK-X-020","file_name":"TASK-X-020.md","implements":["AC-X-002"]}
    ]);
    let driver = script_with("acceptance", &fault, tasks.clone(), tasks).replace(
        "acceptanceCriteria: { \"AC-X-001\": \"criterion\" }",
        "acceptanceCriteria: { \"AC-X-001\": \"criterion\", \"AC-X-002\": \"criterion\" }",
    );
    let out = run(&driver);
    assert!(out.get("error").is_none(), "{out}");
    assert_eq!(
        out["agentCalls"],
        serde_json::json!([
            "acceptance-author-AC-X-001-4",
            "skeleton-author-1",
            "body-TASK-X-010-author-1"
        ]),
        "{out}"
    );
    assert_eq!(out["landed"].as_array().unwrap().len(), 1, "{out}");
    let feedback = r#"if (capability === "freeze-acceptance" && hostCalls.filter(id => id === "freeze-acceptance").length === 1) return {gateEnvelope:{policy_findings:[{text:"repair the other check too",subject:"AC-X-002",remediation_scope:"candidate_artifact"}]}};"#;
    let subsequent = driver
        .replace(
            "hostCalls.push(capability);",
            &format!("hostCalls.push(capability); {feedback}"),
        )
        .replace(
            "JSON.stringify({ id: \"AC-X-001\" })",
            "JSON.stringify({ id: id.split(\"-\").slice(2, -1).join(\"-\") })",
        );
    let out = run(&subsequent);
    assert!(out.get("error").is_none(), "{out}");
    assert_eq!(
        out["landed"].as_array().unwrap().len(),
        2,
        "every check changed by the repair reaches its owner: {out}"
    );
}

#[test]
fn frozen_stage_changed_deliverable_reauthors_its_consumer_with_an_unchanged_entry() {
    let fault = r#"{gateEnvelope:{policy_findings:[{text:"repair the output contract",subject:"TASK-X-010",source_path:"/p/tasks/task-skeleton.json",remediation_scope:"skeleton"}]}}"#;
    let tasks = serde_json::json!([
        {"task_id":"TASK-X-010","file_name":"TASK-X-010.md","implements":["AC-X-001"]},
        {"task_id":"TASK-X-020","file_name":"TASK-X-020.md","implements":["AC-X-001"],
         "depends_on":[{"task_id":"TASK-X-010","consumes":[{"artifact_path":"out/a"}]}]}
    ]);
    let mut next = tasks.clone();
    next[0]["deliverable_contracts"] =
        serde_json::json!([{"kind":"report","artifact_path":"out/a"}]);
    let driver = script_with("skeleton", fault, tasks, next);
    let out = run(&driver);
    assert!(out.get("error").is_none(), "{out}");
    assert_eq!(
        out["agentCalls"],
        serde_json::json!([
            "skeleton-author-1",
            "body-TASK-X-010-author-1",
            "body-TASK-X-020-author-1"
        ]),
        "{out}"
    );
}

#[test]
fn verify_frozen_repeated_typed_transient_reaches_existing_stall_pause() {
    let fault = r#"{gateEnvelope:{operational_error:{kind:"transient",text:"read unavailable"}}}"#;
    let driver = script("skeleton", fault)
        .replace("const failureCount = 1", "const failureCount = Infinity")
        .replace("process.exit(2)", "(console.log(JSON.stringify({error: 'test watchdog', calls: hostCalls.length})), process.exit(0))")
        .replace("pause: async (id) =>", "pause: async (id, evidence) =>")
        .replace("throw new Error(\"workflow paused by run control: script requested pause \" + id)", "throw new Error(JSON.stringify({id, evidence}))");
    let out = run(&driver);
    let pause: serde_json::Value = serde_json::from_str(out["error"].as_str().unwrap())
        .unwrap_or_else(|_| panic!("expected progress pause: {out}"));
    assert_eq!(pause["id"], "pause-verify-frozen-skeleton-1", "{out}");
    assert_eq!(pause["evidence"]["reason"], "operational_no_progress");
    let calls = out["hostCalls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|id| *id == "verify-frozen-skeleton")
        .count();
    assert_eq!(
        calls,
        pause["evidence"]["stall_window"].as_u64().unwrap() as usize
    );
    assert_eq!(pause["evidence"]["progress_history_total"], calls);
    assert_eq!(out["agentCalls"], serde_json::json!([]));
}
