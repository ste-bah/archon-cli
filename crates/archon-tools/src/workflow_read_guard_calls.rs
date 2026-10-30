use super::*;

impl WorkflowReadGuard {
    /// Called at the common tool-dispatch boundary; admission is atomic even
    /// when the model asks for several reads in the same parallel round.
    ///
    /// A refusal is also written to the sidecar, as a `refusal` record and as
    /// a refused `tool_call`, so a session restarted or retried in this
    /// worktree is told not to try the same call again. Live, a retry spent
    /// its first ten minutes re-running the release build and the `git
    /// worktree add` the previous session had already been refused.
    #[cfg(test)]
    pub fn before_tool(&self, name: &str, input: &Value) -> Option<String> {
        self.before_tool_for_key((String::new(), None), false, name, input)
    }

    /// Common tool-dispatch boundary with the calling agent's identity.
    pub fn before_tool_for(&self, ctx: &ToolContext, name: &str, input: &Value) -> Option<String> {
        self.before_tool_for_key(
            (ctx.session_id.clone(), ctx.subagent_id.clone()),
            ctx.parent_subagent_id.is_some(),
            name,
            input,
        )
    }

    fn before_tool_for_key(
        &self,
        key: AgentKey,
        child: bool,
        name: &str,
        input: &Value,
    ) -> Option<String> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.calls = state.calls.saturating_add(1);
        state.calls_since_write = state.calls_since_write.saturating_add(1);
        let agent = state.agent_state.entry(key.clone()).or_default();
        agent.calls = agent.calls.saturating_add(1);
        let call = state.calls;
        let terminal = state
            .agent_state
            .get(&key)
            .and_then(|agent| agent.terminal.clone());
        let refusal = match terminal {
            Some(terminal) => terminal,
            None => {
                let verdict = self.admit(&mut state, &key, name, input);
                thrash::observe(&mut state, &key, name, input, verdict)?
            }
        };
        drop(state);
        let head = record_head(name, input);
        let reason = first_line(&refusal);
        self.remember(json!({
            "kind": REFUSAL_RECORD_KIND, "call": call, "tool": name, "head": head, "reason": reason,
            "agent_id": key.1.clone(), "child": child,
        }));
        self.remember(json!({
            "kind": TOOL_CALL_RECORD_KIND, "call": call, "tool": name, "head": head,
            "status": format!("refused: {reason}"),
            "agent_id": key.1.clone(), "child": child,
        }));
        Some(refusal)
    }

    /// Best effort: a session record that cannot be written must not fail the
    /// call it describes, unlike a read range, whose loss would silently cost
    /// the retry its orientation.
    pub(super) fn remember(&self, record: Value) {
        if let Some(sink) = &self.read_set_path {
            let _ = append_record(sink, &record);
        }
    }

    /// Any agent's terminal thrash verdict, for diagnostics and tests over the
    /// whole guard. Agent runners use [`Self::terminal_failure_for`], so one
    /// agent's verdict never ends another (Issue-521).
    pub fn terminal_failure(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .agent_state
            .values()
            .find_map(|agent| agent.terminal.clone())
    }

    /// The terminal thrash verdict for the calling agent invocation.
    pub fn terminal_failure_for(&self, ctx: &ToolContext) -> Option<String> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .agent_state
            .get(&(ctx.session_id.clone(), ctx.subagent_id.clone()))
            .and_then(|agent| agent.terminal.clone())
    }
}
