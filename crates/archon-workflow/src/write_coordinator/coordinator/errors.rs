//! Aggregated error type for the coordinated fanout flow.

use super::super::conflict_graph::ScheduleError;
use super::super::patch_apply::ApplyError;
use super::super::patch_manifest::PatchError;
use super::super::worktree_isolation::IsolationError;
use super::super::write_plan::WritePlanError;

#[derive(Debug)]
pub enum FanoutError {
    Plan(WritePlanError),
    Isolation(IsolationError),
    Schedule(ScheduleError),
    Patch(PatchError),
    Apply(ApplyError),
    Workflow(String),
}

impl std::fmt::Display for FanoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plan(e) => write!(f, "write plan error: {e}"),
            Self::Isolation(e) => write!(f, "isolation error: {e}"),
            Self::Schedule(e) => write!(f, "schedule error: {e}"),
            Self::Patch(e) => write!(f, "patch error: {e}"),
            Self::Apply(e) => write!(f, "apply error: {e}"),
            Self::Workflow(e) => write!(f, "workflow error: {e}"),
        }
    }
}

impl std::error::Error for FanoutError {}
