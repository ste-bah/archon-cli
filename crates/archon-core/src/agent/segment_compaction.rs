use archon_session::storage::{CompactionLedgerRecord, CompactionSegment, CompactionSummaryStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactedAssembly {
    pub messages: Vec<serde_json::Value>,
    pub swapped_segment_ids: Vec<String>,
    pub digest_fallback_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionModelSource {
    Explicit,
    ProviderPolicy,
    ActiveFallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCompactionModel {
    pub model: String,
    pub source: CompactionModelSource,
    pub fallback_reason: Option<String>,
}

pub fn next_closed_segment_span(
    messages: &[serde_json::Value],
    start: usize,
    preserve_recent_messages: usize,
) -> Option<SegmentSpan> {
    let exclusive_end = messages.len().checked_sub(preserve_recent_messages)?;
    if start >= exclusive_end {
        return None;
    }
    let end = exclusive_end - 1;
    if assistant_tool_ids(&messages[end]).is_empty() {
        return safe_start(messages, start).then_some(SegmentSpan { start, end });
    }
    let result_index = end + 1;
    if result_index >= exclusive_end
        || !results_cover_tool_ids(&messages[result_index], &messages[end])
    {
        return None;
    }
    safe_start(messages, start).then_some(SegmentSpan {
        start,
        end: result_index,
    })
}

pub fn deterministic_segment_digest(
    segment_id: &str,
    source_start: usize,
    messages: &[serde_json::Value],
    limit_bytes: usize,
) -> String {
    let mut lines = vec![format!(
        "[Archon deterministic segment digest: id={segment_id}; span={source_start}..{}]",
        source_start + messages.len().saturating_sub(1)
    )];
    for (offset, message) in messages.iter().enumerate() {
        append_message_digest(&mut lines, source_start + offset, message);
    }
    cap_text(&lines.join("\n"), limit_bytes)
}

pub fn derive_segment_ledger(
    session_id: &str,
    source_start: usize,
    messages: &[serde_json::Value],
    limit_bytes: usize,
) -> Vec<CompactionLedgerRecord> {
    let mut records = Vec::new();
    for (offset, message) in messages.iter().enumerate() {
        let source_index = source_start + offset;
        append_ledger_records(&mut records, session_id, source_index, message, limit_bytes);
    }
    records
}

pub fn assemble_compacted_messages<F>(
    messages: &[serde_json::Value],
    segments: &[CompactionSegment],
    ledger: &[CompactionLedgerRecord],
    preserve_recent_messages: usize,
    limit_bytes: usize,
    _provider_call: F,
) -> CompactedAssembly
where
    F: Fn(&CompactionSegment),
{
    let recent_start = messages.len().saturating_sub(preserve_recent_messages);
    let mut eligible: Vec<&CompactionSegment> = segments
        .iter()
        .filter(|segment| segment.end_index < recent_start as u64)
        .collect();
    eligible.sort_by_key(|segment| segment.start_index);

    let mut assembled = Vec::new();
    let mut swapped = Vec::new();
    let mut digest_count = 0;
    let mut cursor = 0;
    for segment in eligible {
        let start = segment.start_index as usize;
        let end = segment.end_index as usize;
        if start < cursor || end >= messages.len() || start > end {
            continue;
        }
        assembled.extend_from_slice(&messages[cursor..start]);
        let source = &messages[start..=end];
        if segment.summary_status == CompactionSummaryStatus::Redacted {
            assembled.push(serde_json::json!({
                "role": "user",
                "content": format!("[Compacted segment {}: redacted]", segment.id),
            }));
            swapped.push(segment.id.clone());
            cursor = end + 1;
            continue;
        }
        let replacement = if segment.summary_status == CompactionSummaryStatus::Succeeded {
            segment
                .summary
                .clone()
                .filter(|summary| !summary.trim().is_empty())
                .unwrap_or_else(|| {
                    digest_count += 1;
                    deterministic_segment_digest(&segment.id, start, source, limit_bytes)
                })
        } else {
            digest_count += 1;
            deterministic_segment_digest(&segment.id, start, source, limit_bytes)
        };
        let ledger_text = ledger_for_span(ledger, segment.start_index, segment.end_index);
        let ledger_text = if ledger_text.is_empty() {
            let derived = derive_segment_ledger(&segment.session_id, start, source, limit_bytes);
            ledger_for_span(&derived, segment.start_index, segment.end_index)
        } else {
            ledger_text
        };
        let content = cap_text(
            &format!(
                "[Compacted segment {}]\n{}\n[Deterministic ledger]\n{}",
                segment.id, replacement, ledger_text
            ),
            limit_bytes,
        );
        assembled.push(serde_json::json!({"role":"user","content":content}));
        swapped.push(segment.id.clone());
        cursor = end + 1;
    }
    assembled.extend_from_slice(&messages[cursor..]);
    CompactedAssembly {
        messages: assembled,
        swapped_segment_ids: swapped,
        digest_fallback_count: digest_count,
    }
}

pub fn bound_recalled_segment(segment_id: &str, body: &[String], limit_bytes: usize) -> String {
    cap_text(
        &format!("[Recalled segment {segment_id}]\n{}", body.join("\n")),
        limit_bytes,
    )
}

pub fn resolve_compaction_model(
    explicit: Option<&str>,
    provider_default: Option<&str>,
    active: &str,
    available: &[&str],
) -> ResolvedCompactionModel {
    if let Some(model) = explicit.filter(|model| available.contains(model)) {
        return resolved(model, CompactionModelSource::Explicit, None);
    }
    if let Some(model) = provider_default.filter(|model| available.contains(model)) {
        let reason = explicit.map(|_| "explicit model unavailable".to_string());
        return resolved(model, CompactionModelSource::ProviderPolicy, reason);
    }
    let fallback_reason = match (explicit, provider_default) {
        (Some(_), Some(_)) => {
            Some("explicit model unavailable; provider policy model unavailable".into())
        }
        (Some(_), None) => Some("explicit model unavailable".into()),
        (None, Some(_)) => Some("provider policy model unavailable".into()),
        (None, None) => None,
    };
    resolved(
        active,
        CompactionModelSource::ActiveFallback,
        fallback_reason,
    )
}

fn resolved(
    model: &str,
    source: CompactionModelSource,
    fallback_reason: Option<String>,
) -> ResolvedCompactionModel {
    ResolvedCompactionModel {
        model: model.to_string(),
        source,
        fallback_reason,
    }
}

fn safe_start(messages: &[serde_json::Value], start: usize) -> bool {
    start == 0 || !is_tool_result_message(&messages[start])
}

fn is_tool_result_message(message: &serde_json::Value) -> bool {
    message
        .get("content")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|blocks| {
            !blocks.is_empty()
                && blocks.iter().all(|block| {
                    block.get("type").and_then(serde_json::Value::as_str) == Some("tool_result")
                })
        })
}

fn assistant_tool_ids(message: &serde_json::Value) -> Vec<&str> {
    message
        .get("content")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(serde_json::Value::as_str) == Some("tool_use"))
        .filter_map(|block| block.get("id").and_then(serde_json::Value::as_str))
        .collect()
}

fn results_cover_tool_ids(result: &serde_json::Value, tool_use: &serde_json::Value) -> bool {
    let expected = assistant_tool_ids(tool_use);
    let actual: Vec<&str> = result
        .get("content")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|block| block.get("tool_use_id").and_then(serde_json::Value::as_str))
        .collect();
    !expected.is_empty() && expected.iter().all(|id| actual.contains(id))
}

fn append_message_digest(lines: &mut Vec<String>, index: usize, message: &serde_json::Value) {
    let role = message
        .get("role")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    if let Some(text) = message.get("content").and_then(serde_json::Value::as_str) {
        lines.push(format!("turn {index} {role}: {text}"));
        return;
    }
    for block in message
        .get("content")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        append_block_digest(lines, index, role, block);
    }
}

fn append_block_digest(
    lines: &mut Vec<String>,
    index: usize,
    role: &str,
    block: &serde_json::Value,
) {
    match block.get("type").and_then(serde_json::Value::as_str) {
        Some("tool_use") => lines.push(format!(
            "turn {index} {role} tool={} id={} input={}",
            block
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(""),
            block
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(""),
            block.get("input").cloned().unwrap_or_default()
        )),
        Some("tool_result") => lines.push(format!(
            "turn {index} {role} result id={} error={} content={}",
            block
                .get("tool_use_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(""),
            block
                .get("is_error")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            block
                .get("content")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
        )),
        Some("text") => lines.push(format!(
            "turn {index} {role}: {}",
            block
                .get("text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
        )),
        _ => {}
    }
}

fn append_ledger_records(
    records: &mut Vec<CompactionLedgerRecord>,
    session_id: &str,
    source_index: usize,
    message: &serde_json::Value,
    limit_bytes: usize,
) {
    if message.get("role").and_then(serde_json::Value::as_str) == Some("user")
        && let Some(text) = message.get("content").and_then(serde_json::Value::as_str)
    {
        records.push(ledger_record(
            session_id,
            source_index,
            "user_directive",
            cap_text(text, limit_bytes),
            0,
        ));
    }
    for (block_index, block) in message
        .get("content")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let kind = match block.get("type").and_then(serde_json::Value::as_str) {
            Some("tool_use") => "tool_call",
            Some("tool_result")
                if block.get("is_error").and_then(serde_json::Value::as_bool) != Some(true) =>
            {
                "verified_outcome"
            }
            _ => continue,
        };
        records.push(ledger_record(
            session_id,
            source_index,
            kind,
            cap_text(&block.to_string(), limit_bytes),
            block_index,
        ));
    }
}

fn ledger_record(
    session_id: &str,
    source_index: usize,
    kind: &str,
    payload: String,
    ordinal: usize,
) -> CompactionLedgerRecord {
    CompactionLedgerRecord {
        id: format!("ledger:{session_id}:{source_index}:{kind}:{ordinal}"),
        session_id: session_id.to_string(),
        kind: kind.to_string(),
        payload,
        source_start_index: source_index as u64,
        source_end_index: source_index as u64,
        created_at: String::new(),
    }
}

fn ledger_for_span(ledger: &[CompactionLedgerRecord], start: u64, end: u64) -> String {
    ledger
        .iter()
        .filter(|record| record.source_start_index >= start && record.source_end_index <= end)
        .map(|record| format!("{}: {}", record.kind, record.payload))
        .collect::<Vec<_>>()
        .join("\n")
}

fn cap_text(content: &str, limit_bytes: usize) -> String {
    crate::agent::tool_result_context::cap_tool_output_to_bytes(content, limit_bytes).content
}
