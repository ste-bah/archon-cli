use std::path::Path;
use std::sync::Arc;

use archon_core::agent::{
    Agent, FirstToolActionCallback, TurnFinalizationCallback, TurnFinalizationVerdict,
};
use archon_tools::tool::{ToolRunAdmissionCallback, ToolRunOutcomeCallback};

trait CallbackTarget {
    fn set_first_tool_action_callback(&mut self, callback: FirstToolActionCallback);
    fn set_tool_run_callbacks(
        &mut self,
        admission: ToolRunAdmissionCallback,
        outcome: ToolRunOutcomeCallback,
    );
    fn set_turn_finalization_callback(&mut self, callback: TurnFinalizationCallback);
}

impl CallbackTarget for Agent {
    fn set_first_tool_action_callback(&mut self, callback: FirstToolActionCallback) {
        Agent::set_first_tool_action_callback(self, callback);
    }

    fn set_tool_run_callbacks(
        &mut self,
        admission: ToolRunAdmissionCallback,
        outcome: ToolRunOutcomeCallback,
    ) {
        Agent::set_tool_run_callbacks(self, admission, outcome);
    }

    fn set_turn_finalization_callback(&mut self, callback: TurnFinalizationCallback) {
        Agent::set_turn_finalization_callback(self, callback);
    }
}

pub(super) fn install(
    agent: &mut Agent,
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    session_database: &Path,
) {
    install_on_with_session_database(agent, config, session_id, session_database);
}

fn install_on_with_session_database(
    target: &mut impl CallbackTarget,
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    session_database: &Path,
) {
    let guardrail_config = config.clone();
    let guardrail_session_id = session_id.to_string();
    target.set_first_tool_action_callback(Arc::new(
        move |action_id, tool_name, tool_use_id, input| {
            crate::command::world_model::reclassify_active_guardrail_for_session(
                &guardrail_config,
                &guardrail_session_id,
                action_id,
                tool_name,
                tool_use_id,
                input,
            );
            crate::command::world_model::turn_requirements_for_action(
                &guardrail_session_id,
                action_id,
            )
        },
    ));

    // Milestone 3 guardrail admission. Registering the session here rather than
    // inside the callback keeps the callback allocation-free and means a
    // session that never makes a non-`Safe` tool call is still tracked, which
    // matters for the write claims a spawned agent's writes compare against.
    crate::command::topology_admission::install(config, session_id);

    let admission_config = config.clone();
    target.set_tool_run_callbacks(
        // Composed admission: topology guardrails first (in-memory, no database
        // access of any kind), then the world-model guardrail. See
        // `world_model::admit_tool_run_composed`.
        Arc::new(move |request| {
            crate::command::world_model::admit_tool_run_composed(&admission_config, request)
        }),
        // Composed tap: the ambient topology trace, the topology admission
        // release, and the world-model guardrail ledger. See
        // `world_model::tool_run_outcome_taps`.
        Arc::new(crate::command::world_model::tool_run_outcome_taps),
    );

    // Composed finalization: the world-model guardrail first, then the #187
    // completion gate. There is one callback slot, so this has to compose the
    // way `admit_tool_run_composed` does above — installing the gate on its own
    // would silently drop the guardrail, which is the exact class of bug #187
    // was opened about.
    //
    // First blocker wins, and the guardrail goes first because it judges the
    // action's own verification evidence; the gate judges what a review said
    // about it. If both object, the more specific complaint is the useful one.
    let finalization_session_id = session_id.to_string();
    let finalization_session_database = session_database.to_path_buf();
    let completion_gate = config.skills.completion_gate;
    target.set_turn_finalization_callback(Arc::new(move |action_id, _output| {
        match crate::command::world_model::turn_finalization_verdict_for_action_at_session_database(
            &finalization_session_database,
            &finalization_session_id,
            action_id,
        ) {
            TurnFinalizationVerdict::Allowed => {
                super::completion_gate::verdict(&finalization_session_id, completion_gate)
            }
            blocked => blocked,
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingTarget {
        first_action: bool,
        tool_run: bool,
        finalization: Option<TurnFinalizationCallback>,
    }

    impl CallbackTarget for RecordingTarget {
        fn set_first_tool_action_callback(&mut self, _: FirstToolActionCallback) {
            self.first_action = true;
        }

        fn set_tool_run_callbacks(
            &mut self,
            _: ToolRunAdmissionCallback,
            _: ToolRunOutcomeCallback,
        ) {
            self.tool_run = true;
        }

        fn set_turn_finalization_callback(&mut self, callback: TurnFinalizationCallback) {
            self.finalization = Some(callback);
        }
    }

    #[test]
    fn finalization_callback_uses_the_explicit_runtime_session_database() {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("runtime-session.db");
        let session_id = "runtime-callback-session";
        let store = archon_session::storage::SessionStore::open(&database).unwrap();
        let plans = archon_session::plan::PlanStore::new(store.db()).unwrap();
        let mut plan = archon_session::plan::PlanDocument::new("runtime-plan", "Runtime plan");
        plan.session_id = Some(session_id.into());
        plan.status = archon_session::plan::PlanStatus::Executing;
        plan.steps = vec![archon_session::plan::PlanStep {
            number: 1,
            description: "finish the approved work".into(),
            affected_files: Vec::new(),
            status: archon_session::plan::PlanStepStatus::Pending,
            blocked_by: Vec::new(),
            required_evidence: Vec::new(),
            task_id: None,
        }];
        plans.save_plan(session_id, &plan).unwrap();

        let mut target = RecordingTarget::default();
        install_on_with_session_database(
            &mut target,
            &archon_core::config::ArchonConfig::default(),
            session_id,
            &database,
        );
        let callback = target.finalization.expect("finalization callback");

        assert!(matches!(
            callback("", ""),
            archon_core::agent::TurnFinalizationVerdict::Blocked { .. }
        ));
    }

    #[test]
    fn session_world_model_callbacks_install_all_guards() {
        let mut target = RecordingTarget::default();

        let config = archon_core::config::ArchonConfig::default();
        let database = crate::command::store_paths::session_db_path(&config);
        install_on_with_session_database(&mut target, &config, "session-test", &database);

        assert!(target.first_action);
        assert!(target.tool_run);
        assert!(target.finalization.is_some());
    }
}
