//! Review attribution: routing map findings back to the task whose branch
//! produced them, and keeping that attribution across the reduce.

/// Pull one named arrow-function definition out of the prelude by name.
///
/// Sliced to the function's real end rather than a fixed window — a magic
/// width has already made a test in this file report failure on correct
/// code twice.
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

fn run_review_js(driver: &str) -> String {
    let mut script = String::new();
    for name in [
        "findingsFrom",
        "taskIdsOfOutcome",
        "stampTaskIds",
        "attributedMapFindings",
        "findingIdentities",
        "reattributeFindings",
        "findingsByTask",
    ] {
        script.push_str(&prelude_fn(name));
        script.push('\n');
    }
    script.push_str(driver);
    script.push('\n');
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("review.mjs");
    std::fs::write(&path, script).expect("write driver");
    let out = std::process::Command::new("node")
        .arg(&path)
        .output()
        .expect("node must be available; these tests already shell out to zsh and python3");
    assert!(
        out.status.success(),
        "driver failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A fanout map envelope whose findings carry NO task key of any kind —
/// the exact shape all 43 adversarial findings had on run wf-ee4a92fc.
const UNATTRIBUTED_MAP: &str = r#"{"data":{"outcomes":[
      {"item_id":"review-task-tdl-010","result":{"data":{"findings":[
        {"id":"F1","claim":"registry write is not atomic","severity":"high"},
        {"id":"F2","claim":"no fsync on manifest","severity":"medium"}]}}},
      {"item_id":"review-task-tdl-020","result":{"data":{"findings":[
        {"id":"F3","claim":"validation report omits gaps","severity":"high"}]}}}
    ]}}"#;

const ITEM_TASK_IDS: &str =
    r#"{"review-task-tdl-010":"TASK-TDL-010","review-task-tdl-020":"TASK-TDL-020"}"#;

/// The headline defect: without stamping, every finding routes to
/// `unassigned` and `remediateFindings` returns them untouched.
#[test]
fn map_findings_are_attributed_to_the_task_whose_branch_produced_them() {
    let driver = format!(
        r#"const stamped = attributedMapFindings({UNATTRIBUTED_MAP}, {ITEM_TASK_IDS});
const {{ grouped, unassigned }} = findingsByTask(stamped);
console.log(JSON.stringify({{
  total: stamped.length,
  unassigned: unassigned.length,
  tdl010: (grouped["TASK-TDL-010"] || []).length,
  tdl020: (grouped["TASK-TDL-020"] || []).length
}}));"#
    );
    assert_eq!(
        run_review_js(&driver),
        r#"{"total":3,"unassigned":0,"tdl010":2,"tdl020":1}"#
    );
}

/// A reviewer that DID name its tasks must keep exactly what it named — a
/// cross-task finding naming two tasks must not be collapsed to the one
/// branch it happened to surface in.
#[test]
fn reviewer_supplied_attribution_is_never_overwritten() {
    let map = r#"{"data":{"outcomes":[{"item_id":"review-task-tdl-010","result":{"data":{"findings":[
          {"id":"F1","claim":"shared invariant broken","canonical_task_ids":["TASK-TDL-010","TASK-TDL-020"]}]}}}]}}"#;
    let driver = format!(
        r#"const stamped = attributedMapFindings({map}, {ITEM_TASK_IDS});
console.log(JSON.stringify(stamped[0].canonical_task_ids));"#
    );
    assert_eq!(run_review_js(&driver), r#"["TASK-TDL-010","TASK-TDL-020"]"#);
}

/// `preserveMapFindings` is an instruction to a model, not a guarantee. A
/// reduce that returns the same findings stripped of attribution must be
/// repaired from the stamped map set rather than silently losing routing.
#[test]
fn a_reduce_that_drops_attribution_is_repaired_by_identity() {
    let reduce = r#"{"data":{"findings":[
          {"id":"F1","claim":"registry write is not atomic","severity":"high"},
          {"id":"F3","claim":"validation report omits gaps","severity":"high"}]}}"#;
    let driver = format!(
        r#"const stamped = attributedMapFindings({UNATTRIBUTED_MAP}, {ITEM_TASK_IDS});
const repaired = reattributeFindings(findingsFrom({reduce}), stamped);
console.log(JSON.stringify(repaired.map((f) => f.canonical_task_ids)));"#
    );
    assert_eq!(
        run_review_js(&driver),
        r#"[["TASK-TDL-010"],["TASK-TDL-020"]]"#
    );
}

/// A finding the primitive genuinely cannot place must still be RETURNED.
/// Dropping it would trade a silent routing failure for a silent data loss.
#[test]
fn findings_from_an_unmappable_branch_are_kept_unattributed() {
    let map = r#"{"data":{"outcomes":[{"item_id":"review-unknown-item","result":{"data":{"findings":[
          {"id":"F9","claim":"orphan finding"}]}}}]}}"#;
    let driver = format!(
        r#"const stamped = attributedMapFindings({map}, {ITEM_TASK_IDS});
const {{ unassigned }} = findingsByTask(stamped);
console.log(JSON.stringify({{ kept: stamped.length, unassigned: unassigned.length }}));"#
    );
    assert_eq!(run_review_js(&driver), r#"{"kept":1,"unassigned":1}"#);
}

/// The five findings that actually survived run `wf-ee4a92fc`, replayed in
/// the shape they came back in.
///
/// Reproduced from the `PRIOR-RUN-FINDINGS` block the user's own
/// `TASK-TDL-001` task file now carries: `claim`, `counter_evidence`, `id`,
/// `severity`, `source`, `type`, `evidence`, `impact`, `status`, `verdict` —
/// and no task key of any kind. That is why 100% of that run's 43
/// adversarial findings landed in `unassigned` and `remediateFindings`
/// returned every one of them untouched.
const PRIOR_RUN_FINDINGS: &str = r#"{"data":{"outcomes":[
      {"item_id":"review-task-tdl-001","result":{"data":{"findings":[
        {"id":"F1","type":"adversarial review","severity":"high","status":"open","verdict":"unresolved",
         "claim":"Accepted verification treats 170 unique IDs as satisfying normative requirement mapping.",
         "counter_evidence":"Task requires mapping current code and missing implementation to every normative PRD requirement; artifact sample has repeated generic evidence.",
         "evidence":"artifact sample","impact":"acceptance is unearned",
         "source":"task:51-56; gap-audit-current.json:366-390"},
        {"id":"F2","type":"adversarial review","severity":"medium","status":"open","verdict":"unresolved",
         "claim":"Implementation wrote/generated the audit artifact.",
         "counter_evidence":"Recorded generation command contains only a comment, not the generator logic; patch file is zero-line/empty.",
         "evidence":"commands_run[5]","impact":"provenance unproven",
         "source":"implement result commands_run[5]; patch read error"},
        {"id":"F3","type":"adversarial review","severity":"medium","status":"open","verdict":"unresolved",
         "claim":"Verification result has residual_gaps: [].",
         "counter_evidence":"Implementation result and artifact record four fail-closed residual gaps.",
         "evidence":"verification-wave result","impact":"gaps hidden from the report",
         "source":"verification-wave result; gap-audit-current.json:2468-2493"},
        {"id":"F4","type":"adversarial review","severity":"medium","status":"open","verdict":"unresolved",
         "claim":"Storage contract paths are present.",
         "counter_evidence":"Artifact marks wildcard/template paths as present with 'Observed or contract-required', not direct physical evidence.",
         "evidence":"artifact rows","impact":"contract satisfied on paper only",
         "source":"gap-audit-current.json:2527-2567"},
        {"id":"F5","type":"adversarial review","severity":"low","status":"open","verdict":"unresolved",
         "claim":"No files changed / source tree clean proves task safety.",
         "counter_evidence":"The deliverable is a project artifact outside repository source tracking, so git status does not prove artifact diff integrity.",
         "evidence":"git status","impact":"safety argument does not apply",
         "source":"implementation and verification commands_run"}]}}}
    ]}}"#;

const PRIOR_RUN_ITEM_IDS: &str = r#"{"review-task-tdl-001":"TASK-TDL-001"}"#;

/// All five must now route to the task instead of `unassigned`, with their
/// declared fields intact — remediation reads `claim` and `counter_evidence`
/// to know what to fix.
#[test]
fn the_five_surviving_prior_run_findings_attribute_to_their_task() {
    let driver = format!(
        r#"const stamped = attributedMapFindings({PRIOR_RUN_FINDINGS}, {PRIOR_RUN_ITEM_IDS});
const {{ grouped, unassigned }} = findingsByTask(stamped);
const mine = grouped["TASK-TDL-001"] || [];
console.log(JSON.stringify({{
  total: stamped.length,
  unassigned: unassigned.length,
  attributed: mine.length,
  ids: mine.map((f) => f.id),
  keptClaim: mine.every((f) => typeof f.claim === "string" && f.claim.length > 0),
  keptCounter: mine.every((f) => typeof f.counter_evidence === "string"),
  keptSource: mine.every((f) => typeof f.source === "string")
}}));"#
    );
    assert_eq!(
        run_review_js(&driver),
        r#"{"total":5,"unassigned":0,"attributed":5,"ids":["F1","F2","F3","F4","F5"],"keptClaim":true,"keptCounter":true,"keptSource":true}"#
    );
}

/// Re-attachment after the reduce must survive a reducer that RENAMES the
/// attribution field rather than dropping it.
///
/// `preserveMapFindings` is a model instruction, and a model that renames
/// `canonical_task_ids` to `task_ids`, `taskIds` or a scalar `task_id` has
/// not violated anything it was told. Each alias must still route, and a
/// reducer that drops the field entirely must be repaired by identity.
#[test]
fn post_reduce_reattachment_survives_a_reducer_that_renames_the_field() {
    // F1 keeps attribution under a renamed key, F2 under a second alias, F3
    // as a bare scalar, F4/F5 lose it completely.
    let reduce = r#"{"data":{"findings":[
          {"id":"F1","claim":"Accepted verification treats 170 unique IDs as satisfying normative requirement mapping.","task_ids":["TASK-TDL-001"]},
          {"id":"F2","claim":"Implementation wrote/generated the audit artifact.","taskIds":["TASK-TDL-001"]},
          {"id":"F3","claim":"Verification result has residual_gaps: [].","task_id":"TASK-TDL-001"},
          {"id":"F4","claim":"Storage contract paths are present."},
          {"id":"F5","claim":"No files changed / source tree clean proves task safety."}]}}"#;
    let driver = format!(
        r#"const stamped = attributedMapFindings({PRIOR_RUN_FINDINGS}, {PRIOR_RUN_ITEM_IDS});
const repaired = reattributeFindings(findingsFrom({reduce}), stamped);
const {{ grouped, unassigned }} = findingsByTask(repaired);
console.log(JSON.stringify({{
  unassigned: unassigned.length,
  attributed: (grouped["TASK-TDL-001"] || []).map((f) => f.id)
}}));"#
    );
    assert_eq!(
        run_review_js(&driver),
        r#"{"unassigned":0,"attributed":["F1","F2","F3","F4","F5"]}"#
    );
}
