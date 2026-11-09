//! Pure selection and text-derived questions for inspection.

use std::collections::BTreeMap;

use crate::task_set_contract::AcceptanceContract;

pub const INSPECTION_LOGIC: u32 = 1;

/// Whether text explicitly names an artifact or command.
pub fn names_path_command_or_file(text: &str) -> bool {
    // The template asks for artifacts and commands in backticks (assets/templates/workflow-prd.md §5.1).
    if let (Some(first), Some(last)) = (text.find('`'), text.rfind('`'))
        && last > first + 1
    {
        return true;
    }
    text.split_whitespace().any(|word| {
        let word = word
            .trim_start_matches(['(', '[', '"', '\''])
            .trim_end_matches([')', ']', '"', '\'', '.', ',', ';', ':', '!', '?']);
        let bytes = word.as_bytes();
        let rooted = (word.len() > 1 && word.starts_with(['/', '\\']))
            || word.starts_with("./")
            || word.starts_with("../")
            || word.starts_with("~/")
            || (bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && matches!(bytes[2], b'/' | b'\\'));
        rooted || has_extension_after_separator(word)
    })
}

fn has_extension_after_separator(word: &str) -> bool {
    let Some((_, last)) = word.rsplit_once(['/', '\\']) else {
        return false;
    };
    let Some((stem, extension)) = last.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && (1..=10).contains(&extension.len())
        && extension.bytes().all(|ch| ch.is_ascii_alphanumeric())
        && extension.bytes().any(|ch| ch.is_ascii_alphabetic())
}

/// Constraints whose text can be inspected without an explicitly named artifact.
pub fn inspection_eligible(prd: &str) -> BTreeMap<String, String> {
    let acceptance = crate::obligation_ids::acceptance_ids(prd);
    crate::obligation_ids::constraint_texts(prd)
        .into_iter()
        .filter(|(id, text)| !acceptance.contains(id) && !names_path_command_or_file(text))
        .collect()
}

/// Inspection closes only constraints no frozen check covers.
pub fn inspection_items(prd: &str, contract: &AcceptanceContract) -> BTreeMap<String, String> {
    let covered = super::coverage::covered_requirements(contract);
    inspection_eligible(prd)
        .into_iter()
        .filter(|(id, _)| !covered.contains(id))
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub struct ChecklistQuestion {
    pub number: u32,
    pub statement: String,
}

/// Questions preserve all text, including punctuation, in source order.
pub fn checklist(text: &str) -> Vec<ChecklistQuestion> {
    let mut statements: Vec<String> = Vec::new();
    let (mut start, mut parentheses, mut brackets, mut backticks) = (0, 0u32, 0u32, false);
    for (index, ch) in text.char_indices() {
        match ch {
            '`' => backticks = !backticks,
            '(' if !backticks => parentheses += 1,
            ')' if !backticks => parentheses = parentheses.saturating_sub(1),
            '[' if !backticks => brackets += 1,
            ']' if !backticks => brackets = brackets.saturating_sub(1),
            _ => {}
        }
        let end = index + ch.len_utf8();
        if matches!(ch, '.' | ';' | '!' | '?')
            && !backticks
            && parentheses == 0
            && brackets == 0
            && text[end..].chars().next().is_none_or(char::is_whitespace)
            && !(ch == '.' && protected_dot(text, index))
        {
            statements.push(text[start..end].trim().to_string());
            start = end;
        }
    }
    if !text[start..].trim().is_empty() || statements.is_empty() {
        statements.push(text[start..].trim().to_string());
    }
    let mut merged: Vec<String> = Vec::new();
    for statement in statements {
        let capped = merged.len() == 8;
        if let Some(previous) = merged.last_mut()
            && (statement.split_whitespace().count() < 3
                || previous.split_whitespace().count() < 3
                || capped)
        {
            previous.push(' ');
            previous.push_str(&statement);
        } else {
            merged.push(statement);
        }
    }
    merged
        .into_iter()
        .enumerate()
        .map(|(index, statement)| ChecklistQuestion {
            number: index as u32 + 1,
            statement,
        })
        .collect()
}

fn protected_dot(text: &str, index: usize) -> bool {
    let before = &text[..index];
    let after = &text[index + 1..];
    let decimal = before
        .chars()
        .next_back()
        .is_some_and(|ch| ch.is_ascii_digit())
        && after.chars().next().is_some_and(|ch| ch.is_ascii_digit());
    let single_letter = before
        .chars()
        .rev()
        .take_while(|ch| ch.is_alphabetic())
        .count()
        == 1;
    decimal || single_letter
}

#[cfg(test)]
#[path = "inspection_items_tests.rs"]
mod tests;
