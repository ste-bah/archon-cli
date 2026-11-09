//! Generic messages and host-constructed changes from unique text replacements.
use super::*;
use crate::command::workflow_task_set::acceptance_reply_blocks::reply_blocks;
use archon_workflow::breaker_verdict::rule::requirement_span;
use serde::Deserialize;
#[derive(Deserialize)]
struct Reply {
    mutants: Vec<serde_json::Value>,
}
#[derive(Deserialize)]
struct Proposal {
    kind: MutantKind,
    breaks: String,
    edits: Vec<Edit>,
}
#[derive(Deserialize)]
struct Edit {
    key: String,
    find: String,
    replace: String,
}
pub(super) struct Parsed {
    pub(super) mutants: Vec<(SuppliedMutant, String)>,
    pub(super) refusals: Vec<String>,
}
pub(super) fn prompt(id: &str, text: &str, view: &BaseView, feedback: &str) -> String {
    let files: Vec<_> = view
        .files
        .iter()
        .map(|(key, f)| serde_json::json!({"key":key,"bytes":f.bytes.len()}))
        .collect();
    let quote = "three or more exact requirement words";
    let quote = serde_json::to_string(&quote).expect("requirement phrase");
    format!(
        "Requirement {id}:\n{text}\nRead files in {}.\nFiles: {}\n1. Return 2 to 5 behaviour mutants in the mutants list.\n2. Put only a contiguous span of at least three exact requirement words in breaks; put any explanation in optional why.\n3. Use only listed files and keep each change able to build and run.\n4. Each find must occur exactly once in the original file; replace must change its content.\n5. Read files only; do not run commands or write files.\n6. Return one JSON object; replace the example values with real edits:\n{{\"mutants\":[{{\"kind\":\"behaviour\",\"breaks\":{quote},\"why\":\"explanation of the changed behaviour\",\"edits\":[{{\"key\":\"listed key\",\"find\":\"unique original text\",\"replace\":\"changed text\"}}]}}]}}\n{feedback}",
        view.root().display(),
        serde_json::to_string(&files).expect("file list")
    )
}
pub(super) fn gut_prompt(text: &str, view: &BaseView, key: &str, feedback: &str) -> String {
    let example = serde_json::json!({"key":"listed key","find":"complete original body","replace":"minimal body"});
    format!(
        "Requirement: {text}\nRead the listed files {key} in {}.\n1. Pick one function or block in one listed file that implements the requirement.\n2. Copy its complete body exactly into find.\n3. Put the smallest body that still builds and runs into replace, removing the stated behaviour.\n4. Use only the listed files; read only, do not run commands or write files.\n5. Return one JSON object with real values:\n{example}\n{feedback}",
        view.root().display()
    )
}
pub(super) fn parse_gut(
    reply: &str,
    task: &str,
    id: &str,
    text: &str,
    view: &BaseView,
    key: &str,
) -> Result<Parsed, String> {
    let edit: Edit =
        serde_json::from_value(object(reply)?).map_err(|e| format!("proposal_invalid: {e}"))?;
    if !serde_json::from_str::<Vec<String>>(key)
        .map_or_else(|_| edit.key == key, |keys| keys.contains(&edit.key))
    {
        return Err("key_not_target".into());
    }
    parse(&serde_json::json!({"mutants":[{"kind":"body_gut","breaks":"","edits":[{"key":edit.key,"find":edit.find,"replace":edit.replace}]}]}).to_string(), task, id, text, view)
}

pub(super) fn object(text: &str) -> Result<serde_json::Value, String> {
    let blocks = reply_blocks(text);
    if let Some(block) = blocks.iter().find(|b| b.info == "json") {
        if !block.closed {
            return Err("unterminated_reply".into());
        }
        return serde_json::from_str(&block.content).map_err(|e| format!("invalid_reply: {e}"));
    }
    let start = text.find('{').ok_or("invalid_reply")?;
    serde_json::Deserializer::from_str(&text[start..])
        .into_iter::<serde_json::Value>()
        .next()
        .ok_or_else(|| "invalid_reply".to_string())?
        .map_err(|e| format!("invalid_reply: {e}"))
}
pub(super) fn parse(
    reply: &str,
    task: &str,
    id: &str,
    text: &str,
    view: &BaseView,
) -> Result<Parsed, String> {
    let reply: Reply =
        serde_json::from_value(object(reply)?).map_err(|e| format!("invalid_reply: {e}"))?;
    let mut result = Parsed {
        mutants: vec![],
        refusals: vec![],
    };
    for raw in reply.mutants {
        let raw_display: String = raw.to_string().chars().take(2048).collect();
        let mut m: Proposal = match serde_json::from_value(raw.clone()) {
            Ok(m) => m,
            Err(e) => {
                result
                    .refusals
                    .push(format!("proposal_invalid: {e}: {raw_display}"));
                continue;
            }
        };
        if m.kind == MutantKind::Behaviour {
            if let Some(span) = requirement_span(text, &m.breaks) {
                m.breaks = span;
            } else {
                result
                    .refusals
                    .push(format!("breaks_not_in_requirement: {raw_display}"));
                continue;
            }
        }
        let make = || -> Result<String, &str> {
            let mut images = std::collections::BTreeMap::new();
            for edit in &m.edits {
                let file = view.files.get(&edit.key).ok_or("key_not_in_view")?;
                let before = std::str::from_utf8(&file.bytes).map_err(|_| "non_text_edit")?;
                if edit.find.is_empty() {
                    return Err("find_ambiguous");
                }
                match occurrences(before, &edit.find) {
                    0 => return Err("find_absent"),
                    1 => {}
                    _ => return Err("find_ambiguous"),
                }
                let after = images
                    .entry(edit.key.clone())
                    .or_insert_with(|| before.to_string());
                if occurrences(after, &edit.find) != 1 {
                    return Err("overlapping_edits");
                }
                *after = after.replacen(&edit.find, &edit.replace, 1);
            }
            let mut patch = String::new();
            for (key, after) in images {
                let before =
                    std::str::from_utf8(&view.files[&key].bytes).map_err(|_| "non_text_edit")?;
                if before.split_whitespace().eq(after.split_whitespace()) {
                    continue;
                }
                patch.push_str(&archon_workflow::breaker_verdict::rule::edit_patch(
                    &key, before, &after,
                ));
            }
            if patch.is_empty() {
                return Err("patch_empty");
            }
            Ok(patch)
        };
        match make() {
            Ok(patch) => {
                let limit = if m.kind == MutantKind::Behaviour { 5 } else { 1 };
                if result.mutants.iter().filter(|(v,_)|v.kind == m.kind).count() >= limit {
                    result.refusals.push(format!("proposal_over_limit: {raw_display}"));
                    continue;
                }
                result.mutants.push((
                SuppliedMutant {
                    task_id: task.into(),
                    requirement_id: id.into(),
                    kind: m.kind,
                    patch,
                },
                m.breaks,
            ));
            },
            Err(code) => result.refusals.push(format!("{code}: {}", serde_json::json!({"breaks": m.breaks, "keys": m.edits.iter().map(|e| &e.key).collect::<Vec<_>>()}))),
        }
    }
    for refusal in &mut result.refusals {
        *refusal = refusal.chars().take(2048).collect();
    }
    if result.mutants.is_empty() {
        return Err(format!(
            "no_valid_proposals: behaviour=0 body_gut=0; {}",
            result.refusals.join("; ")
        ));
    }
    Ok(result)
}

fn occurrences(text: &str, needle: &str) -> usize {
    text.char_indices()
        .filter(|(i, _)| text[*i..].starts_with(needle))
        .count()
}
