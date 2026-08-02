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
/// `Condition` is the one kind the table does not name. Finding W6 records that
/// `StageSpec::condition` has no evaluator anywhere in the tree and zero
/// read-sites, so a `Condition` stage always proceeds — it is an unconditional
/// pass-through. It therefore lowers to `Plan`, the only role with neither
/// side-effect nor gating semantics. Lowering it to `Gate` would be actively
/// unsafe: it would let a stage that gates nothing satisfy
/// [`TaskGraph::ungated_irreversible`].
fn lower_role(kind: StageKind) -> NodeRole {
    use crate::ir::GateKind;
    match kind {
        StageKind::Agent | StageKind::Implementation | StageKind::Fanout => NodeRole::Work,
        StageKind::Reduce => NodeRole::Reduce,
        StageKind::QualityGate => NodeRole::Verify,
        StageKind::HumanGate => NodeRole::Gate(GateKind::Human),
        StageKind::Checkpoint => NodeRole::Gate(GateKind::Checkpoint),
        StageKind::Tool => NodeRole::Tool,
        StageKind::Condition => NodeRole::Plan,
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

/// `WorkflowSpec::permissions` → [`PermissionClass`].
///
/// The map is `BTreeMap<String, Value>` with no schema — `deserialize_permissions`
/// accepts any JSON object verbatim and nothing in the tree reads the result
/// — so this is a best-effort read of the shapes that are plausibly authored:
/// a per-stage entry keyed by stage id, or a blanket `default`/`*` entry, whose
/// value is either the class as a string or an object carrying it under
/// `class`, `permission`, or `level`.
///
/// Anything unrecognised lowers to `Safe`. That is deliberately fail-open:
/// milestone 1 enforces nothing, and the design requires milestone 3's
/// enforcement never to fail closed on a bookkeeping gap. An authored graph
/// that wants an irreversible node marked must say so in a shape this reads.
fn lower_permission(spec: &WorkflowSpec, stage: &StageSpec) -> PermissionClass {
    spec.permissions
        .get(&stage.id)
        .or_else(|| spec.permissions.get("default"))
        .or_else(|| spec.permissions.get("*"))
        .and_then(permission_from_value)
        .unwrap_or(PermissionClass::Safe)
}

fn permission_from_value(value: &serde_json::Value) -> Option<PermissionClass> {
    match value {
        serde_json::Value::String(raw) => permission_from_str(raw),
        serde_json::Value::Object(fields) => ["class", "permission", "level"]
            .iter()
            .find_map(|key| fields.get(*key))
            .and_then(permission_from_value),
        _ => None,
    }
}

fn permission_from_str(raw: &str) -> Option<PermissionClass> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "safe" => Some(PermissionClass::Safe),
        "risky" => Some(PermissionClass::Risky),
        "irreversible" => Some(PermissionClass::Irreversible),
        _ => None,
    }
}
