//! The no-progress window of one subagent session (Issue 288).
//!
//! A session's own limit is a no-progress window, never a total. It opens
//! when the session starts and renews only on NOVEL activity: a tool call
//! whose tool and canonical arguments this session has not made before, or
//! assistant text this session has not produced before. A repeated identical
//! call, an alternation between two identical calls and repeated text renew
//! nothing, so a looping session ends one window after its last novel
//! activity, and a session that keeps doing new work runs for as long as it
//! does. There is no turn-count cap here.
//!
//! The same novelty renews the host's dispatch clocks
//! (`archon_tools::subagent_dispatch_clock::progress`), so the runner and the
//! host clocks measure one window from one signal. The expiry is a host cut
//! whose text names the last novel activity; a workflow reads it as a host
//! call timeout, a resumable stop, never as a verdict on the work.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use tokio::time::Instant;

use super::*;

/// How much of a novel activity its record quotes.
const ACTIVITY_PREVIEW_CHARS: usize = 160;

pub(super) struct ProgressWindow {
    window: Option<Duration>,
    deadline: Option<Instant>,
    renewed_at: Instant,
    seen: HashSet<u64>,
    last: Option<String>,
}

impl ProgressWindow {
    /// Open the window now; `None` is unlimited.
    pub(super) fn open(window_secs: Option<u64>) -> anyhow::Result<Self> {
        let now = Instant::now();
        let window = window_secs.map(Duration::from_secs);
        let deadline = window
            .map(|window| {
                now.checked_add(window)
                    .ok_or_else(|| anyhow::anyhow!("host timeout exceeds supported clock range"))
            })
            .transpose()?;
        Ok(Self {
            window,
            deadline,
            renewed_at: now,
            seen: HashSet::new(),
            last: None,
        })
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    pub(super) fn expired(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    /// Move the deadline by credit the session earned (a cargo lock wait).
    pub(super) fn extend(&mut self, by: Duration) {
        self.deadline = self.deadline.map(|deadline| deadline + by);
    }

    /// Observe one finished model turn: its text and the tool calls it made.
    /// Any novel part renews the window; `true` when it did.
    pub(super) fn observe_turn(&mut self, turn: u32, text: &str, tools: &[PendingTool]) -> bool {
        let mut novel = None;
        let text = text.trim();
        if !text.is_empty() && self.seen.insert(digest(&("text", text))) {
            novel = Some(format!(
                "turn {turn}: new assistant text \"{}\"",
                preview(text)
            ));
        }
        for tool in tools {
            let arguments = serde_json::from_str::<serde_json::Value>(&tool.input_json)
                .map(|value| archon_tools::repeat_tool_guard::canonical_arguments(&value))
                .unwrap_or_else(|_| tool.input_json.trim().to_string());
            if self
                .seen
                .insert(digest(&("tool", tool.name.as_str(), arguments.as_str())))
            {
                novel = Some(format!(
                    "turn {turn}: new tool call {} {}",
                    tool.name,
                    preview(&arguments)
                ));
            }
        }
        let Some(activity) = novel else {
            return false;
        };
        let now = Instant::now();
        self.renewed_at = now;
        if let Some(window) = self.window {
            self.deadline = now.checked_add(window).or(self.deadline);
        }
        archon_tools::subagent_dispatch_clock::progress(&activity);
        self.last = Some(activity);
        true
    }

    /// The error the session ends with when the window ran out in `phase`.
    pub(super) fn stall_error(&self, phase: &str, turn: u32) -> anyhow::Error {
        anyhow::anyhow!(
            "subagent timed out after {}s without progress {phase} at turn {turn}: no new tool \
             call and no new assistant text for its no-progress window of {}s, which renews \
             on novel activity only; {}",
            self.renewed_at.elapsed().as_secs(),
            self.window.map_or(0, |window| window.as_secs()),
            archon_tools::subagent_dispatch_clock::last_progress_text(self.last.as_deref()),
        )
    }
}

fn digest(value: &impl Hash) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn preview(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= ACTIVITY_PREVIEW_CHARS {
        return flat;
    }
    let mut kept: String = flat.chars().take(ACTIVITY_PREVIEW_CHARS).collect();
    kept.push_str("...");
    kept
}
