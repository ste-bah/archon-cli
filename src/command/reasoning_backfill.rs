use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

pub(crate) fn render_backfill(
    root: &Path,
    sessions: Option<usize>,
    emit_world_rows: bool,
    include_llm: bool,
) -> Result<String> {
    let session_store = archon_session::storage::SessionStore::open_default()
        .map_err(|e| anyhow::anyhow!("open session store: {e}"))?;
    let limit = sessions.unwrap_or(usize::MAX).min(u32::MAX as usize) as u32;
    let metas = session_store
        .list_sessions(limit)
        .map_err(|e| anyhow::anyhow!("list sessions: {e}"))?;
    let rq_store = archon_reasoning_quality::store::ReasoningQualityStore::open(root)?;
    let world_root = dirs::home_dir().map(|home| home.join(".archon").join("world-model"));
    let extractor = archon_reasoning_quality::DeterministicExtractor::new(
        archon_reasoning_quality::ExtractorConfig {
            shadow: true,
            ..Default::default()
        },
    );

    let mut sessions_read = 0usize;
    let mut messages_read = 0usize;
    let mut events_written = 0usize;
    for meta in metas {
        let raw = session_store
            .load_messages(&meta.id)
            .map_err(|e| anyhow::anyhow!("load messages for {}: {e}", meta.id))?;
        let mut evidence_refs = Vec::new();
        let mut tool_uses = HashMap::new();
        sessions_read += 1;
        messages_read += raw.len();

        for (idx, raw_message) in raw.iter().enumerate() {
            let Ok(message) = serde_json::from_str::<serde_json::Value>(raw_message) else {
                continue;
            };
            capture_tool_uses(&message, &mut tool_uses);
            capture_tool_results(&message, &tool_uses, &mut evidence_refs);
            let Some(text) = assistant_text(&message) else {
                continue;
            };
            let input = archon_reasoning_quality::ReasoningTurnInput {
                session_id: meta.id.clone(),
                turn_number: idx as u64,
                assistant_text: text,
                evidence_refs: evidence_refs.clone(),
                cwd: Some(meta.working_directory.clone()),
                workspace_root: Some(meta.working_directory.clone()),
                store_raw_text: false,
            };
            let events = extractor.extract_turn(&input);
            if events.is_empty() {
                continue;
            }
            rq_store.append_events(&events)?;
            events_written += events.len();
            if emit_world_rows {
                crate::runtime::reasoning_quality::bridge_reasoning_events(
                    &events,
                    None,
                    root,
                    world_root.as_deref(),
                    true,
                    false,
                );
            }
        }
    }

    Ok(format!(
        "Reasoning Backfill\n\
         ==================\n\
         Mode: deterministic{}\n\
         Sessions read: {}\n\
         Messages read: {}\n\
         Events written: {}\n\
         Emit world rows: {}\n\
         LLM critic requested: {}\n\
         LLM critic status: {}",
        if include_llm { " + llm-requested" } else { "" },
        sessions_read,
        messages_read,
        events_written,
        emit_world_rows,
        include_llm,
        if include_llm {
            "not run during non-interactive backfill without explicit cost confirmation"
        } else {
            "not requested"
        }
    ))
}

#[derive(Debug, Clone)]
struct ToolUseMeta {
    name: String,
    input: serde_json::Value,
}

fn capture_tool_uses(message: &serde_json::Value, tool_uses: &mut HashMap<String, ToolUseMeta>) {
    if message.get("role").and_then(|role| role.as_str()) != Some("assistant") {
        return;
    }
    for block in content_blocks(message) {
        if block.get("type").and_then(|value| value.as_str()) != Some("tool_use") {
            continue;
        }
        let Some(id) = block.get("id").and_then(|value| value.as_str()) else {
            continue;
        };
        tool_uses.insert(
            id.to_string(),
            ToolUseMeta {
                name: block
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or("tool")
                    .to_string(),
                input: block
                    .get("input")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
            },
        );
    }
}

fn capture_tool_results(
    message: &serde_json::Value,
    tool_uses: &HashMap<String, ToolUseMeta>,
    evidence_refs: &mut Vec<archon_reasoning_quality::EvidenceRef>,
) {
    if message.get("role").and_then(|role| role.as_str()) != Some("user") {
        return;
    }
    for block in content_blocks(message) {
        if block.get("type").and_then(|value| value.as_str()) != Some("tool_result") {
            continue;
        }
        let tool_use_id = block
            .get("tool_use_id")
            .and_then(|value| value.as_str())
            .unwrap_or("tool-result");
        let meta = tool_uses.get(tool_use_id);
        let content = block_text(block);
        evidence_refs.push(archon_reasoning_quality::EvidenceRef {
            evidence_id: format!("backfill:{tool_use_id}"),
            kind: evidence_kind(meta.map(|meta| meta.name.as_str()).unwrap_or("tool")),
            entity_key: meta.and_then(|meta| entity_key(&meta.name, &meta.input)),
            output_hash: Some(archon_reasoning_quality::hash_hex(&content)),
            redacted_excerpt: Some(content.chars().take(600).collect()),
            created_at: chrono::Utc::now(),
        });
    }
}

fn assistant_text(message: &serde_json::Value) -> Option<String> {
    if message.get("role").and_then(|role| role.as_str()) != Some("assistant") {
        return None;
    }
    let text = content_blocks(message)
        .into_iter()
        .filter(|block| block.get("type").and_then(|value| value.as_str()) == Some("text"))
        .filter_map(|block| {
            block
                .get("text")
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!text.trim().is_empty()).then_some(text)
}

fn content_blocks(message: &serde_json::Value) -> Vec<&serde_json::Value> {
    match message.get("content") {
        Some(serde_json::Value::Array(blocks)) => blocks.iter().collect(),
        Some(value) => vec![value],
        None => Vec::new(),
    }
}

fn block_text(block: &serde_json::Value) -> String {
    match block.get("content") {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn evidence_kind(tool_name: &str) -> archon_reasoning_quality::EvidenceKind {
    let lower = tool_name.to_lowercase();
    if lower.contains("read") || lower.contains("open") {
        archon_reasoning_quality::EvidenceKind::FileRead
    } else if lower.contains("search") || lower.contains("grep") {
        archon_reasoning_quality::EvidenceKind::Search
    } else {
        archon_reasoning_quality::EvidenceKind::PluginResult
    }
}

fn entity_key(tool_name: &str, input: &serde_json::Value) -> Option<String> {
    for key in ["path", "file_path", "filename", "query", "pattern"] {
        if let Some(value) = input.get(key).and_then(|value| value.as_str()) {
            return Some(value.to_string());
        }
    }
    input
        .get("command")
        .and_then(|value| value.as_str())
        .map(|value| format!("{tool_name}:{}", value.chars().take(80).collect::<String>()))
}
