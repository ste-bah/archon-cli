//! Consecutive identical tool calls, counted per agent (#200 Phase 2).
//!
//! A model that has stopped making progress often does not stop calling. It
//! reissues the same `Grep`, the same `Read`, the same denied `Bash`, and every
//! repetition costs a round trip and a slice of the context window while the
//! answer stays exactly as absent as it was the first time. Nothing in the tree
//! noticed that before this module: there was no consecutive-call tracking
//! anywhere in the agent loop or the tool layer.
//!
//! What this records is a *run*: how many times in a row one agent has called
//! one tool with arguments that canonicalise to the same string. At the
//! configured run lengths it produces an advisory addressed to the model. It
//! never vetoes a call, never rewrites one, and never delays one — the decision
//! stays where it was, and a legitimately repeated call is not slowed by a
//! microsecond. That is what makes the guard removable: turn it off and the
//! behaviour left behind is today's, exactly.
//!
//! Three properties carry the whole value and each of them is a thing a naive
//! implementation gets wrong:
//!
//! - **Untracked calls are transparent.** With `TodoWrite` excluded,
//!   `Grep X -> TodoWrite -> Grep X` is still a run of two `Grep X`. A
//!   bookkeeping tool interleaved into a loop must not launder it, so an
//!   excluded tool neither increments the run nor resets it.
//! - **Refused calls count.** A model hammering a call that permissions keep
//!   rejecting is the loop most worth breaking, and the refusal is the reason
//!   its result never changes — so the advisory says so.
//! - **Keying is per agent, not per session.** `ToolContext::session_id` is
//!   copied verbatim from parent to child, so on its own it cannot separate a
//!   subagent from its parent. [`ChainKey`] pairs it with `subagent_id`, the
//!   same distinction the read-before-write registry draws for the same reason.
//!
//! In memory only, and dropped when the process ends. A resumed session starts
//! with a fresh chain; this is a heuristic nudge and not a logged invariant, so
//! a few extra reminders after a resume are the accepted cost of never
//! resurrecting a run that no longer describes anything.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};

use crate::tool::ToolContext;

/// Run lengths at which an advisory is produced, absent configuration.
const DEFAULT_THRESHOLDS: [u32; 3] = [3, 5, 8];
/// Tools that are transparent to the chain, absent configuration.
const DEFAULT_EXCLUDE: [&str; 1] = ["TodoWrite"];
/// How much of the repeated argument string a reminder quotes, absent
/// configuration.
const DEFAULT_PREVIEW_CHARS: usize = 500;

/// Agents whose chains are held at once.
///
/// Each entry holds the full canonical argument string of the last call, which
/// for a looping `Write` is the whole payload. One per agent is nothing; an
/// unbounded map of them across a long-lived process with `execute-plan`
/// spawning a subagent per task is not. The least recently touched entry is
/// evicted at the cap, which costs a missed reminder and no correctness.
const MAX_TRACKED_AGENTS: usize = 512;

/// Advisories held for an agent that has not drained them.
///
/// Both tool loops drain after every round, so the queue is normally empty or
/// holds one. A context that runs tools without draining — a test harness, a
/// bare registry dispatch — would otherwise accumulate forever.
const MAX_PENDING_REMINDERS: usize = 4;

/// `[guard.repeat_tool]` — when to tell the model it is repeating itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RepeatToolConfig {
    /// Count runs at all. `false` restores the behaviour that predates this
    /// module: nothing is observed, nothing is injected.
    pub enabled: bool,
    /// Run lengths that produce an advisory, ascending, each at least 2.
    ///
    /// The advisory escalates with position, so the order is meaning and not
    /// presentation: the first entry gets the short nudge and the last gets the
    /// blunt one.
    pub thresholds: Vec<u32>,
    /// Tools that are transparent to the chain — they neither extend a run nor
    /// break one.
    pub exclude: Vec<String>,
    /// How many characters of the repeated arguments a reminder quotes.
    ///
    /// Bounds the *reminder* only. The chain always compares the full canonical
    /// argument string, so truncating here cannot make two different calls look
    /// identical — and a looping `Write` cannot ride its whole payload into the
    /// next request.
    pub arguments_preview_chars: usize,
}

impl Default for RepeatToolConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            thresholds: DEFAULT_THRESHOLDS.to_vec(),
            exclude: DEFAULT_EXCLUDE
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
            arguments_preview_chars: DEFAULT_PREVIEW_CHARS,
        }
    }
}

impl RepeatToolConfig {
    /// Reject a threshold list that cannot mean what it says.
    ///
    /// Loudly, at config load, rather than by falling back to the defaults. A
    /// silent fallback would leave the operator reading their own `thresholds =
    /// []` in the file while the guard ran on `[3, 5, 8]`, and the first way
    /// they would learn otherwise is a reminder they configured away.
    ///
    /// Checked even when `enabled = false`: a list that is wrong is wrong now,
    /// and finding out only on the day the guard is switched on is finding out
    /// late.
    pub fn validate(&self) -> Result<(), String> {
        if self.thresholds.is_empty() {
            return Err(
                "guard.repeat_tool.thresholds must name at least one run length; \
                 omit the key to take the defaults, or set enabled = false to turn the guard off"
                    .to_string(),
            );
        }
        if self.arguments_preview_chars == 0 {
            return Err(
                "guard.repeat_tool.arguments_preview_chars must be at least 1; \
                 0 would quote nothing and leave the reminder unable to name what is being repeated"
                    .to_string(),
            );
        }
        let mut previous: Option<u32> = None;
        for &threshold in &self.thresholds {
            if threshold < 2 {
                return Err(format!(
                    "guard.repeat_tool.thresholds must each be at least 2 — \
                     a run of one call is the first call, not a repeat — got {threshold}"
                ));
            }
            match previous {
                Some(prev) if prev == threshold => {
                    return Err(format!(
                        "guard.repeat_tool.thresholds must not repeat a value, got {threshold} twice"
                    ));
                }
                Some(prev) if prev > threshold => {
                    return Err(format!(
                        "guard.repeat_tool.thresholds must ascend — the advisory escalates with \
                         position — got {threshold} after {prev}"
                    ));
                }
                _ => {}
            }
            previous = Some(threshold);
        }
        Ok(())
    }

    fn excludes(&self, tool_name: &str) -> bool {
        self.exclude.iter().any(|name| name == tool_name)
    }
}

/// Which agent a run belongs to.
///
/// `session_id` alone will not do. It is copied verbatim from parent to child,
/// so keying on it would let a subagent's repetition trip its parent's counter
/// and vice versa, and a lead coordinating five children would see their runs
/// interleaved into one meaningless chain. `subagent_id` is `None` for the
/// top-level agent, which is an answer and not missing data.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChainKey {
    pub session_id: String,
    pub subagent_id: Option<String>,
}

impl ChainKey {
    #[must_use]
    pub fn new(session_id: &str, subagent_id: Option<&str>) -> Self {
        Self {
            session_id: session_id.to_string(),
            subagent_id: subagent_id.map(str::to_string),
        }
    }

    /// The agent this tool invocation belongs to.
    #[must_use]
    pub fn of(ctx: &ToolContext) -> Self {
        Self::new(&ctx.session_id, ctx.subagent_id.as_deref())
    }
}

/// One agent's current run.
#[derive(Debug, Default)]
struct Chain {
    tool: String,
    /// Full canonical arguments of the run's calls — never the preview.
    arguments: String,
    run: u32,
    refused_in_run: u32,
    /// How many thresholds this run has already fired, which is also the
    /// escalation level of the next one.
    fired: usize,
    pending: Vec<String>,
    touched: u64,
}

/// Process-global chains, one per agent.
#[derive(Default)]
pub struct RepeatToolChains {
    chains: Mutex<HashMap<ChainKey, Chain>>,
    clock: AtomicU64,
}

/// The registry both tool loops observe into and drain from.
pub static REPEAT_TOOL_CHAINS: LazyLock<RepeatToolChains> =
    LazyLock::new(RepeatToolChains::default);

impl RepeatToolChains {
    /// Record one tool attempt, whether or not it was allowed to run.
    ///
    /// `refused` is true for an attempt that never reached the tool — admission
    /// blocked it, or the sandbox precheck did. It still extends the run,
    /// because a call that keeps being refused is the clearest loop there is,
    /// and it is carried into the advisory so the model is told *why* the
    /// result is not changing.
    pub fn observe(
        &self,
        key: &ChainKey,
        config: &RepeatToolConfig,
        tool_name: &str,
        input: &serde_json::Value,
        refused: bool,
    ) {
        if !config.enabled || config.thresholds.is_empty() || config.excludes(tool_name) {
            return;
        }
        let arguments = canonical_arguments(input);
        let tick = self.clock.fetch_add(1, Ordering::Relaxed);
        let Ok(mut chains) = self.chains.lock() else {
            return;
        };
        evict_for(&mut chains, key);
        let chain = chains.entry(key.clone()).or_default();
        chain.touched = tick;
        if chain.run > 0 && chain.tool == tool_name && chain.arguments == arguments {
            chain.run += 1;
        } else {
            chain.tool = tool_name.to_string();
            chain.arguments = arguments;
            chain.run = 1;
            chain.refused_in_run = 0;
            chain.fired = 0;
            // `pending` deliberately survives: an advisory earned by the run
            // that just ended is still owed to the model, and dropping it here
            // would lose exactly the reminder that made the model change tool.
        }
        if refused {
            chain.refused_in_run += 1;
        }
        let Some(&threshold) = config.thresholds.get(chain.fired) else {
            return;
        };
        if chain.run < threshold {
            return;
        }
        let level = chain.fired;
        chain.fired += 1;
        let reminder = reminder_text(config, chain, level);
        if chain.pending.len() >= MAX_PENDING_REMINDERS {
            chain.pending.remove(0);
        }
        chain.pending.push(reminder);
    }

    /// Take everything owed to this agent, leaving nothing behind.
    #[must_use]
    pub fn take_reminders(&self, key: &ChainKey) -> Vec<String> {
        let Ok(mut chains) = self.chains.lock() else {
            return Vec::new();
        };
        chains
            .get_mut(key)
            .map(|chain| std::mem::take(&mut chain.pending))
            .unwrap_or_default()
    }

    /// Current run length for an agent. For tests and diagnostics.
    #[must_use]
    pub fn run_length(&self, key: &ChainKey) -> u32 {
        self.chains
            .lock()
            .ok()
            .and_then(|chains| chains.get(key).map(|chain| chain.run))
            .unwrap_or(0)
    }
}

/// Make room for a new agent by dropping the least recently touched one.
fn evict_for(chains: &mut HashMap<ChainKey, Chain>, incoming: &ChainKey) {
    if chains.len() < MAX_TRACKED_AGENTS || chains.contains_key(incoming) {
        return;
    }
    let oldest = chains
        .iter()
        .min_by_key(|(_, chain)| chain.touched)
        .map(|(key, _)| key.clone());
    if let Some(key) = oldest {
        chains.remove(&key);
    }
}

/// The comparison key for one call's arguments.
///
/// Object keys are sorted at every depth so `{a:1,b:2}` and `{b:2,a:1}` are one
/// call repeated rather than two different ones.
///
/// The sort is not redundant even though `serde_json` is currently built
/// *without* `preserve_order` in this workspace's normal dependency graph — a
/// `Value::Object` is a `BTreeMap` today and `to_string` already emits sorted
/// keys. `preserve_order` is a feature of a transitive dependency, additive
/// across the crate graph, and any crate anyone adds may switch it on. Under
/// that flip a comparison relying on the map's own ordering starts missing
/// loops, and it does so silently: there is no failure, only reminders that
/// stop appearing. Twelve lines buy independence from a flag nobody in this
/// repo controls.
fn canonical_arguments(input: &serde_json::Value) -> String {
    canonicalise(input).to_string()
}

fn canonicalise(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable();
            let mut sorted = serde_json::Map::with_capacity(keys.len());
            for key in keys {
                sorted.insert(key.clone(), canonicalise(&map[key]));
            }
            serde_json::Value::Object(sorted)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonicalise).collect())
        }
        other => other.clone(),
    }
}

/// The arguments as the reminder quotes them.
fn preview(arguments: &str, max_chars: usize) -> String {
    let total = arguments.chars().count();
    if total <= max_chars {
        return arguments.to_string();
    }
    let kept: String = arguments.chars().take(max_chars).collect();
    let omitted = total - max_chars;
    format!("{kept}... ({omitted} more characters, not shown)")
}

/// The advisory for one crossed threshold.
///
/// Addressed to the model in the second person, because it arrives as a user
/// turn after the round's tool results and has to read like something worth
/// acting on rather than a log line. It escalates: a nudge, then instructions,
/// then a refusal to keep pretending the loop is going anywhere.
fn reminder_text(config: &RepeatToolConfig, chain: &Chain, level: usize) -> String {
    let arguments = preview(&chain.arguments, config.arguments_preview_chars);
    let refusals = if chain.refused_in_run == chain.run {
        format!(
            "\nEvery one of those {} calls was refused before it ran, so none of them produced \
             a result at all. Repeating the call cannot change that.\n",
            chain.run
        )
    } else if chain.refused_in_run > 0 {
        format!(
            "\n{} of those {} calls were refused before they ran.\n",
            chain.refused_in_run, chain.run
        )
    } else {
        String::new()
    };
    let advice = match level {
        0 => {
            "Re-read the result of the last one before calling it again — an identical call \
              returns an identical result. If it did not answer the question, change something \
              material: different arguments, a different tool, or a different question."
        }
        1 => {
            "An identical call cannot produce a different result, so this is not going to \
              resolve itself. Do this instead:\n\
              1. Re-read the last result of this call. The answer is either in it or not there \
              at all.\n\
              2. If it is not there, change the approach — different arguments, a different \
              tool, or a different question.\n\
              3. If you cannot make progress, say what you established and what you could not, \
              and stop."
        }
        _ => {
            "This is a loop and it will not break itself. Do not call this tool with these \
              arguments again. Either change the approach outright, or end here and state \
              plainly what you found and what you were unable to determine."
        }
    };
    format!(
        "[repeat-tool guard] You have called {} {} times in a row with identical arguments:\n\n\
         {}\n{}\n{}",
        chain.tool, chain.run, arguments, refusals, advice
    )
}

#[cfg(test)]
#[path = "repeat_tool_guard_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "repeat_tool_guard_config_tests.rs"]
mod config_tests;
