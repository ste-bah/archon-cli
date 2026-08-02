//! Invariant 3 — ungated irreversible action.
//!
//! A tool call classified [`PermissionClass::Irreversible`] that no **passed**
//! gate dominates is blocked.
//!
//! # The narrowing
//!
//! [`TaskGraph::ungated_irreversible`](crate::ir::TaskGraph::ungated_irreversible)
//! evaluates gate *presence*, because a static graph has no notion of a gate
//! having been passed. This narrows it to gates recorded as passed in the
//! executed prefix, via
//! [`LiveTopology::on_gate_passed`](super::LiveTopology::on_gate_passed).
//!
//! **`StageKind::Checkpoint` therefore never gates**, because nothing anywhere
//! in the tree marks a checkpoint passed. That is intended and it is the
//! fail-safe direction; the tripwire comment in `lower_workflow.rs` explains
//! why it must stay that way. Specs persisted before the W6 deletion
//! deserialize a legacy `condition` stage to `Checkpoint`, and a condition
//! stage never had an evaluator — so counting checkpoint presence as gating
//! would let a condition nobody ever evaluated authorise a deploy.
//!
//! # The design defect this works around
//!
//! See [`GateEnforcement`](super::GateEnforcement). Read literally, with the
//! invariant on by default, this check blocks every irreversible action in
//! every plain session, because a plain session has no gate construct so no
//! gate can ever have passed. The default is therefore
//! [`GateEnforcement::WhereDeclared`]: enforce where the structure declares
//! gates, which is where an author expressed the intent this invariant
//! protects. The literal reading is available as
//! [`GateEnforcement::Always`].

use super::state::SessionState;
use super::verdict::{Invariant, ToolIntent, Verdict};
use super::{GateEnforcement, LiveTopologyConfig};
use crate::ir::PermissionClass;

/// Admit an irreversible tool call against the gates passed so far.
pub(super) fn admit_irreversible(
    state: &SessionState,
    config: LiveTopologyConfig,
    intent: &ToolIntent,
) -> Verdict {
    if intent.permission != PermissionClass::Irreversible {
        return Verdict::Allowed;
    }

    let structure = state.structure();
    match config.ungated_irreversible {
        GateEnforcement::Off => return Verdict::Allowed,
        GateEnforcement::WhereDeclared if !structure.declares_gates() => {
            return Verdict::Allowed;
        }
        GateEnforcement::WhereDeclared | GateEnforcement::Always => {}
    }

    let passed = state.gates_passed();

    // A node the declared graph knows is judged by its own dominators. A node
    // it does not know — a tool call in an undeclared prefix, or a spawn the
    // graph never mentioned — has an executed prefix that is a chain, so every
    // gate already passed in this session precedes it and therefore dominates
    // it. With nothing passed there is nothing to precede it and it is blocked.
    if !structure.knows(&intent.node_id) {
        return if passed.is_empty() {
            blocked(intent, &[], &passed)
        } else {
            Verdict::Allowed
        };
    }

    let dominators = structure.dominating_gates(&intent.node_id);
    if dominators.iter().any(|gate| passed.contains(gate)) {
        return Verdict::Allowed;
    }
    blocked(intent, dominators, &passed)
}

fn blocked(intent: &ToolIntent, dominators: &[String], passed: &[String]) -> Verdict {
    let required = if dominators.is_empty() {
        "no gate dominates it at all".to_string()
    } else {
        format!(
            "its dominating gate(s) [{}] have not passed",
            dominators.join(", ")
        )
    };
    let seen = if passed.is_empty() {
        "no gate has passed in this session".to_string()
    } else {
        format!("gates passed so far: [{}]", passed.join(", "))
    };
    Verdict::blocked(
        Invariant::UngatedIrreversible,
        format!(
            "ungated_irreversible: '{node}' cannot run '{tool}' — the call is classified \
             irreversible and {required}; {seen}. Pass the gate first, or route the \
             irreversible step downstream of one. A checkpoint stage does not count: nothing \
             marks a checkpoint passed.",
            node = intent.node_id,
            tool = intent.tool,
        ),
    )
}
