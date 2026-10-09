//! CommonMark list continuation rules for obligation text.

use super::{is_list_item, is_setext_underline};

/// Append lazy or indented prose, plus prose from ordinary nested sub-bullets.
/// A sub-bullet with its own obligation ID stays an independent obligation.
pub(super) fn append(
    lines: &[&str],
    start: usize,
    item_indentation: usize,
    text: &mut Vec<String>,
) {
    for (index, line) in lines.iter().enumerate().skip(start) {
        if line.trim().is_empty() {
            break;
        }
        let trimmed = line.trim_start();
        if lines
            .get(index + 1)
            .is_some_and(|underline| is_setext_underline(underline))
        {
            break;
        }
        let indentation = line.len() - trimmed.len();
        if is_list_item(trimmed) {
            if indentation <= item_indentation || starts_with_obligation_id(trimmed) {
                break;
            }
            let bullet_text = list_item_body(trimmed);
            if !bullet_text.is_empty() {
                text.push(format!(";{bullet_text}"));
            }
            continue;
        }
        if is_block_start(trimmed) {
            break;
        }
        // Prose at or below the marker's indentation is a lazy continuation.
        let _lazy_continuation = indentation <= item_indentation;
        text.push(trimmed.to_string());
    }
}

pub(super) fn join(parts: &[String]) -> String {
    let mut joined = String::new();
    for part in parts {
        if let Some(nested) = part.strip_prefix(';') {
            joined.push_str("; ");
            joined.push_str(nested.trim());
        } else {
            if !joined.is_empty() {
                joined.push(' ');
            }
            joined.push_str(part.trim());
        }
    }
    joined.trim().to_string()
}

fn is_block_start(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with('#')
        || line.starts_with('|')
        || line.starts_with("```")
        || line.starts_with("~~~")
        || line.starts_with('<')
        || line.starts_with('>')
        || matches!(line, "---" | "***")
}

fn starts_with_obligation_id(line: &str) -> bool {
    let body = list_item_body(line);
    ["REQ-", "AC-", "G-", "DONE-"]
        .iter()
        .any(|prefix| body.starts_with(prefix))
}

fn list_item_body(line: &str) -> &str {
    let line = line.trim_start();
    if let Some(rest) = line.strip_prefix(['-', '*', '+']) {
        return rest.trim_start();
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && line[digits..].starts_with(['.', ')']) {
        return line[digits + 1..].trim_start();
    }
    line
}
