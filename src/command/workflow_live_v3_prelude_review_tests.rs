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

    const ITEM_TASK_IDS: &str = r#"{"review-task-tdl-010":"TASK-TDL-010","review-task-tdl-020":"TASK-TDL-020"}"#;

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
}
