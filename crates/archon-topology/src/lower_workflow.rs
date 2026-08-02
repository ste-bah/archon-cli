//! `WorkflowSpec` → [`TaskGraph`].
//!
//! The richest of the three surfaces and the only one that is near-lossless:
//! stage kinds carry role, `expected_target_files` carries writes, and a
//! fan-out `foreach` carries the sole piece of dataflow anywhere in the tree.
//!
//! Read-only. Lowering never mutates or re-validates the spec — cycles and
//! fan-out contracts stay `WorkflowSpec::validate`'s business, and the same
//! defects surface again from [`TaskGraph::waves`].

use archon_workflow::spec::{StageKind, StageSpec, WorkflowSpec};

use crate::ir::{
    DataRef, FanoutSpec, GraphBudget, GraphOrigin, NodeRole, PermissionClass, TaskGraph, TaskNode,
    WriteTarget,
};

/// Lower a validated (or unvalidated) spec into the IR.
///
/// Infallible by construction: every structural defect a spec can carry is a
/// defect of the resulting graph too, and is reported by [`TaskGraph::waves`]
/// or [`TaskGraph::validate`] rather than swallowed here.
#[must_use]
pub fn lower_workflow_spec(spec: &WorkflowSpec, run_id: impl Into<String>) -> TaskGraph {
    let run_id = run_id.into();
    TaskGraph {
        id: run_id.clone(),
        origin: GraphOrigin::Workflow { run_id },
        nodes: spec
            .stages
            .iter()
            .map(|stage| lower_stage(spec, stage))
            .collect(),
        budget: GraphBudget {
            max_parallelism: spec.max_parallelism,
            max_agents: spec.max_agents,
            // WorkflowSpec has no loop construct, so every graph is one round.
            max_rounds: 1,
        },
    }
}

fn lower_stage(spec: &WorkflowSpec, stage: &StageSpec) -> TaskNode {
    let fanout = lower_fanout(stage);
    TaskNode {
        id: stage.id.clone(),
        role: lower_role(stage.kind),
        depends_on: stage.depends_on.clone(),
        // The `foreach` source is the only interpolation that exists today, so
        // it is the only dataflow milestone 1 can recover. Every other stage
        // lowers with `consumes` empty, which means *unknown* — see
        // `TaskNode::consumes`.
        consumes: fanout
            .as_ref()
            .and_then(|fanout| fanout.source.clone())
            .into_iter()
            .collect(),
        writes: stage
            .expected_target_files
            .iter()
            .map(|path| WriteTarget::Path(path.clone()))
            .collect(),
        permission: lower_permission(spec, stage),
        agent: stage.agent.clone(),
        fanout,
    }
}

/// Stage kind → node role, per the milestone 1 mapping table.
///
/// `StageKind::Condition` no longer exists: finding W6 established that
/// `StageSpec::condition` had no evaluator anywhere in the tree, so such a
/// stage never branched, and the variant was removed along with the field.
/// Specs persisted before that removal deserialize `condition` to `Checkpoint`,
/// which is behaviour-preserving for execution — an unevaluated condition
/// always proceeded — but note the consequence here: those legacy stages lower
/// to `NodeRole::Gate(Checkpoint)` and are indistinguishable from a real
/// checkpoint, because the discriminating field is erased at deserialize.
/// [`TaskGraph::ungated_irreversible`] will therefore count one as gating.
/// Milestone 3 narrows that relation to gates actually *passed* in the executed
/// prefix, which neutralises it — a checkpoint never presented is never passed.
fn lower_role(kind: StageKind) -> NodeRole {
    use crate::ir::GateKind;
    match kind {
        StageKind::Agent | StageKind::Implementation | StageKind::Fanout => NodeRole::Work,
        StageKind::Reduce => NodeRole::Reduce,
        StageKind::QualityGate => NodeRole::Verify,
        StageKind::HumanGate => NodeRole::Gate(GateKind::Human),
        StageKind::Checkpoint => NodeRole::Gate(GateKind::Checkpoint),
        StageKind::Tool => NodeRole::Tool,
    }
}

fn lower_fanout(stage: &StageSpec) -> Option<FanoutSpec> {
    if stage.kind != StageKind::Fanout {
        return None;
    }
    Some(FanoutSpec {
        source: stage
            .foreach
            .as_deref()
            .and_then(parse_foreach_accessor)
            .map(|(producer, accessor)| DataRef::new(producer, accessor)),
        max_parallelism: stage.max_parallelism,
    })
}

/// Parse `${producer.accessor}`.
///
/// Reimplemented rather than imported: `archon_workflow::spec`'s copy is
/// `pub(crate)`. Kept byte-compatible with it, including the trimming.
fn parse_foreach_accessor(foreach: &str) -> Option<(&str, &str)> {
    let inner = foreach.trim().strip_prefix("${")?.strip_suffix('}')?;
    let (producer, accessor) = inner.split_once('.')?;
    let producer = producer.trim();
    let accessor = accessor.trim();
    if producer.is_empty() || accessor.is_empty() {
        return None;
    }
    Some((producer, accessor))
}

/// Keys carrying the level inside an object-shaped permissions entry.
///
/// `level` is canonical — it is the name of the thing being declared. `class`
/// and `permission` are accepted aliases, retained because they were among the
/// shapes milestone 1's guesswork already read and specs may exist that use
/// them.
const LEVEL_KEYS: [&str; 3] = ["level", "class", "permission"];

/// Keys that declare a level for every stage that has no entry of its own.
const BLANKET_KEYS: [&str; 2] = ["default", "*"];

/// `WorkflowSpec::permissions` → [`PermissionClass`], per the declared format.
///
/// Milestone 1 read this field with no schema to conform to and guessed. The
/// format is now defined — see [`crate::permission`] — and grounded in
/// `archon_tools::tool::PermissionLevel`, the enum admission already keys off.
/// The map is read as: the stage's own id, then the blanket `default` (alias
/// `*`); the value is the level as a string, or an object carrying it under
/// `level` / `class` / `permission`.
///
/// Anything unrecognised lowers to `Safe` — including a *present but
/// unparseable* per-stage entry, which does **not** fall through to `default`.
/// A stage that says something about itself has spoken, even if unintelligibly,
/// and inheriting a blanket `dangerous` from a typo would fail closed. Milestone
/// 3 requires enforcement never to fail closed on a bookkeeping gap. Rejecting
/// typos belongs in a validator, where failing loudly costs nothing;
/// [`crate::permission::is_declared_permission`] is there for one.
fn lower_permission(spec: &WorkflowSpec, stage: &StageSpec) -> PermissionClass {
    spec.permissions
        .get(&stage.id)
        .or_else(|| {
            BLANKET_KEYS
                .iter()
                .find_map(|key| spec.permissions.get(*key))
        })
        .and_then(permission_from_value)
        .unwrap_or(PermissionClass::Safe)
}

fn permission_from_value(value: &serde_json::Value) -> Option<PermissionClass> {
    match value {
        serde_json::Value::String(raw) => PermissionClass::from_declared(raw),
        serde_json::Value::Object(fields) => LEVEL_KEYS
            .iter()
            .find_map(|key| fields.get(*key))
            .and_then(permission_from_value),
        _ => None,
    }
}
