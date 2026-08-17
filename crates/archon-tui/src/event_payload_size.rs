use std::mem::size_of;
use std::path::PathBuf;

use crate::events::TuiEvent;

pub(super) fn heap_bytes(event: &TuiEvent) -> usize {
    match event {
        TuiEvent::TextDelta(text)
        | TuiEvent::ThinkingDelta(text)
        | TuiEvent::TransientThinkingDelta(text)
        | TuiEvent::Error(text)
        | TuiEvent::ModelChanged(text)
        | TuiEvent::BtwResponse(text)
        | TuiEvent::SessionRenamed(text)
        | TuiEvent::PermissionModeChanged(text)
        | TuiEvent::SetTheme(text)
        | TuiEvent::VoiceText(text) => string_bytes(text),
        TuiEvent::ToolStart { name, id } => string_bytes(name) + string_bytes(id),
        TuiEvent::ToolOutputChunk { id, chunk } => string_bytes(id) + string_bytes(chunk),
        TuiEvent::ToolComplete {
            name,
            id,
            output,
            transcript_summary,
            ..
        } => {
            string_bytes(name)
                + string_bytes(id)
                + string_bytes(output)
                + transcript_summary.as_ref().map_or(0, string_bytes)
        }
        TuiEvent::PermissionPrompt { tool, description } => {
            string_bytes(tool) + string_bytes(description)
        }
        TuiEvent::AskUserPrompt { question, .. } => string_bytes(question),
        TuiEvent::ShowSessionPicker(entries) => {
            vec_bytes(entries) + entries.iter().map(session_entry_bytes).sum::<usize>()
        }
        TuiEvent::ShowMcpManager(entries) | TuiEvent::UpdateMcpManager(entries) => {
            vec_bytes(entries) + entries.iter().map(mcp_entry_bytes).sum::<usize>()
        }
        TuiEvent::ShowMessageSelector(entries) => {
            vec_bytes(entries)
                + entries
                    .iter()
                    .map(|entry| string_bytes(&entry.id) + string_bytes(&entry.preview))
                    .sum::<usize>()
        }
        TuiEvent::ShowSkillsMenu(entries) => {
            vec_bytes(entries)
                + entries
                    .iter()
                    .map(|entry| string_bytes(&entry.name) + string_bytes(&entry.description))
                    .sum::<usize>()
        }
        TuiEvent::ShowFilePicker { root, entries } => {
            path_bytes(root) + file_entries_bytes(entries)
        }
        TuiEvent::ShowSearchResults { query, entries } => {
            string_bytes(query) + file_entries_bytes(entries)
        }
        TuiEvent::OpenViewRows { rows, .. } => {
            vec_bytes(rows)
                + rows
                    .iter()
                    .map(|row| {
                        string_bytes(&row.id)
                            + string_bytes(&row.title)
                            + string_bytes(&row.status)
                            + string_bytes(&row.detail)
                    })
                    .sum::<usize>()
        }
        TuiEvent::VideoIngestProgress(update) => {
            string_bytes(&update.video_id)
                + string_bytes(&update.latest_text)
                + string_bytes(&update.status)
        }
        TuiEvent::AgentActivity(update) => {
            string_bytes(&update.id)
                + string_bytes(&update.name)
                + option_string_bytes(&update.current_tool)
                + option_string_bytes(&update.detail)
                + option_string_bytes(&update.run_id)
                + option_string_bytes(&update.parent_id)
                + option_string_bytes(&update.artifact_id)
                + option_string_bytes(&update.provider)
                + option_string_bytes(&update.model)
        }
        TuiEvent::ActivityStream(update) => {
            string_bytes(&update.id)
                + string_bytes(&update.name)
                + option_string_bytes(&update.provider)
                + option_string_bytes(&update.model)
                + string_bytes(&update.text)
                + option_string_bytes(&update.tool)
        }
        TuiEvent::ContextPressureUpdated {
            context_name,
            resolution_source,
            ..
        } => option_string_bytes(context_name) + option_string_bytes(resolution_source),
        TuiEvent::SetAgentInfo { name, color } => string_bytes(name) + option_string_bytes(color),
        _ => 0,
    }
}

fn string_bytes(value: &String) -> usize {
    value.capacity()
}

fn option_string_bytes(value: &Option<String>) -> usize {
    value.as_ref().map_or(0, string_bytes)
}

fn vec_bytes<T>(value: &Vec<T>) -> usize {
    value.capacity().saturating_mul(size_of::<T>())
}

fn file_entries_bytes(entries: &Vec<crate::events::FileEntry>) -> usize {
    vec_bytes(entries)
        + entries
            .iter()
            .map(|entry| string_bytes(&entry.name) + path_bytes(&entry.path))
            .sum::<usize>()
}

fn path_bytes(value: &PathBuf) -> usize {
    value.capacity()
}

fn session_entry_bytes(entry: &crate::events::SessionPickerEntry) -> usize {
    string_bytes(&entry.id) + string_bytes(&entry.name) + string_bytes(&entry.last_active)
}

fn mcp_entry_bytes(entry: &crate::events::McpServerEntry) -> usize {
    string_bytes(&entry.name)
        + string_bytes(&entry.state)
        + vec_bytes(&entry.tools)
        + entry.tools.iter().map(string_bytes).sum::<usize>()
}
