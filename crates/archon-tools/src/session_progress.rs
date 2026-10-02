//! What one agent session has been doing, kept where the host can read it
//! after the session is gone (Issue-213 C5).
//!
//! A call killed mid-flight produced no result and so said nothing about what
//! it had been doing for hours. The facts that answer that are cheap and are
//! already passing through the host: the turn the runner is on, the tool call
//! it just admitted, the files the write tools just changed. They are recorded
//! here, keyed by the subagent id, so a host holding the session id it
//! dispatched can read them back by prefix (the subagent id begins with it)
//! and write them into the interrupted call's record.
//!
//! Filesystem and process facts only: no tool output is parsed and no
//! language or project is named. Bounded, so an unbounded run of sessions
//! cannot grow it without limit.

use std::collections::{BTreeSet, HashMap};
use std::sync::{LazyLock, Mutex};

/// Sessions kept at once; the least recently touched is dropped first.
const MAX_TRACKED_SESSIONS: usize = 512;
/// Paths kept per session; later ones are counted, not listed.
const MAX_TOUCHED_PATHS: usize = 200;
/// Most recent written paths kept in order, for "what did this round write".
const RECENT_WRITES: usize = 64;
/// Characters of a tool call's arguments kept as its summary.
const TOOL_SUMMARY_CHARS: usize = 300;

/// A snapshot of one session's progress.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct SessionProgress {
    pub agent_id: String,
    /// The last turn the runner started (1-based).
    pub turns: u32,
    /// Tool calls admitted or refused, in total.
    pub tool_calls: u64,
    /// The most recent tool call: its name and a bounded argument summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_tool_call: Option<String>,
    /// Files the session's write tools changed, as the tools resolved them.
    pub touched_paths: Vec<String>,
    /// Writes recorded, including those to paths past the list's bound.
    pub writes: u64,
}

#[derive(Default)]
struct Entry {
    turns: u32,
    tool_calls: u64,
    last_tool_call: Option<String>,
    touched: BTreeSet<String>,
    /// Every write's path, most recent last, bounded.
    recent: std::collections::VecDeque<String>,
    writes: u64,
    tick: u64,
}

#[derive(Default)]
struct Registry {
    entries: HashMap<String, Entry>,
    clock: u64,
}

static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(Mutex::default);

fn with_entry(agent_id: &str, update: impl FnOnce(&mut Entry)) {
    if agent_id.is_empty() {
        return;
    }
    let Ok(mut registry) = REGISTRY.lock() else {
        return;
    };
    registry.clock += 1;
    let tick = registry.clock;
    if !registry.entries.contains_key(agent_id) && registry.entries.len() >= MAX_TRACKED_SESSIONS {
        let oldest = registry
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.tick)
            .map(|(key, _)| key.clone());
        if let Some(key) = oldest {
            registry.entries.remove(&key);
        }
    }
    let entry = registry.entries.entry(agent_id.to_string()).or_default();
    entry.tick = tick;
    update(entry);
}

/// The runner started turn `turn` (1-based).
pub fn note_turn(agent_id: &str, turn: u32) {
    with_entry(agent_id, |entry| entry.turns = entry.turns.max(turn));
}

/// A tool call was admitted or refused.
pub fn note_tool_call(agent_id: &str, tool_name: &str, input: &serde_json::Value) {
    let arguments = input.to_string();
    let mut summary: String = arguments.chars().take(TOOL_SUMMARY_CHARS).collect();
    if arguments.chars().count() > TOOL_SUMMARY_CHARS {
        summary.push_str("...");
    }
    with_entry(agent_id, |entry| {
        entry.tool_calls += 1;
        entry.last_tool_call = Some(format!("{tool_name} {summary}"));
    });
}

/// A write tool changed `path`.
pub fn note_touched(agent_id: &str, path: &std::path::Path) {
    let path = path.display().to_string();
    with_entry(agent_id, |entry| {
        entry.writes += 1;
        if entry.recent.len() == RECENT_WRITES {
            entry.recent.pop_front();
        }
        entry.recent.push_back(path.clone());
        if entry.touched.len() < MAX_TOUCHED_PATHS {
            entry.touched.insert(path);
        }
    });
}

/// Writes recorded for one session so far; `0` for an unknown one.
pub fn writes(agent_id: &str) -> u64 {
    REGISTRY
        .lock()
        .ok()
        .and_then(|registry| registry.entries.get(agent_id).map(|entry| entry.writes))
        .unwrap_or(0)
}

/// The paths of the writes after the first `since` of them, in order: what a
/// round wrote, given the write count before it. At most the last
/// [`RECENT_WRITES`]; `None` when more than that were made, so a caller can
/// tell "nothing" from "too many to list".
pub fn writes_since(agent_id: &str, since: u64) -> Option<Vec<std::path::PathBuf>> {
    let registry = REGISTRY.lock().ok()?;
    let Some(entry) = registry.entries.get(agent_id) else {
        return Some(Vec::new());
    };
    let count = usize::try_from(entry.writes.saturating_sub(since)).unwrap_or(usize::MAX);
    if count > entry.recent.len() {
        return None;
    }
    Some(
        entry
            .recent
            .iter()
            .skip(entry.recent.len() - count)
            .map(std::path::PathBuf::from)
            .collect(),
    )
}

/// Distinct paths the session's write tools changed so far (up to the list's
/// bound); `0` for an unknown one. Grows only when a NEW path is written.
pub fn touched_paths(agent_id: &str) -> usize {
    REGISTRY
        .lock()
        .ok()
        .and_then(|registry| {
            registry
                .entries
                .get(agent_id)
                .map(|entry| entry.touched.len())
        })
        .unwrap_or(0)
}

/// Every session whose id is `session_id` or begins with `session_id-`, the
/// shape a subagent id minted for a dispatched session takes.
pub fn snapshot_for(session_id: &str) -> Vec<SessionProgress> {
    if session_id.is_empty() {
        return Vec::new();
    }
    let Ok(registry) = REGISTRY.lock() else {
        return Vec::new();
    };
    let prefix = format!("{session_id}-");
    let mut found = registry
        .entries
        .iter()
        .filter(|(id, _)| id.as_str() == session_id || id.starts_with(&prefix))
        .map(|(id, entry)| SessionProgress {
            agent_id: id.clone(),
            turns: entry.turns,
            tool_calls: entry.tool_calls,
            last_tool_call: entry.last_tool_call.clone(),
            touched_paths: entry.touched.iter().cloned().collect(),
            writes: entry.writes,
        })
        .collect::<Vec<_>>();
    found.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
    found
}

#[cfg(test)]
#[path = "session_progress_tests.rs"]
mod tests;
