use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceCriterion, JudgeDecision, JudgeVerdict,
};

/// Parse one candidate while preserving the host-owned parts of its source entry.
#[cfg(test)]
pub(super) fn candidate_entry(
    reply: &str,
    frozen: &AcceptanceCriterion,
) -> std::result::Result<AcceptanceCriterion, String> {
    candidate_entry_with_markers(reply, frozen, &[])
}

pub(super) fn candidate_entry_with_markers(
    reply: &str,
    frozen: &AcceptanceCriterion,
    markers: &[String],
) -> std::result::Result<AcceptanceCriterion, String> {
    use crate::command::workflow_task_set::acceptance_reply_blocks as blocks;
    if let Some(refusal) = blocks::unclosed_block_refusal_with_markers(reply, markers) {
        return Err(refusal);
    }
    let reply_blocks = blocks::reply_blocks_with_markers(reply, markers);
    let has_fenced_entry_and_check = reply_blocks.iter().any(|block| block.info == "json")
        && reply_blocks.iter().any(|block| block.info == "check");
    let document = if has_fenced_entry_and_check {
        let json_block = reply_blocks
            .iter()
            .find(|block| block.info == "json")
            .expect("the JSON block was observed");
        let entry_prefix_lines = json_block
            .content
            .find('{')
            .map(|start| json_block.content[..start].matches('\n').count())
            .unwrap_or_default();
        let source_line_prefix = "\n".repeat(json_block.opened_line + entry_prefix_lines);
        std::borrow::Cow::Owned(
            format!(
                "{source_line_prefix}{}",
                blocks::extract_object_with_markers(reply, markers)
            )
            .into_bytes(),
        )
    } else {
        crate::command::workflow_freeze_candidate::candidate_document(reply.trim().as_bytes())
    };
    let mut value: serde_json::Value = match serde_json::from_slice(&document) {
        Ok(value) => value,
        Err(error) => {
            let extracted = blocks::extract_object_with_markers(reply, markers);
            let diagnostic =
                crate::command::workflow_json_parse_diagnostic::describe(&document, &error);
            serde_json::from_str(&extracted).map_err(|_| format!(
                "the reply for check '{}' is not one JSON entry ({diagnostic}); return the raw JSON object alone",
                frozen.id
            ))?
        }
    };
    if value["id"] != serde_json::json!(frozen.id)
        && let Some([only]) = value["acceptance"].as_array().map(Vec::as_slice)
        && only["id"] == serde_json::json!(frozen.id)
    {
        value = only.clone();
    }
    if value["id"] != serde_json::json!(frozen.id) {
        return Err(format!(
            "the reply is not an entry with id '{}'; return exactly that entry",
            frozen.id
        ));
    }
    if let Some(refusal) = blocks::resolve_command_block_with_markers(&mut value, reply, markers) {
        return Err(refusal);
    }
    let check: AcceptanceCheck =
        serde_json::from_value(value["check"].clone()).map_err(|error| {
            format!(
                "check '{}': the entry's check does not match either shape ({error})",
                frozen.id
            )
        })?;
    if check == frozen.check {
        return Err(format!(
            "check '{}': the reply repeats the check being replaced; change it so it resolves every finding above",
            frozen.id
        ));
    }
    Ok(AcceptanceCriterion {
        id: frozen.id.clone(),
        criterion: frozen.criterion.clone(),
        check,
        gap_permitted: frozen.gap_permitted,
        covers: frozen.covers.clone(),
        judgment: JudgeVerdict {
            verdict: JudgeDecision::Refuted,
            counterexample: String::new(),
            reason: String::new(),
            host_call_id: String::new(),
            sampling: None,
        },
    })
}
