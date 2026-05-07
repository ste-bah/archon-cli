use std::collections::HashSet;

use anyhow::Result;
use cozo::DbInstance;

#[derive(Debug, Clone, Default)]
pub(super) struct KbRunContext {
    pub(super) pack_id: Option<String>,
    pub(super) text: String,
    pub(super) document_count: usize,
    pub(super) chunk_count: usize,
    pub(super) warning: Option<String>,
}

pub(super) fn situation_with_kb_context(situation: &str, kb: &KbRunContext) -> String {
    if kb.pack_id.is_none() {
        return situation.to_string();
    }

    let pack = kb.pack_id.as_deref().unwrap_or("");
    let warning = kb
        .warning
        .as_ref()
        .map(|w| format!("\nWarning: {w}"))
        .unwrap_or_default();
    let context = if kb.text.trim().is_empty() {
        "No matching document chunks were found.".to_string()
    } else {
        kb.text.clone()
    };

    format!("{situation}\n\n## Knowledge Base Context: {pack}\n{warning}\n\n{context}")
}

pub(super) fn load_kb_run_context(db: &DbInstance, pack_id: Option<&str>) -> Result<KbRunContext> {
    let Some(pack_id) = pack_id.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(KbRunContext::default());
    };

    let docs = match read_doc_source_matches(db, pack_id) {
        Ok(docs) => docs,
        Err(e) => {
            return Ok(KbRunContext {
                pack_id: Some(pack_id.to_string()),
                warning: Some(format!("document store unavailable: {e}")),
                ..KbRunContext::default()
            });
        }
    };

    let doc_ids: HashSet<String> = docs.iter().map(|(id, _)| id.clone()).collect();
    let chunks = read_doc_chunks_for_pack(db, pack_id, &doc_ids)?;
    let text = chunks
        .iter()
        .take(8)
        .map(|(chunk_id, doc_id, content)| {
            format!(
                "### {doc_id} / {chunk_id}\n{}",
                truncate_for_prompt(content, 700)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let warning = if chunks.is_empty() {
        Some(format!("no doc_chunks matched knowledge pack '{pack_id}'"))
    } else {
        None
    };

    Ok(KbRunContext {
        pack_id: Some(pack_id.to_string()),
        text,
        document_count: docs.len(),
        chunk_count: chunks.len(),
        warning,
    })
}

fn read_doc_source_matches(db: &DbInstance, pack_id: &str) -> Result<Vec<(String, String)>> {
    let rows = db
        .run_script(
            "?[document_id, source_path] := *doc_sources{document_id, source_path}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("query doc_sources failed: {e}"))?;

    Ok(rows
        .rows
        .iter()
        .filter_map(|row| {
            let document_id = row.first()?.get_str()?.to_string();
            let source_path = row.get(1)?.get_str()?.to_string();
            let haystack = format!("{document_id}\n{source_path}");
            haystack
                .contains(pack_id)
                .then_some((document_id, source_path))
        })
        .collect())
}

fn read_doc_chunks_for_pack(
    db: &DbInstance,
    pack_id: &str,
    doc_ids: &HashSet<String>,
) -> Result<Vec<(String, String, String)>> {
    let rows = db
        .run_script(
            "?[chunk_id, document_id, content] := *doc_chunks{chunk_id, document_id, content}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("query doc_chunks failed: {e}"))?;

    Ok(rows
        .rows
        .iter()
        .filter_map(|row| {
            let chunk_id = row.first()?.get_str()?.to_string();
            let document_id = row.get(1)?.get_str()?.to_string();
            let content = row.get(2)?.get_str()?.to_string();
            (doc_ids.contains(&document_id) || document_id.contains(pack_id)).then_some((
                chunk_id,
                document_id,
                content,
            ))
        })
        .collect())
}

fn truncate_for_prompt(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated = text.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}
