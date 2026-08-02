//! How IR values are spelled on the wire.
//!
//! One place for every enum-to-string mapping the stored rows use, so that the
//! relation writer ([`super::rows`]) and the learning summary
//! ([`super::learning_summary`]) cannot drift apart on what a role or an origin
//! is called. Changing a spelling here changes it for both, which is the point.

use archon_topology::ir::{GraphOrigin, PermissionClass, WriteTarget};

pub(super) fn origin_label(origin: &GraphOrigin) -> &'static str {
    match origin {
        GraphOrigin::Workflow { .. } => "workflow",
        GraphOrigin::Team { .. } => "team",
        GraphOrigin::Session { .. } => "session",
    }
}

/// `(run_id, session_id)` — whichever the origin carries; the other is empty.
pub(super) fn origin_ids(origin: &GraphOrigin) -> (String, String) {
    match origin {
        GraphOrigin::Workflow { run_id } => (run_id.clone(), String::new()),
        GraphOrigin::Team { session_id } | GraphOrigin::Session { session_id } => {
            (String::new(), session_id.clone())
        }
    }
}

pub(super) fn role_label(role: archon_topology::ir::NodeRole) -> String {
    use archon_topology::ir::NodeRole;
    match role {
        NodeRole::Plan => "plan".to_string(),
        NodeRole::Work => "work".to_string(),
        NodeRole::Verify => "verify".to_string(),
        NodeRole::Reduce => "reduce".to_string(),
        NodeRole::Tool => "tool".to_string(),
        NodeRole::Gate(kind) => format!("gate:{}", gate_label(kind)),
    }
}

fn gate_label(kind: archon_topology::ir::GateKind) -> &'static str {
    match kind {
        archon_topology::ir::GateKind::Human => "human",
        archon_topology::ir::GateKind::Checkpoint => "checkpoint",
    }
}

pub(super) fn permission_label(permission: PermissionClass) -> &'static str {
    match permission {
        PermissionClass::Safe => "safe",
        PermissionClass::Risky => "risky",
        PermissionClass::Irreversible => "irreversible",
    }
}

pub(super) fn write_target_label(target: &WriteTarget) -> String {
    match target {
        WriteTarget::Path(path) => format!("path:{path}"),
        WriteTarget::Artifact(key) => format!("artifact:{key}"),
    }
}
