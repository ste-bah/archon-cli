//! Obs-22: `reviewMapReduce` hands the reduce the map findings AND a roster
//! of the branches that ran, so a branch that reviewed its task and found
//! nothing is not mistaken for a task never reviewed.
//!
//! Executes the REAL prelude functions against a fake `w` that records what
//! the reduce receives. The host replaces this roster with its own at
//! dispatch (`v2::review_roster`); this pins that the script side sends one
//! at all, and that it is shaped like the host's.

/// Pull one named arrow-function definition out of the prelude by name.
fn prelude_fn(name: &str) -> String {
    let prelude = super::super::V3_PRIMITIVES_JS;
    let marker = format!("  const {name} = ");
    let start = prelude
        .find(&marker)
        .unwrap_or_else(|| panic!("prelude must define {name}"));
    let end = start
        + prelude[start..]
            .find("\n  };")
            .unwrap_or_else(|| panic!("{name} must end with a closing arrow body"))
        + 5;
    prelude[start..end].to_string()
}

fn run_js(driver: &str) -> String {
    let mut script = String::from(
        "const slug = (t) => String(t).toLowerCase().replace(/[^a-z0-9]+/g, \"-\");\n",
    );
    for name in [
        "reviewFindings",
        "outcomesOf",
        "reviewRoster",
        "reviewMapReduce",
    ] {
        script.push_str(&prelude_fn(name));
        script.push('\n');
    }
    script.push_str(driver);
    script.push('\n');
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("roster.mjs");
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
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The live map shape, reduced: outcome views with host-stamped ids, and the
/// host's attributed findings -- two on the first branch, none on the second.
const MAP_ENV: &str = r#"{ "status": "accepted", "data": {
  "outcomes": [
    { "item_id": "adversarial-review-map-0", "status": "accepted", "canonical_task_ids": ["TASK-A"] },
    { "item_id": "adversarial-review-map-1", "status": "accepted", "canonical_task_ids": ["TASK-B"] }
  ],
  "review_findings": { "findings": [
    { "id": "F1", "canonical_task_ids": ["TASK-A"] },
    { "id": "F2", "canonical_task_ids": ["TASK-A"] }
  ] } } }"#;

#[test]
fn review_map_reduce_hands_the_reduce_findings_and_a_branch_roster() {
    let driver = format!(
        r#"const calls = [];
const w = {{
  parallel: async (id, items, opts) => {{ calls.push(["parallel", id, items.length]); return {MAP_ENV}; }},
  reduce: async (id, input, opts) => {{ calls.push(["reduce", id, input]); return {{ review_findings: {{ findings: [{{ id: "X" }}] }} }}; }},
}};
const out = await reviewMapReduce("adversarial-review", "adversarial_findings", "map task", "reduce task", ["TASK-A", "TASK-B"], null);
console.log(JSON.stringify({{ out, calls: calls.map((c) => c.slice(0, 2)), reduceInput: calls[1][2] }}));"#
    );
    let got = run_js(&driver);
    let value: serde_json::Value = serde_json::from_str(&got).expect("json");
    assert_eq!(
        value["calls"],
        serde_json::json!([
            ["parallel", "adversarial-review-map"],
            ["reduce", "adversarial-review-reduce"]
        ])
    );
    assert_eq!(
        value["reduceInput"]["findings"],
        serde_json::json!([{ "id": "F1", "canonical_task_ids": ["TASK-A"] }, { "id": "F2", "canonical_task_ids": ["TASK-A"] }])
    );
    assert_eq!(
        value["reduceInput"]["branch_roster"],
        serde_json::json!([
            { "item_id": "adversarial-review-map-0", "canonical_task_ids": ["TASK-A"], "status": "accepted", "finding_count": 2 },
            { "item_id": "adversarial-review-map-1", "canonical_task_ids": ["TASK-B"], "status": "accepted", "finding_count": 0 },
        ])
    );
    // The review still returns the host's merged attachment from the reduce.
    assert_eq!(value["out"], serde_json::json!([{ "id": "X" }]));
}

#[test]
fn a_map_without_outcome_views_yields_an_empty_roster_not_a_throw() {
    let got = run_js(
        r#"console.log(JSON.stringify([reviewRoster(null), reviewRoster({}), reviewRoster({ data: { outcomes: [] } })]));"#,
    );
    assert_eq!(got, "[[],[],[]]");
}
