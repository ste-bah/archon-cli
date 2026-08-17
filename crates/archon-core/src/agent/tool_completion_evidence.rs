use super::*;

impl Agent {
    pub(super) fn record_executed_test_evidence(
        &self,
        pre: &tool_types::PreflightResult,
        raw_result: &ToolResult,
        accepted_result: &mut ToolResult,
    ) {
        if pre.tool_name != "Bash" {
            return;
        }
        let Some(execution) = raw_result.authoritative_bash_execution() else {
            return;
        };
        if execution.session_id() != self.config.session_id
            || execution.tool_use_id() != pre.tool_id
        {
            *accepted_result = ToolResult::error(
                "Test command result cannot be accepted because its authoritative Bash execution identity does not match this tool call",
            );
            return;
        }
        let (Some(store), Some(authority)) = (
            self.plan_store.as_ref(),
            self.plan_approval_authority.as_ref(),
        ) else {
            return;
        };
        match archon_tools::bash_evidence::record_authoritative_test_execution(
            store, authority, execution,
        ) {
            Ok(Some(evidence)) => {
                match store.verify_test_command_evidence(
                    authority,
                    &self.config.session_id,
                    &evidence.run_id,
                    &evidence.evidence_id,
                ) {
                    Ok(_) => accepted_result.content.push_str(&format!(
                        "\n[Completion evidence: run_id={} evidence_id={}]",
                        evidence.run_id, evidence.evidence_id
                    )),
                    Err(error) => {
                        *accepted_result = ToolResult::error(format!(
                            "Test command result cannot be accepted because independent verification failed: {error}"
                        ));
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                *accepted_result = ToolResult::error(format!(
                    "Test command result cannot be accepted because durable completion evidence could not be recorded: {error}"
                ));
            }
        }
    }
}
