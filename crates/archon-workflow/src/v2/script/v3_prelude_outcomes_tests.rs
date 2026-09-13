//! `outcomesOf`: reconciling the two host views of one write branch.
//!
//! A fanout batch reports each branch twice. `data.outcomes` carries the
//! verdict; `data.items` carries the evidence arrays. A branch that changed a
//! file and ran commands can appear in `outcomes` with `files_changed: []` and
//! `commands_run: []`, so a caller handed that view alone concludes the branch
//! proved nothing — which is how a fully implemented task gets sent back
//! through remediation.

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
    let mut script = prelude_fn("outcomesOf");
    script.push('\n');
    script.push_str(driver);
    script.push('\n');
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("outcomes.mjs");
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

/// The exact shape a write wave produced on run wf-84bdba5d: the branch
/// changed one file and ran four commands, and its `outcomes` entry records
/// neither.
const SPLIT_BRANCH: &str = r#"{"data":{
      "outcomes":[{"status":"accepted","canonical_task_ids":["TASK-A"],
                   "files_changed":[],"commands_run":[]}],
      "items":[{"status":"accepted",
                "files_changed":[{"path":"src/produced.txt"}],
                "commands_run":[{"kind":"inspect"},{"kind":"test"},
                                {"kind":"test"},{"kind":"test"}]}]}}"#;

#[test]
fn an_outcome_carries_the_evidence_recorded_on_its_item() {
    let got = run_js(&format!(
        r#"const merged = outcomesOf({SPLIT_BRANCH});
console.log(JSON.stringify({{
  status: merged[0].status,
  files: merged[0].files_changed.length,
  cmds: merged[0].commands_run.length,
}}));"#
    ));
    assert_eq!(got, r#"{"status":"accepted","files":1,"cmds":4}"#);
}

/// The outcome's own evidence is authoritative when it has any: the item view
/// must never overwrite a verdict the branch actually reported.
#[test]
fn an_outcome_with_its_own_evidence_is_not_overwritten_by_its_item() {
    let batch = r#"{"data":{
      "outcomes":[{"status":"noop","files_changed":[],
                   "commands_run":[{"kind":"inspect"}]}],
      "items":[{"status":"accepted",
                "files_changed":[{"path":"src/other.txt"}],
                "commands_run":[{"kind":"a"},{"kind":"b"}]}]}}"#;
    let got = run_js(&format!(
        r#"const merged = outcomesOf({batch});
console.log(JSON.stringify({{
  status: merged[0].status,
  files: merged[0].files_changed.length,
  cmds: merged[0].commands_run.length,
}}));"#
    ));
    // status and the non-empty commands_run come from the outcome; the empty
    // files_changed is backfilled from the item rather than left blank.
    assert_eq!(got, r#"{"status":"noop","files":1,"cmds":1}"#);
}

/// The shape a v3 script is actually handed: the host spreads a fan-out's
/// `data` at the top level (`result_view_json_shaped`), so `outcomes` and
/// `items` are `batch.outcomes` / `batch.items` and there is no `data` key at
/// all. The fixtures above use the persisted-record shape, which the helper
/// also accepts; this is the one a live wave returns.
#[test]
fn a_live_envelope_with_top_level_arrays_is_joined_the_same_way() {
    let batch = r#"{
      "status":"accepted","summary":"fanout done",
      "outcomes":[{"status":"accepted","canonical_task_ids":["TASK-A"],
                   "files_changed":[],"commands_run":[]}],
      "items":[{"status":"accepted",
                "files_changed":[{"path":"src/produced.txt"}],
                "commands_run":[{"kind":"test"}]}],
      "result":{"status":"accepted","summary":"fanout done","data":{"peak_parallelism":1}}}"#;
    let got = run_js(&format!(
        r#"const merged = outcomesOf({batch});
console.log(JSON.stringify({{
  n: merged.length,
  task: merged[0].canonical_task_ids[0],
  files: merged[0].files_changed.length,
  cmds: merged[0].commands_run.length,
}}));"#
    ));
    assert_eq!(got, r#"{"n":1,"task":"TASK-A","files":1,"cmds":1}"#);
}
