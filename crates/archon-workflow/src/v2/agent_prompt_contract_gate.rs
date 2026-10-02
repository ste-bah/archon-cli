//! Whether a call is judged against a task's declared contract, and so must be
//! shown it.
//!
//! Decided from the call's ROLE, which the host already holds, and never from
//! a substring of its id. An id embeds user-chosen text (the canonical task id
//! the author put into the label), so a `contains("review")` test turned the
//! gate on for every call of a task whose id happened to hold that word and
//! left an identical task named differently without it: two calls doing the
//! same job were shown different contracts because of how a task was named.
//!
//! The role signals, in order:
//! - the method: `source.rs` derives `Implementation` for every write-capable
//!   branch (implementations and remediations alike, whatever the author
//!   called them), and `FinalReport` reads the whole run;
//! - the item kind the call declares (`focused_verification`, `review_map`,
//!   `noop_proof`, `implementation`), inherited by every branch of the call;
//! - a declared review or remediation contract, which only calls that judge a
//!   task's work carry;
//! - for the fixed-plan stages that declare none of the above, a match on the
//!   engine-generated PREFIX of the id, never on the whole string, and only
//!   for a call the host itself planned (`options.host_planned`), never for an
//!   authored v3 id, whose leading words are the author's label.

use crate::v2::{WorkflowV2HostCall, WorkflowV2HostMethod};

/// Item kinds whose calls are judged against, or judge, a task's contract.
const CONTRACT_ITEM_KINDS: &[&str] = &[
    "implementation",
    "focused_verification",
    "review_map",
    "noop_proof",
];

/// Declared option keys only a call judging a task's work carries.
const CONTRACT_OPTION_KEYS: &[&str] = &["reviewContract", "remediationContract"];

/// Engine-generated id prefixes of the review, verification, artifact and
/// completion-evidence stages that declare no item kind or contract of their
/// own. Matched at the START of the id, where the stage name sits, so a task
/// id embedded later in the id cannot turn the gate on.
const CONTRACT_STAGE_PREFIXES: &[&str] = &[
    "verification-",
    "post-remediation-verification-",
    "noop-proof-",
    "wave-completion-evidence-",
    "review-",
    "adversarial-review",
    "cross-cutting-review",
    "artifact-",
    "remediation-",
    "ownership-expansion-",
    "final-evidence-reconciliation-",
    "completion-claim-repair-",
];

pub(super) fn uses_task_contract_context(call: &WorkflowV2HostCall, base_call_id: &str) -> bool {
    matches!(
        call.method,
        WorkflowV2HostMethod::FinalReport | WorkflowV2HostMethod::Implementation
    ) || declares_contract_role(call)
        || (call.options.host_planned && planned_contract_stage(base_call_id))
}

/// A stage of the host's own plan that judges a task's work. Read only for a
/// host-planned call: an authored v3 id is `<label>-<ordinal>`, and a label
/// that happens to START with a stage name (`verification-queue-010`) is the
/// author's word, not the engine's.
fn planned_contract_stage(base_call_id: &str) -> bool {
    CONTRACT_STAGE_PREFIXES
        .iter()
        .any(|prefix| base_call_id.starts_with(prefix))
        || base_call_id == "final-zero-gap-audit"
}

fn declares_contract_role(call: &WorkflowV2HostCall) -> bool {
    call.options
        .item_kind
        .as_deref()
        .is_some_and(|kind| CONTRACT_ITEM_KINDS.contains(&kind))
        || CONTRACT_OPTION_KEYS
            .iter()
            .any(|key| call.options.extra.get(*key).is_some_and(|v| !v.is_null()))
}
