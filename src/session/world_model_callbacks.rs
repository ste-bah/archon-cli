use std::sync::Arc;

use archon_core::agent::{Agent, FirstToolActionCallback, TurnFinalizationCallback};
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
) {
    install_on(agent, config, session_id);
}

fn install_on(
    target: &mut impl CallbackTarget,
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
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

    let finalization_session_id = session_id.to_string();
    target.set_turn_finalization_callback(Arc::new(move |action_id, _output| {
        crate::command::world_model::turn_finalization_verdict_for_action(
            &finalization_session_id,
            action_id,
        )
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingTarget {
        first_action: bool,
        tool_run: bool,
        finalization: bool,
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

        fn set_turn_finalization_callback(&mut self, _: TurnFinalizationCallback) {
            self.finalization = true;
        }
    }

    #[test]
    fn session_world_model_callbacks_install_all_guards() {
        let mut target = RecordingTarget::default();

        install_on(
            &mut target,
            &archon_core::config::ArchonConfig::default(),
            "session-test",
        );

        assert!(target.first_action);
        assert!(target.tool_run);
        assert!(target.finalization);
    }
}
