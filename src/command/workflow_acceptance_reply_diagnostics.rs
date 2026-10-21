use super::acceptance_reply_blocks::{reply_blocks_with_markers, strip_reply_markers_with_map};

fn received_line_number(inserted_newline_lines: &[usize], normalized_line: usize) -> usize {
    normalized_line.saturating_sub(
        inserted_newline_lines
            .iter()
            .filter(|inserted_line| **inserted_line <= normalized_line)
            .count(),
    )
}

pub(crate) fn unclosed_block_refusal(content: &str) -> Option<String> {
    unclosed_block_refusal_with_markers(content, &[])
}

pub(crate) fn unclosed_block_refusal_with_markers(
    content: &str,
    markers: &[String],
) -> Option<String> {
    let (normalized, inserted_newline_lines) = strip_reply_markers_with_map(content, markers);
    let content = normalized.as_str();
    let mut inside_fence = false;
    let mut fence_len = 0usize;
    let mut opened_line = 0usize;
    let mut opened_info = String::new();
    let mut info_closer: Option<(usize, String)> = None;
    let mut duplicate_check_line = None;
    let mut saw_check_opener = false;
    for (index, line) in content.lines().enumerate() {
        let spaces = line.chars().take_while(|ch| *ch == ' ').count();
        let suffix = &line[spaces..];
        let ticks = suffix.bytes().take_while(|byte| *byte == b'`').count();
        if inside_fence {
            if spaces <= 3
                && ticks >= fence_len
                && suffix[ticks..].trim_matches([' ', '\t']).is_empty()
            {
                inside_fence = false;
            } else if spaces <= 3 && ticks >= fence_len {
                let info = suffix[ticks..].trim();
                if !info.is_empty() {
                    if info_closer.is_none() {
                        info_closer = Some((
                            received_line_number(&inserted_newline_lines, index + 1),
                            info.to_string(),
                        ));
                    }
                    if info == "check" {
                        if saw_check_opener {
                            duplicate_check_line.get_or_insert(received_line_number(
                                &inserted_newline_lines,
                                index + 1,
                            ));
                        }
                        saw_check_opener = true;
                    }
                }
            }
            continue;
        }
        if let Some(marker) = line.find("```") {
            if line[..marker].chars().any(|ch| !ch.is_whitespace()) {
                let before = line[..marker].trim();
                let mut quote: String = before.chars().take(80).collect();
                if before.chars().count() > 80 {
                    quote.push('…');
                }
                return Some(format!(
                    "fence marker on line {} follows text; a fence must start its own line; text before the marker: {quote:?}",
                    received_line_number(&inserted_newline_lines, index + 1)
                ));
            }
        }
        if spaces <= 3 && ticks >= 3 && !suffix[ticks..].contains('`') {
            inside_fence = true;
            fence_len = ticks;
            if info_closer.is_none() {
                opened_line = received_line_number(&inserted_newline_lines, index + 1);
                opened_info = suffix[ticks..].trim().to_string();
            }
        }
    }
    let blocks = reply_blocks_with_markers(content, markers);
    let block_form_without_check = blocks
        .iter()
        .find(|block| block.info == "json")
        .and_then(|block| serde_json::from_str::<serde_json::Value>(&block.content).ok())
        .and_then(|entry| entry.pointer("/check/command_block").cloned())
        == Some(serde_json::Value::Bool(true))
        && !blocks.iter().any(|block| block.info == "check");
    let malformed_json_entry = blocks.iter().any(|block| {
        block.info == "json" && serde_json::from_str::<serde_json::Value>(&block.content).is_err()
    });
    // Info-string lines are only structural evidence when they explain a
    // refusal. Resolved check script content is never inspected; a JSON block
    // is diagnosed here only when it is malformed or block form has no check.
    let has_check_block = blocks.iter().any(|block| block.info == "check");
    if (inside_fence
        || (opened_info == "json" && info_closer.is_some())
        || (!has_check_block && (malformed_json_entry || block_form_without_check)))
        && let Some((line_number, info)) = info_closer
    {
        let displayed_info = if info == "check" { "check" } else { "<info>" };
        let duplicate = duplicate_check_line.map_or_else(String::new, |line| {
            format!("; line {line} is a duplicate check fence opener")
        });
        return Some(format!(
            "the fence opened on line {opened_line} is not closed; line {line_number} \"```{displayed_info}\" has an info string and cannot close it{duplicate}; put a bare ``` on its own line before the next fence; keep exactly one non-empty check fence for block form"
        ));
    }
    blocks
        .into_iter()
        .find(|block| !block.closed)
        .map(|block| {
            if let Some((line_number, before)) = content.lines().enumerate().find_map(|(index, line)| {
                let marker = line.find("```")?;
                (line[..marker].chars().any(|ch| !ch.is_whitespace())).then(|| (index + 1, line[..marker].trim()))
            }) {
                let mut quote: String = before.chars().take(80).collect();
                if before.chars().count() > 80 { quote.push('…'); }
                let line_number = received_line_number(&inserted_newline_lines, line_number);
                return format!("fence marker on line {line_number} follows text; a fence must start its own line; text before the marker: {quote:?}");
            }
            if let Some((line_number, line)) = content.lines().collect::<Vec<_>>().into_iter().enumerate().rev().find(|(_, line)| !line.trim().is_empty()) {
                let spaces = line.chars().take_while(|ch| *ch == ' ').count();
                let suffix = &line[spaces..];
                let ticks = suffix.bytes().take_while(|byte| *byte == b'`').count();
                if spaces <= 3 && ticks >= 3 && suffix[ticks..].trim_matches([' ', '\t']).is_empty() {
                    let line_number = received_line_number(&inserted_newline_lines, line_number + 1);
                    return format!("closing fence on line {line_number} has no opener; every closing fence must close an open block");
                }
            }
            format!(
                "the ```{} block opened at line {} is never closed (the reply may have been cut off); close every block with a line that is exactly ```",
                block.info, received_line_number(&inserted_newline_lines, block.opened_line)
            )
        })
}

#[cfg(test)]
mod tests {
    use super::unclosed_block_refusal_with_markers;

    #[test]
    fn leaked_marker_inside_open_fence_keeps_received_line_numbers() {
        let reply = "```json\nbad {\"id\":\"A\"} <LEAK> ```\n```check\nrun\n```";
        let finding = unclosed_block_refusal_with_markers(reply, &["<LEAK>".into()])
            .expect("the info fence inside JSON is malformed");
        assert_eq!(
            finding,
            "the fence opened on line 1 is not closed; line 3 \"```check\" has an info string and cannot close it; put a bare ``` on its own line before the next fence; keep exactly one non-empty check fence for block form"
        );
    }

    #[test]
    fn same_line_marker_split_maps_later_fences_to_received_lines() {
        let reply = "{\"id\":\"A\",\"check\":{\"kind\":\"command\",\"command_block\":true}}<LEAK>```json\n{}\n```check\nrun";
        let finding = unclosed_block_refusal_with_markers(reply, &["<LEAK>".into()])
            .expect("the JSON fence remains unclosed");
        assert_eq!(
            finding,
            "the fence opened on line 1 is not closed; line 3 \"```check\" has an info string and cannot close it; put a bare ``` on its own line before the next fence; keep exactly one non-empty check fence for block form"
        );
    }
}
