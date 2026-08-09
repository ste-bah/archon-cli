//! Review attribution and remediation budget.

#[cfg(test)]
mod review_attribution_tests {
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
}

#[cfg(test)]
mod remediation_budget_tests {
    /// Runs the REAL prelude JS, not a Rust reimplementation of it.
    ///
    /// Every other prelude guard in this file asserts on source text, which
    /// catches drift but cannot catch wrong behaviour. The budget is arithmetic
    /// over gap sets, so it is worth executing: a source assertion would have
    /// passed on a rule that funded exactly the wrong task.
    fn run_budget_js(driver: &str) -> String {
        let prelude = super::super::V3_PRIMITIVES_JS;
        let start = prelude
            .find("  const remediationBudget = (opts = {}) => {")
            .expect("remediationBudget must exist");
        let end = start
            + prelude[start..]
                .find("\n  };\n")
                .expect("remediationBudget end")
            + 5;
        let script = format!("{}\n{driver}\n", &prelude[start..end]);
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("budget.mjs");
        std::fs::write(&path, script).expect("write driver");
        let out = std::process::Command::new("node")
            .arg(&path)
            .output()
            .expect("node must be available; the tests already shell out to zsh and python3");
        assert!(
            out.status.success(),
            "driver failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn envelope(gap_ids: &[&str]) -> String {
        let gaps = gap_ids
            .iter()
            .map(|id| format!(r#"{{"id":"{id}"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        format!(r#"{{"result":{{"residual_gaps":[{gaps}]}}}}"#)
    }

    /// The observed TDL-040 sequence: 5 gaps, one closed, then flat.
    /// A raw-count rule keeps funding it; this must stop on the plateau.
    #[test]
    fn budget_extends_while_the_original_diagnosis_shrinks_and_stops_on_a_plateau() {
        let a1 = envelope(&[
            "registry-missing",
            "paging",
            "mcp-path",
            "tui-alias",
            "filter-mismatch",
        ]);
        let a2 = envelope(&["paging", "mcp-path", "tui-alias", "zero-match"]);
        let a3 = envelope(&["paging", "mcp-path", "tui-alias", "zero-test-noise"]);
        let driver = format!(
            r#"const b = remediationBudget();
console.log([b.shouldContinue(1, {a1}), b.shouldContinue(2, {a2}), b.shouldContinue(3, {a3})].join(","));"#
        );
        // 1,2 are inside the base budget; 3 is the plateau and must stop.
        assert_eq!(run_budget_js(&driver), "true,true,false");
    }

    /// Churned gaps were never in the baseline, so they cannot buy budget.
    /// This is what removes the need to classify substantive vs incidental.
    #[test]
    fn gaps_absent_from_the_baseline_cannot_earn_budget() {
        let a1 = envelope(&["real-one"]);
        let a2 = envelope(&["real-one", "churn-a"]);
        let a3 = envelope(&["real-one", "churn-b"]);
        let driver = format!(
            r#"const b = remediationBudget();
b.shouldContinue(1, {a1}); b.shouldContinue(2, {a2});
console.log(b.shouldContinue(3, {a3}));"#
        );
        assert_eq!(run_budget_js(&driver), "false");
    }

    /// Branch-suffixed ids cannot match across attempts and would read as
    /// perpetual churn, so they are excluded rather than silently normalised.
    #[test]
    fn branch_suffixed_gap_ids_are_excluded_from_the_baseline() {
        let a1 = envelope(&["invalid_write_branch_output_review-remediate-task-tdl-020-2-63-0"]);
        let driver = format!(
            r#"const b = remediationBudget();
b.shouldContinue(1, {a1});
// Baseline is empty, so nothing can be "still closing": it must not extend
// past the base budget on the strength of an unmatchable id.
console.log(b.shouldContinue(3, {a1}));"#
        );
        assert_eq!(run_budget_js(&driver), "false");
    }

    /// The envelope the host produces when schema repair failed but the patch
    /// was confirmed landed against the declared baseline.
    fn schema_landed_envelope() -> String {
        r#"{"result":{"data":{"schema_repair_patch_landed":true},"residual_gaps":[]}}"#.to_string()
    }

    /// An attempt that died to schema repair WITH a landed patch produced work
    /// and no verdict, so it must not be charged to the task. Attempt 3 is
    /// refused on the flat budget; with the refund it is funded.
    #[test]
    fn a_schema_failure_with_a_landed_patch_buys_one_extra_attempt() {
        let none = envelope(&[]);
        let landed = schema_landed_envelope();
        let driver = format!(
            r#"const plain = remediationBudget();
const refunded = remediationBudget();
// Same call on both, except the refunded one also saw a landed-patch impl.
console.log([
  plain.shouldContinue(1, {none}),
  plain.shouldContinue(3, {none}),
  refunded.shouldContinue(1, {none}, {landed}),
  refunded.shouldContinue(3, {none}),
].join(","));"#
        );
        // plain: funded at 1, refused at 3. refunded: funded at 1 AND at 3.
        assert_eq!(run_budget_js(&driver), "true,false,true,true");
    }

    /// The bound is the entire safety argument. Schema repair already retries
    /// under its own cap, so an unbounded exemption turns a burned attempt into
    /// a hung task. An agent that lands a patch and emits garbage EVERY time
    /// must still run out.
    #[test]
    fn the_schema_refund_is_bounded_to_once_per_task() {
        let none = envelope(&[]);
        let landed = schema_landed_envelope();
        let driver = format!(
            r#"const b = remediationBudget();
// Four consecutive landed-patch schema failures. Only the first may pay out,
// so funding reaches base+1 = 4 and no further.
b.shouldContinue(1, {none}, {landed});
console.log([
  b.shouldContinue(3, {none}, {landed}),
  b.shouldContinue(4, {none}, {landed}),
  b.shouldContinue(5, {none}, {landed}),
].join(","));"#
        );
        // 3 < 4 funded; 4 and 5 are not — the refund did not compound.
        assert_eq!(run_budget_js(&driver), "true,false,false");
    }

    /// Without the marker nothing changes. Guards the refund against becoming a
    /// blanket extra attempt for every task — which would quietly raise the
    /// budget for the stuck tasks the progress rule exists to cut off.
    #[test]
    fn an_envelope_without_the_marker_earns_no_refund() {
        let none = envelope(&[]);
        // Shaped like the marker but false, and a look-alike key: neither pays.
        let decoys = r#"{"result":{"data":{"schema_repair_patch_landed":false,"schema_repair_patch_landed_maybe":true}}}"#;
        let driver = format!(
            r#"const b = remediationBudget();
b.shouldContinue(1, {none}, {decoys});
console.log(b.shouldContinue(3, {none}, {decoys}));"#
        );
        assert_eq!(run_budget_js(&driver), "false");
    }

    /// A verifier that named nothing gives nothing to measure; fall back to the
    /// flat budget rather than inventing progress from an empty set.
    #[test]
    fn an_empty_first_verdict_falls_back_to_the_flat_budget() {
        let none = envelope(&[]);
        let driver = format!(
            r#"const b = remediationBudget();
console.log([b.shouldContinue(1, {none}), b.shouldContinue(3, {none})].join(","));"#
        );
        assert_eq!(run_budget_js(&driver), "true,false");
    }

    /// The exact call a generated script emitted, against an explicit
    /// instruction not to. `hardCap: 3` makes `ceiling === funded`, so the
    /// progress check never runs and a converging task is cut at three.
    /// Enforced here because prompt text demonstrably did not prevent it.
    #[test]
    fn a_script_supplied_fixed_bound_cannot_disable_the_progress_check() {
        // Gap set shrinks every attempt: 3 -> 2 -> 1 of the baseline remain.
        let a1 = envelope(&["gap-a", "gap-b", "gap-c"]);
        let a2 = envelope(&["gap-a", "gap-b"]);
        let a3 = envelope(&["gap-a"]);
        let a4 = envelope(&[]);
        let driver = format!(
            r#"const b = remediationBudget({{ baseAttempts: 3, hardCap: 3, maxSchemaRefunds: 0 }});
b.shouldContinue(1, {a1}); b.shouldContinue(2, {a2}); b.shouldContinue(3, {a3});
console.log(b.shouldContinue(4, {a4}));"#
        );
        // Attempt 4 must be funded: the diagnosis is still closing, and the
        // floor keeps the ceiling at 6 regardless of the requested 3.
        assert_eq!(run_budget_js(&driver), "true");
    }

    /// The floor must not turn into "never stop". A plateau still ends the
    /// budget — widening the ceiling only funds attempts that are converging.
    #[test]
    fn the_hard_cap_floor_still_stops_on_a_plateau() {
        let a1 = envelope(&["gap-a", "gap-b"]);
        let flat = envelope(&["gap-a", "gap-b"]);
        let driver = format!(
            r#"const b = remediationBudget({{ hardCap: 3 }});
b.shouldContinue(1, {a1}); b.shouldContinue(2, {flat}); b.shouldContinue(3, {flat});
console.log(b.shouldContinue(4, {flat}));"#
        );
        assert_eq!(run_budget_js(&driver), "false");
    }

    /// `maxSchemaRefunds: 0` switched off a refund whose safety argument is the
    /// once-per-task bound, not the ability to disable it. Floored at 1.
    #[test]
    fn a_script_cannot_switch_off_the_schema_refund() {
        let none = envelope(&[]);
        let landed = schema_landed_envelope();
        let driver = format!(
            r#"const b = remediationBudget({{ baseAttempts: 2, hardCap: 2, maxSchemaRefunds: 0 }});
b.shouldContinue(1, {none}, {landed});
console.log(b.shouldContinue(2, {none}, {landed}));"#
        );
        // Without the floor this is false at the flat bound; the refund funds it.
        assert_eq!(run_budget_js(&driver), "true");
    }

    /// Widening is still the caller's to do — the floor is a floor, not a pin.
    #[test]
    fn a_script_may_still_widen_the_budget() {
        let a1 = envelope(&["gap-a", "gap-b", "gap-c"]);
        let a2 = envelope(&["gap-a", "gap-b"]);
        let driver = format!(
            r#"const b = remediationBudget({{ baseAttempts: 8 }});
b.shouldContinue(1, {a1});
console.log(b.shouldContinue(7, {a2}));"#
        );
        // Inside a caller-widened base window, funded without needing progress.
        assert_eq!(run_budget_js(&driver), "true");
    }
}
