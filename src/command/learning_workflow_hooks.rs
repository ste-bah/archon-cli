//! Derive a generated run's `learning_hooks` from what the run is actually
//! about.
//!
//! # Why this exists
//!
//! `WorkflowScriptPlan::generated` hardcoded `learning_hooks: Vec::new()`. Every
//! generated run — which is every decomposed-PRD run — therefore reached
//! [`crate::command::topology_fold::workflow_learning`] with an empty hook list,
//! and an empty hook list dispatches nothing by design. The template path
//! (`from_template`) carried a spec's authored hooks through, so the bridge
//! worked there and only there: the surface that produces almost all real runs
//! learned nothing from any of them.
//!
//! A fixed default would have "fixed" it dishonestly — every run would claim to
//! want the same subsystems regardless of what it did. So the hooks are derived
//! from the run's own content, using the classifier the topology corpus already
//! keys on ([`archon_topology::classify_task`]). One classification, one hook
//! set, and the same text that buckets the run in the corpus decides what it
//! feeds back.
//!
//! # What is derivable
//!
//! Only `sona`, `reasoning_bank` and `desc` have a write-side consumer on
//! `LearningIntegration`. `world_model`, `jepa`, `rlm` and `reflexion` do not:
//! the fold counts them as unrouted and reports them. Deriving one of those
//! would be manufacturing an unrouted hook on purpose, so this module never
//! emits them — a spec may still *author* one and get the honest unrouted
//! report, but nothing here invents it.
//!
//! Every candidate is then filtered by its own `[learning.*]` toggle. An
//! operator who turned a subsystem off does not get it back because a task
//! description mentioned an audit. When every candidate is disabled the result
//! is empty, and empty still dispatches nothing.

use archon_core::config::LearningConfig;
use archon_topology::{TaskClass, classify_task};

use archon_workflow::task_universe::WorkflowV2TaskUniverse;

/// Cap on how much task-universe prose feeds the classifier.
///
/// The classifier tokenizes into a `BTreeSet`, so more text is monotonically
/// more marker hits and a large universe would drown the run's own description
/// in incidental vocabulary. Titles first, then acceptance criteria, both
/// bounded, keeps the signal on what the tasks *are*.
const MAX_CLASSIFIER_CHARS: usize = 8_000;

/// Hook names this module is allowed to emit, in the order they are reported.
///
/// Kept as an explicit list rather than derived from the config struct: the
/// contract is "these three have a consumer", and that is a fact about
/// `LearningIntegration`, not about which config keys happen to exist.
pub(crate) const DERIVABLE_HOOKS: &[&str] = &["sona", "reasoning_bank", "desc"];

/// Derive the hook list for a generated run.
pub(crate) fn derive_learning_hooks(
    task: &str,
    task_universe: Option<&WorkflowV2TaskUniverse>,
    learning: &LearningConfig,
) -> Vec<String> {
    let class = classify_generated_run(task, task_universe);
    let mut hooks = Vec::new();

    // SONA records a trajectory per agent outcome. A workflow run is a batch
    // run, not an interactive session, so `pipeline_recording` is the toggle
    // that actually consents to it — `enabled` alone covers interactive use.
    if learning.sona.enabled && learning.sona.pipeline_recording {
        hooks.push("sona".to_string());
    }

    // ReasoningBank answers "has this claim been made before, and how did it go".
    // That is worth a query when the work is diagnostic or judgemental — there
    // is a prior claim to check. Greenfield and refactor work has none, and
    // asking anyway costs a retrieval that can only return noise.
    if learning.reasoning_bank.enabled
        && matches!(
            class,
            TaskClass::BugHunt | TaskClass::Review | TaskClass::Migration
        )
    {
        hooks.push("reasoning_bank".to_string());
    }

    // DESC episodes are the part of the loop that actually closes today:
    // `build_fold_learning` constructs the episode store and nothing else, and
    // episodes written now are what `InjectionFilter` feeds into a later run.
    // Every class benefits from recording what happened, so this is gated only
    // on the operator's toggle.
    if learning.desc.enabled {
        hooks.push("desc".to_string());
    }

    // Structural, not decorative: nothing this module emits may be a name the
    // fold cannot route. A hook with no consumer is counted as unrouted and
    // reported, and manufacturing one on purpose would be noise.
    hooks.retain(|hook| DERIVABLE_HOOKS.contains(&hook.as_str()));
    hooks
}

/// The class of work a generated run represents.
pub(crate) fn classify_generated_run(
    task: &str,
    task_universe: Option<&WorkflowV2TaskUniverse>,
) -> TaskClass {
    classify_task(&classifier_text(task, task_universe))
}

fn classifier_text(task: &str, task_universe: Option<&WorkflowV2TaskUniverse>) -> String {
    let mut text = task.trim().to_string();
    let Some(universe) = task_universe else {
        return text;
    };
    for prose in universe.declared_prose() {
        push_bounded(&mut text, prose);
    }
    text
}

fn push_bounded(text: &mut String, addition: &str) {
    if text.len() >= MAX_CLASSIFIER_CHARS {
        return;
    }
    text.push(' ');
    text.push_str(addition.trim());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every toggle on, and SONA consenting to batch recording.
    fn all_enabled() -> LearningConfig {
        let mut learning = LearningConfig::default();
        learning.sona.enabled = true;
        learning.sona.pipeline_recording = true;
        learning.reasoning_bank.enabled = true;
        learning.desc.enabled = true;
        learning
    }

    #[test]
    fn a_review_run_asks_for_the_bank_and_a_greenfield_run_does_not() {
        let review = derive_learning_hooks(
            "audit and review the accepted tasks for contradictions",
            None,
            &all_enabled(),
        );
        let greenfield = derive_learning_hooks(
            "implement and scaffold a new provider ingest module",
            None,
            &all_enabled(),
        );
        assert_eq!(review, ["sona", "reasoning_bank", "desc"]);
        assert_eq!(greenfield, ["sona", "desc"]);
    }

    /// The bug the module exists to fix: a generated plan used to hardcode an
    /// empty list, so no generated run ever dispatched. A default-config run
    /// must now name at least the one subsystem the fold actually constructs.
    #[test]
    fn a_default_config_run_derives_a_non_empty_hook_list() {
        let hooks = derive_learning_hooks(
            "implement the decomposed PRD tasks",
            None,
            &LearningConfig::default(),
        );
        assert_eq!(
            hooks,
            ["desc"],
            "default config disables sona batch recording but keeps desc"
        );
    }

    /// An operator's toggles win over the classifier. All off must be empty,
    /// and empty must stay empty — the fold dispatches nothing for it.
    #[test]
    fn disabled_subsystems_are_never_derived_and_all_off_is_empty() {
        let mut learning = all_enabled();
        learning.sona.pipeline_recording = false;
        learning.reasoning_bank.enabled = false;
        learning.desc.enabled = false;
        assert!(
            derive_learning_hooks("audit and review everything", None, &learning).is_empty(),
            "no candidate may survive its own toggle being off"
        );
    }

    /// Nothing this module emits may be a hook the fold cannot route.
    #[test]
    fn only_hooks_with_a_consumer_are_ever_derived() {
        for task in [
            "audit the data lake",
            "migrate the registry schema",
            "fix the failing crash",
            "refactor and simplify the store",
            "build a new command",
        ] {
            for hook in derive_learning_hooks(task, None, &all_enabled()) {
                assert!(
                    DERIVABLE_HOOKS.contains(&hook.as_str()),
                    "derived unroutable hook '{hook}' for task '{task}'"
                );
            }
        }
    }
}
