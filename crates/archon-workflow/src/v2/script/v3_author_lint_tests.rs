//! The hand-rolled status predicate lint: flags the live defect, passes the
//! prelude-delegating fixture and every reporting helper.

use super::*;

const LIVE_STYLE_PREDICATE: &str = r#"export const meta = { name: 'x', description: 'y', phases: [] }
function isAccepted(env) {
  if (!env) return false
  if (env.status === 'noop') return true
  if (env.status !== 'accepted') return false
  const files = Array.isArray(env.files_changed) ? env.files_changed.length : 0
  const cmds = Array.isArray(env.commands_run) ? env.commands_run.length : 0
  return files > 0 || cmds > 0
}
function summarize(env) {
  return String((env && env.summary) || JSON.stringify(env.commands_run || [])).slice(0, 600)
}
"#;

#[test]
fn the_live_scripts_is_accepted_is_a_defect() {
    let defects =
        hand_rolled_predicate_defects(archon_test_support::fixtures::WF719F_AUTHORED_WORKFLOW_JS);
    assert_eq!(defects.len(), 1, "{defects:?}");
    let defect = &defects[0];
    assert!(defect.contains("`isAccepted`"), "{defect}");
    assert!(defect.contains(".commands_run"), "{defect}");
    assert!(defect.contains(".files_changed"), "{defect}");
    assert!(
        defect.contains("accepted(env)") && defect.contains("usable(env)"),
        "{defect}"
    );
    assert!(
        defect.contains("if (usable(impl) && accepted(check))"),
        "{defect}"
    );
}

#[test]
fn a_status_predicate_over_the_arrays_is_flagged_and_reporting_helpers_are_not() {
    let defects = hand_rolled_predicate_defects(LIVE_STYLE_PREDICATE);
    assert_eq!(defects.len(), 1, "{defects:?}");
    assert!(defects[0].contains("`isAccepted`"));
    assert!(!defects[0].contains("summarize"));
}

#[test]
fn the_reference_fixture_that_delegates_to_the_prelude_passes() {
    let source = include_str!("../../../tests/fixtures/write-wave-synthetic/workflow.js");
    assert!(
        source.contains("function isAccepted(env)"),
        "fixture still defines the wrapper"
    );
    assert_eq!(hand_rolled_predicate_defects(source), Vec::<String>::new());
}

#[test]
fn the_reference_example_and_the_prelude_pass() {
    assert_eq!(
        hand_rolled_predicate_defects(V3_PRIMITIVE_REFERENCE),
        Vec::<String>::new()
    );
    // The prelude's `usable` reads both arrays beside `.status` — it IS the
    // host's rule. It is never handed to the lint, and this pins that the lint
    // would refuse it if it were, so the boundary is not accidental.
    assert!(!hand_rolled_predicate_defects(V3_PRIMITIVES_JS).is_empty());
}

#[test]
fn arrow_and_expression_forms_are_covered() {
    let block_arrow = "const implUsable = (env) => {\n  return env.status === 'accepted' && env.files_changed.length > 0\n}\n";
    assert_eq!(hand_rolled_predicate_defects(block_arrow).len(), 1);
    let expression_arrow = "const checkOk = env => env && env.status === 'accepted' && (env.commands_run || []).length > 0\n";
    assert_eq!(hand_rolled_predicate_defects(expression_arrow).len(), 1);
    let function_expression = "let verifyPassed = function (env) { return env.status !== 'failed' && env.commands_run.length }\n";
    assert_eq!(hand_rolled_predicate_defects(function_expression).len(), 1);
    let succeeded = "function implSucceeded(env) { return env.status === 'accepted' && env.commands_run.length > 0 }\n";
    assert_eq!(hand_rolled_predicate_defects(succeeded).len(), 1);
}

#[test]
fn narrowness_status_alone_or_arrays_alone_or_other_names_pass() {
    // A status-only predicate re-derives nothing about the arrays.
    let status_only = "function isAccepted(env) { return !!env && (env.status === 'accepted' || env.status === 'noop') }\n";
    assert!(hand_rolled_predicate_defects(status_only).is_empty());
    // Evidence helpers named for what they collect, not for a verdict.
    let helpers = concat!(
        "function boundedEvidenceFor(id) { const env = finalImpl[id]; return [{ status: env.status, files: env.files_changed.slice(0, 20), commands: env.commands_run.map((c) => c.command) }] }\n",
        "function lookupTask(env) { return env.status && env.files_changed }\n",
        "const tokenBudget = (env) => env.status ? env.commands_run.length : 0\n",
        "function hookFor(env) { return env.status === 'accepted' ? env.files_changed : [] }\n",
        "function remediationEvidence(env) { return JSON.stringify({ status: env.status, commands: env.commands_run }) }\n",
    );
    assert_eq!(hand_rolled_predicate_defects(helpers), Vec::<String>::new());
    // Reads through the prelude's own names are not definitions.
    let uses = "const done = usable(impl) && accepted(check)\nif (accepted(check)) acceptedTaskIds.push(t.id)\n";
    assert!(hand_rolled_predicate_defects(uses).is_empty());
}

#[test]
fn braces_inside_strings_do_not_end_the_body_early() {
    let source = "function isOk(env) {\n  const label = 'open {'\n  return env.status === 'accepted' && env.files_changed.length > 0\n}\n";
    assert_eq!(hand_rolled_predicate_defects(source).len(), 1);
}

#[tokio::test]
async fn the_draft_preflight_rejects_a_hand_rolled_predicate_and_passes_the_prelude_form() {
    let fixture = include_str!("../../../tests/fixtures/write-wave-synthetic/workflow.js");
    let expected = ["TASK-SYN-010", "TASK-SYN-020"]
        .iter()
        .map(|id| id.to_string())
        .collect::<std::collections::BTreeSet<_>>();
    validate_authored_draft(fixture, &expected)
        .await
        .expect("the prelude-delegating fixture passes the draft pre-flight");

    let hand_rolled = fixture.replace(
        "function isAccepted(env) {\n  if (typeof accepted === 'function') return accepted(env)\n  return !!(env && (env.status === 'accepted' || env.status === 'noop'))\n}",
        "function isAccepted(env) {\n  if (!env || env.status !== 'accepted') return env && env.status === 'noop'\n  return (env.files_changed || []).length > 0 || (env.commands_run || []).length > 0\n}",
    );
    assert_ne!(
        hand_rolled, fixture,
        "the fixture's wrapper must be the text this test rewrites"
    );
    let error = validate_authored_draft(&hand_rolled, &expected)
        .await
        .expect_err("a hand-rolled predicate is a pre-flight defect");
    assert!(
        error.contains("own status predicate `isAccepted`"),
        "{error}"
    );
    assert!(error.contains("usable(impl) && accepted(check)"), "{error}");
}
