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
                // Milestone 3: the same lowering feeds admission, so a team run
                // is admitted against the shape it declared rather than against
                // a reconstruction. Keyed by the trace's session id — see the
                // namespace caveat in `topology_admission`.
                crate::command::topology_admission::declare_graph(&self.session_id, &graph);
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
                crate::command::topology_admission::on_node_started(&self.session_id, subtask_id);
            }
            OrchestratorEvent::AgentComplete { subtask_id, .. } => {
                self.record(
                    TraceRecord::new(&ts, &self.graph_id, TraceKind::NodeFinished)
                        .with_node(subtask_id),
                );
                // Releases the node's write claims and its live-agent slot. The
                // lifetime agent total is not released: that is the point of it.
                crate::command::topology_admission::on_node_finished(&self.session_id, subtask_id);
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
                if *will_retry {
                    crate::command::topology_admission::on_node_started(
                        &self.session_id,
                        subtask_id,
                    );
                } else {
                    crate::command::topology_admission::on_node_finished(
                        &self.session_id,
                        subtask_id,
                    );
                }
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
