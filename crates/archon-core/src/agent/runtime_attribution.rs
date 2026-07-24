use super::AgentConfig;

#[derive(Debug, Clone, Copy)]
pub struct RuntimeAttribution<'a> {
    pub run_id: &'a str,
    pub session_id: &'a str,
    pub role: &'a str,
    pub origin: &'a str,
    pub turn: Option<u64>,
    pub round: Option<u64>,
    pub denominator: Option<u64>,
}

impl AgentConfig {
    pub fn runtime_context_extra(&self) -> serde_json::Value {
        serde_json::json!({
            "archon_runtime": {
                "run_id": self.session_id,
                "session_id": self.session_id,
                "role": "assistant",
                "agent_type": self.agent_type,
                "agent_version": self.agent_version,
            }
        })
    }

    pub fn auxiliary_runtime_extra(
        &self,
        role: &str,
        origin: &str,
        turn: u64,
        round: Option<u32>,
        denominator: Option<u64>,
    ) -> serde_json::Value {
        self.runtime_attribution_extra(role, origin, Some(turn), round.map(u64::from), denominator)
    }

    pub fn runtime_attribution_extra(
        &self,
        role: &str,
        origin: &str,
        turn: Option<u64>,
        round: Option<u64>,
        denominator: Option<u64>,
    ) -> serde_json::Value {
        self.runtime_attribution_extra_for_scope(RuntimeAttribution {
            run_id: &self.session_id,
            session_id: &self.session_id,
            role,
            origin,
            turn,
            round,
            denominator,
        })
    }

    pub fn runtime_attribution_extra_for_scope(
        &self,
        attribution: RuntimeAttribution<'_>,
    ) -> serde_json::Value {
        let mut extra = self.runtime_context_extra();
        let runtime = &mut extra["archon_runtime"];
        runtime["run_id"] = serde_json::json!(attribution.run_id);
        runtime["session_id"] = serde_json::json!(attribution.session_id);
        runtime["role"] = serde_json::json!(attribution.role);
        runtime["origin"] = serde_json::json!(attribution.origin);
        insert_optional(runtime, "turn", attribution.turn);
        insert_optional(runtime, "round", attribution.round);
        insert_optional(runtime, "effective_denominator", attribution.denominator);
        extra
    }

    pub fn request_runtime_extra(
        &self,
        turn: u64,
        round: u32,
        denominator: u64,
    ) -> serde_json::Value {
        self.runtime_attribution_extra(
            "assistant",
            "main_session",
            Some(turn),
            Some(u64::from(round)),
            Some(denominator),
        )
    }
}

fn insert_optional(runtime: &mut serde_json::Value, name: &str, value: Option<u64>) {
    if let Some(value) = value {
        runtime[name] = serde_json::json!(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auxiliary_attribution_keeps_unknown_denominator_absent() {
        let config = AgentConfig {
            session_id: "session-1".into(),
            agent_type: "reviewer".into(),
            agent_version: Some("1.0.0".into()),
            ..AgentConfig::default()
        };

        let extra = config.auxiliary_runtime_extra("subagent", "subagent", 4, Some(2), None);

        assert_eq!(extra["archon_runtime"]["run_id"], "session-1");
        assert_eq!(extra["archon_runtime"]["session_id"], "session-1");
        assert_eq!(extra["archon_runtime"]["role"], "subagent");
        assert_eq!(extra["archon_runtime"]["origin"], "subagent");
        assert_eq!(extra["archon_runtime"]["turn"], 4);
        assert_eq!(extra["archon_runtime"]["round"], 2);
        assert!(
            extra["archon_runtime"]
                .get("effective_denominator")
                .is_none()
        );
    }

    #[test]
    fn request_attribution_carries_agent_identity() {
        let config = AgentConfig {
            session_id: "session-1".into(),
            agent_type: "reviewer".into(),
            agent_version: Some("1.0.0".into()),
            ..AgentConfig::default()
        };

        let extra = config.request_runtime_extra(2, 3, 100);

        assert_eq!(extra["archon_runtime"]["role"], "assistant");
        assert_eq!(extra["archon_runtime"]["agent_type"], "reviewer");
        assert_eq!(extra["archon_runtime"]["agent_version"], "1.0.0");
        assert_eq!(extra["archon_runtime"]["turn"], 2);
        assert_eq!(extra["archon_runtime"]["round"], 3);
        assert_eq!(extra["archon_runtime"]["effective_denominator"], 100);
    }
}
