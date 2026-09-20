use super::SubagentRunner;
impl SubagentRunner {
    pub(super) fn evidence_recovery_message(&self) -> serde_json::Value {
        let mut hint = "Your evidence was summarised. This is not task completion: return the full contracted artifact, not a narration or a handover. Preserve established findings and re-establish only missing evidence.\n".to_string();
        if let Some(guard) = &self.tool_context.workflow_read_guard {
            hint.push_str(&guard.orientation());
        }
        if let Some(landing) = &self.tool_context.audit_landing {
            hint.push_str(
                &landing
                    .hint()
                    .unwrap_or_else(|e| format!("Host audit record recovery failed: {e}")),
            );
        }
        if let (Some(store), Some(id)) = (&self.transcript_store, &self.transcript_agent_id) {
            hint.push_str(&format!("\nOriginal evidence remains in {}. Use read-own-evidence with offset/limit for needed transcript lines, not the whole log.",store.transcript_path(id).display()));
        }
        serde_json::json!({"role":"user","content":hint})
    }
}

pub(super) struct ReadOwnEvidence(pub std::path::PathBuf);
#[async_trait::async_trait]
impl archon_tools::tool::Tool for ReadOwnEvidence {
    fn name(&self) -> &str {
        "read-own-evidence"
    }
    fn description(&self) -> &str {
        "Read a bounded range of this agent's own durable evidence transcript after compaction. No path argument; offset and char_offset are 0-based, limit is at most 10 lines. Use next_offset and next_char_offset to continue a large line."
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object","properties":{"offset":{"type":"integer","minimum":0},"char_offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":10}}})
    }
    async fn execute(
        &self,
        input: serde_json::Value,
        _: &archon_tools::tool::ToolContext,
    ) -> archon_tools::tool::ToolResult {
        use std::io::{BufRead, BufReader};
        let offset = input
            .get("offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .min(usize::MAX as u64) as usize;
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(3)
            .clamp(1, 10) as usize;
        let file = match std::fs::File::open(&self.0) {
            Ok(file) => file,
            Err(e) => return archon_tools::tool::ToolResult::error(e.to_string()),
        };
        let char_offset = input
            .get("char_offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .min(usize::MAX as u64) as usize;
        let mut lines = Vec::new();
        let mut bytes = 0;
        let mut truncated = false;
        let mut next_offset = offset;
        let mut next_char_offset = 0;
        for (index, line) in BufReader::new(file)
            .lines()
            .skip(offset)
            .take(limit)
            .enumerate()
        {
            let line = match line {
                Ok(line) => line,
                Err(e) => return archon_tools::tool::ToolResult::error(e.to_string()),
            };
            let skip = if index == 0 { char_offset } else { 0 };
            let mut chunk = String::new();
            let mut used = 0;
            for ch in line.chars().skip(skip) {
                if bytes + chunk.len() + ch.len_utf8() > 32768 {
                    truncated = true;
                    break;
                }
                chunk.push(ch);
                used += 1;
            }
            bytes += chunk.len();
            lines.push(chunk);
            if truncated {
                next_offset = offset + index;
                next_char_offset = skip + used;
                break;
            }
            next_offset = offset + index + 1;
        }
        archon_tools::tool::ToolResult::success(serde_json::json!({"offset":offset,"next_offset":next_offset,"next_char_offset":next_char_offset,"lines":lines,"truncated":truncated}).to_string())
    }
    fn capability(&self) -> archon_tools::tool::ToolCapability {
        archon_tools::tool::ToolCapability::HostLocal
    }
    fn permission_level(&self, _: &serde_json::Value) -> archon_tools::tool::PermissionLevel {
        archon_tools::tool::PermissionLevel::Safe
    }
    fn working_tree_effect(&self) -> archon_tools::tool::WorkingTreeEffect {
        archon_tools::tool::WorkingTreeEffect::None
    }
}
impl SubagentRunner {
    pub(crate) fn install_evidence_reader(&mut self) {
        if let (Some(store), Some(id)) = (&self.transcript_store, &self.transcript_agent_id) {
            let mut registry = (*self.registry).clone();
            registry.replace(Box::new(ReadOwnEvidence(store.transcript_path(id))));
            self.tool_definitions = archon_llm::provider::shared_tools(registry.tool_definitions());
            self.registry = std::sync::Arc::new(registry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_tools::tool::{Tool, ToolContext};
    #[tokio::test]
    async fn evidence_reader_pages_large_utf8_lines_without_losing_tail() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("own.jsonl");
        let expected = format!("{}FINAL_EVIDENCE", "é".repeat(20000));
        std::fs::write(&path, format!("{expected}\n")).unwrap();
        let tool = ReadOwnEvidence(path);
        let first = tool
            .execute(serde_json::json!({}), &ToolContext::default())
            .await;
        let first: serde_json::Value = serde_json::from_str(&first.content).unwrap();
        assert_eq!(first["truncated"], true);
        let second=tool.execute(serde_json::json!({"offset":first["next_offset"],"char_offset":first["next_char_offset"]}),&ToolContext::default()).await;
        let second: serde_json::Value = serde_json::from_str(&second.content).unwrap();
        assert_eq!(
            format!(
                "{}{}",
                first["lines"][0].as_str().unwrap(),
                second["lines"][0].as_str().unwrap()
            ),
            expected
        );
        assert_eq!(second["next_offset"], 1);
    }
}
