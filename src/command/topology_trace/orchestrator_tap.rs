//! Tap two: an orchestrator event becomes trace records.
//!
//! Owns the projection of [`OrchestratorEvent`], including the one case that is
//! not a record at all — `TaskDecomposed` lowers a subtask list into a declared
//! graph, so a team run folds against an authored shape rather than a
//! reconstruction.

use archon_core::orchestrator::events::OrchestratorEvent;
use archon_topology::ir::{GraphOrigin, TaskGraph};
use archon_topology::trace::{TraceKind, TraceRecord};

use super::{AmbientTrace, now};

impl AmbientTrace {
    /// Project an orchestrator event into trace records.
    ///
    /// Only the four variants the design names are projected. The others
    /// (`AgentProgress`, `TeamFailed`) are never emitted anywhere in the tree,
    /// and `AgentComplete` / `AgentFailed` are handled because they carry the
    /// per-node terminal outcome the corpus needs.
    pub(crate) fn record_orchestrator_event(&self, event: &OrchestratorEvent) {
        let ts = now();
        match event {
            OrchestratorEvent::TaskDecomposed { subtasks } => {
                // The decomposition *is* the declared graph. Lowering it here
                // means a team run folds against an authored shape rather than
                // a reconstruction, which is strictly better information.
                let graph =
                    archon_core::orchestrator::topology::lower_subtasks(subtasks, &self.session_id);
                let graph = TaskGraph {
                    id: self.graph_id.clone(),
                    origin: GraphOrigin::Team {
                        session_id: self.session_id.clone(),
                    },
                    ..graph
                };
                self.declare_graph(&graph);
            }
            OrchestratorEvent::AgentSpawned {
                agent_id,
                agent_type,
                subtask_id,
            } => {
                self.record(
                    TraceRecord::new(&ts, &self.graph_id, TraceKind::AgentSpawned)
                        .with_node(subtask_id)
                        .with_agent(agent_type)
                        .with_detail(agent_id),
                );
                self.record(
                    TraceRecord::new(&ts, &self.graph_id, TraceKind::NodeStarted)
                        .with_node(subtask_id),
                );
            }
            OrchestratorEvent::AgentComplete { subtask_id, .. } => {
                self.record(
                    TraceRecord::new(&ts, &self.graph_id, TraceKind::NodeFinished)
                        .with_node(subtask_id),
                );
            }
            OrchestratorEvent::AgentFailed {
                subtask_id,
                will_retry,
                ..
            } => {
                let kind = if *will_retry {
                    TraceKind::Retry
                } else {
                    TraceKind::NodeFinished
                };
                self.record(
                    TraceRecord::new(&ts, &self.graph_id, kind)
                        .with_node(subtask_id)
                        .with_outcome(false, true),
                );
            }
            OrchestratorEvent::TeamComplete { .. } => {
                self.record(TraceRecord::new(
                    &ts,
                    &self.graph_id,
                    TraceKind::NodeFinished,
                ));
            }
            OrchestratorEvent::TeamCancelled => {
                self.record(
                    TraceRecord::new(&ts, &self.graph_id, TraceKind::NodeFinished)
                        .with_outcome(false, true)
                        .with_detail("team cancelled"),
                );
            }
            // Declared but never emitted anywhere in the tree.
            OrchestratorEvent::AgentProgress { .. } | OrchestratorEvent::TeamFailed { .. } => {}
        }
    }
}
