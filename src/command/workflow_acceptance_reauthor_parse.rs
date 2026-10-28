use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceCriterion, JudgeDecision, JudgeVerdict,
};

fn visible_block_form_intent(reply: &str, markers: &[String]) -> bool {
    use crate::command::workflow_task_set::acceptance_reply_blocks as blocks;
    let source = blocks::strip_reply_markers(reply, markers);
    let reply_blocks = blocks::reply_blocks_with_markers(&source, &[]);
    crate::command::workflow_json_parse_diagnostic::entry_check_command_block_true_offsets(&source)
        .into_iter()
        .any(|start| {
            !reply_blocks
                .iter()
                .any(|block| block.info == "check" && start >= block.start && start < block.end)
        })
}

fn parse_refusal_advice(
    diagnostic: &str,
    extracted: &str,
    has_check_fence: bool,
    visible_block_form: bool,
) -> &'static str {
    let parsed = serde_json::from_str::<serde_json::Value>(extracted).ok();
    let parsed_block_form = parsed
        .as_ref()
        .and_then(|value| value.pointer("/check/command_block"))
        == Some(&serde_json::Value::Bool(true));
    if visible_block_form || parsed_block_form {
        "correct the JSON entry and put the script in exactly one non-empty fence labelled check after it"
    } else if has_check_fence
        && parsed
            .as_ref()
            .and_then(|value| value.pointer("/check/command"))
            .is_some()
    {
        "the reply mixes an inline command with a check fence and must use one form"
    } else if has_check_fence {
        "correct the JSON entry and keep exactly one non-empty check fence for block form"
    } else if diagnostic
        .contains("the check is a multi-line script; return it in block form instead:")
    {
        ""
    } else {
        "return the raw JSON object alone"
    }
}

fn format_parse_refusal(id: &str, diagnostic: &str, advice: &str) -> String {
    const MULTILINE_DIRECTION: &str =
        "; the check is a multi-line script; return it in block form instead:";
    if let Some((syntax, direction)) = diagnostic.split_once(MULTILINE_DIRECTION) {
        return format!(
            "the reply for check '{id}' is not one JSON entry ({syntax}); the check is a multi-line script; return it in block form instead:{direction}"
        );
    }
    let separator = if advice.is_empty() { "" } else { "; " };
    format!("the reply for check '{id}' is not one JSON entry ({diagnostic}){separator}{advice}")
}

pub(super) fn shape_diagnoses(
    reply: &str,
    markers: &[String],
    extraction: &crate::command::workflow_task_set::acceptance_reply_blocks::ObjectExtraction,
) -> Vec<String> {
    use crate::command::workflow_task_set::acceptance_reply_blocks as blocks;
    let source = blocks::strip_reply_markers(reply, markers);
    let reply_blocks = blocks::reply_blocks(&source);
    let mut diagnoses = Vec::new();
    let wrapped_array = reply_blocks
        .iter()
        .filter(|block| block.info == "json")
        .any(|block| {
            serde_json::from_str::<serde_json::Value>(block.content.trim())
                .ok()
                .is_some_and(|value| value.is_array())
        })
        || reply_blocks
            .iter()
            .find(|block| block.info == "check")
            .and_then(|check| {
                serde_json::from_str::<serde_json::Value>(source[..check.start].trim()).ok()
            })
            .is_some_and(|value| value.is_array())
        || serde_json::from_str::<serde_json::Value>(source.trim())
            .ok()
            .is_some_and(|value| value.is_array())
        || matches!(
            extraction,
            blocks::ObjectExtraction::ArrayWrappedEntry(_)
                | blocks::ObjectExtraction::SeveralDifferentEntries {
                    array_wrapped: true,
                    ..
                }
        );
    if wrapped_array {
        diagnoses.push("return the entry object itself, not an array".into());
    }
    if let blocks::ObjectExtraction::SeveralDifferentEntries { count, .. } = extraction {
        diagnoses.push(format!(
            "the reply holds {count} different entries; return exactly one"
        ));
    }
    if let blocks::ObjectExtraction::MisplacedBrace {
        missing_fields,
        next_field,
        additional_entries,
    } = extraction
    {
        let missing = if missing_fields.is_empty() {
            String::new()
        } else if missing_fields.len() == 1 {
            format!("; check lacks required field {}", missing_fields[0])
        } else {
            format!(
                "; check lacks required fields {}",
                missing_fields.join(", ")
            )
        };
        diagnoses.push(format!(
            "the entry closes early, before \"{next_field}\"{missing}; a closing brace is extra or misplaced"
        ));
        for id in additional_entries {
            diagnoses.push(format!(
                "the reply also holds additional entry {}; return exactly one",
                serde_json::to_string(id).expect("serializing a string"),
            ));
        }
    }
    if reply_blocks
        .iter()
        .any(|block| block.info == "check" && block.content.trim().is_empty())
    {
        diagnoses.push("the check fence is empty; put the script in it".into());
    }

    let mut outside = source.as_bytes().to_vec();
    for block in &reply_blocks {
        outside[block.start..block.end].fill(b' ');
    }
    let outside = String::from_utf8(outside).expect("mask preserves UTF-8");
    let trimmed = outside.trim();
    let starts_entry = trimmed.find(['{', '[']);
    let ends_entry = trimmed.rfind(['}', ']']);
    let has_outside_text = match (starts_entry, ends_entry) {
        (Some(start), Some(end)) => {
            !trimmed[..start].trim().is_empty() || !trimmed[end + 1..].trim().is_empty()
        }
        _ => false,
    } || reply_blocks
        .last()
        .is_some_and(|block| !source[block.end..].trim().is_empty());
    if has_outside_text {
        diagnoses.push("return the entry only".into());
    }
    if matches!(extraction, blocks::ObjectExtraction::NoneFound) {
        diagnoses.push("no entry was found in the reply".into());
    }
    diagnoses
}

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
    if blocks::unclosed_block_refusal_with_markers(reply, markers).is_some() {
        return candidate_entry_with_markers_inner(reply, frozen, markers);
    }
    let array_wrapped = matches!(
        blocks::extract_object_outcome_with_markers(reply, markers),
        blocks::ObjectExtraction::ArrayWrappedEntry(_)
    );
    let result = candidate_entry_with_markers_inner(reply, frozen, markers);
    match result {
        Err(refusal) if array_wrapped => {
            const ARRAY_SHAPE: &str = "return the entry object itself, not an array";
            if refusal.starts_with("the reply is not an entry with id '") {
                Err(ARRAY_SHAPE.into())
            } else if refusal.starts_with(ARRAY_SHAPE) {
                Err(refusal)
            } else {
                Err(format!("{ARRAY_SHAPE}; {refusal}"))
            }
        }
        result => result,
    }
}

fn candidate_entry_with_markers_inner(
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
    let extraction = blocks::extract_object_outcome_with_markers(reply, markers);
    let extracted = match &extraction {
        blocks::ObjectExtraction::Entry(entry)
        | blocks::ObjectExtraction::ArrayWrappedEntry(entry) => entry.clone(),
        blocks::ObjectExtraction::NoneFound
        | blocks::ObjectExtraction::SeveralDifferentEntries { .. }
        | blocks::ObjectExtraction::MisplacedBrace { .. } => {
            let shape = shape_diagnoses(reply, markers, &extraction);
            let details = shape.join(". ");
            return Err(format!(
                "the reply for check '{}' has formatting problems: {details}",
                frozen.id
            ));
        }
    };
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
    if let Err(error) = serde_json::from_str::<serde_json::Value>(&extracted) {
        let has_check_fence = reply_blocks.iter().any(|block| block.info == "check");
        let entry_line = reply_blocks
            .iter()
            .find(|block| block.info == "json")
            .map(|block| {
                block.opened_line
                    + block
                        .content
                        .find('{')
                        .map(|start| block.content[..start].matches('\n').count() + 1)
                        .unwrap_or(1)
            })
            .unwrap_or_else(|| {
                let check_start = reply_blocks
                    .iter()
                    .find(|block| block.info == "check")
                    .map(|block| block.start)
                    .unwrap_or(reply.len());
                reply[..check_start]
                    .find('{')
                    .map(|start| reply[..start].matches('\n').count() + 1)
                    .unwrap_or(1)
            });
        let located = format!("{}{extracted}", "\n".repeat(entry_line.saturating_sub(1)));
        let located_error = serde_json::from_str::<serde_json::Value>(&located)
            .err()
            .unwrap_or(error);
        let diagnostic = crate::command::workflow_json_parse_diagnostic::describe(
            located.as_bytes(),
            &located_error,
        );
        let advice = parse_refusal_advice(
            &diagnostic,
            &extracted,
            has_check_fence,
            visible_block_form_intent(reply, markers),
        );
        return Err(format_parse_refusal(&frozen.id, &diagnostic, advice));
    }
    let mut value: serde_json::Value = match serde_json::from_slice(&document) {
        Ok(value) => value,
        Err(error) => {
            let extracted = blocks::extract_object_with_markers(reply, markers);
            let diagnostic =
                crate::command::workflow_json_parse_diagnostic::describe(&document, &error);
            serde_json::from_str(&extracted).map_err(|_| {
                let has_check_fence = reply_blocks.iter().any(|block| block.info == "check");
                let located_diagnostic = if has_check_fence {
                    // Parse the extracted entry with its original line offset,
                    // so the syntax location remains relative to the received
                    // reply even when the check script follows it.
                    let entry_line = reply_blocks
                        .iter()
                        .find(|block| block.info == "json")
                        .map(|block| {
                            block.opened_line
                                + block
                                    .content
                                    .find('{')
                                    .map(|start| block.content[..start].matches('\n').count() + 1)
                                    .unwrap_or(1)
                        })
                        .unwrap_or_else(|| {
                            let check_start = reply_blocks
                                .iter()
                                .find(|block| block.info == "check")
                                .map(|block| block.start)
                                .unwrap_or(reply.len());
                            reply[..check_start]
                                .find('{')
                                .map(|start| reply[..start].matches('\n').count() + 1)
                                .unwrap_or(1)
                        });
                    let located =
                        format!("{}{extracted}", "\n".repeat(entry_line.saturating_sub(1)));
                    serde_json::from_str::<serde_json::Value>(&located)
                        .err()
                        .map(|error| {
                            crate::command::workflow_json_parse_diagnostic::describe(
                                located.as_bytes(),
                                &error,
                            )
                        })
                        .unwrap_or(diagnostic)
                } else {
                    diagnostic
                };
                let advice = parse_refusal_advice(
                    &located_diagnostic,
                    &extracted,
                    has_check_fence,
                    visible_block_form_intent(reply, markers),
                );
                format_parse_refusal(&frozen.id, &located_diagnostic, advice)
            })?
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
        if !has_fenced_entry_and_check
            && refusal.contains("mixes inline check.command and a check fence")
        {
            return Err(
                "the reply mixes an inline command with a check fence and must use one form".into(),
            );
        }
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
