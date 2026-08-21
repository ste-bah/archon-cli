//! Serialize cargo-running branches WITHOUT serializing their wave.
//!
//! Concurrent `cargo` invocations in one checkout contend on the same
//! target-directory lock, so cargo-running branches must not overlap. The old
//! enforcement pinned the ENTIRE wave to `maxParallelism = 1` the moment any
//! one item mentioned a cargo command — observed live as a 27-branch
//! verification wave running strictly one branch at a time when most branches
//! were file inspections that never touch cargo. The configured parallelism
//! was never once reached.
//!
//! The scheduler already has the right instrument: per-role semaphores with
//! per-role limits. So cargo-running items are retagged into one scheduling
//! role capped at 1, and every other item keeps the wave's configured width.
//! The pin's guarantee is preserved exactly — no two cargo branches ever run
//! together — while the rest of the wave parallelizes.
//!
//! Matched on command text alone; carries no task, provider or PRD knowledge.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::generated_lifecycle_support::raw_strings;
use crate::v2::WorkflowV2FanoutItem;

/// The scheduling role cargo-running branches share.
///
/// Only a scheduling identity: the agent's behavioural role stays in the
/// branch call's options, which this retagging never touches.
pub const CARGO_SERIAL_ROLE: &str = "cargo-serial";

/// Rust build tools that take the shared target-directory lock. `rustc` and
/// `rustdoc` contend exactly as `cargo` does — they are what cargo runs.
const RUST_BUILD_TOOLS: [&str; 3] = ["cargo", "rustc", "rustdoc"];

/// Does this item's declared verification/evidence run rust build tooling?
pub fn item_has_cargo_commands(item: &Value) -> bool {
    raw_strings(
        item,
        &[
            "focused_verification",
            "commands",
            "command",
            "expected_evidence",
        ],
    )
    .iter()
    .any(|text| runs_rust_build_tool(text))
}

/// Match the tool as a command word rather than a substring.
///
/// The previous test was `contains("cargo ")`, which agents routinely walk
/// straight past: told the host owns the shared target directory, they
/// hand-roll the underlying compiler instead — `rustc --edition 2024 --extern
/// ...` — which takes the same lock, matches nothing, and is scheduled at full
/// wave width. Observed live: three such branches running concurrently for 21
/// to 37 minutes, contending on the very directory the role limit exists to
/// protect.
///
/// Word matching also tightens the old rule: prose naming the `cargo-serial`
/// role, or a path like `cargo-audit`, no longer counts as running cargo,
/// while `/usr/bin/rustc` and a bare trailing `cargo` now do.
fn runs_rust_build_tool(text: &str) -> bool {
    text.split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '(' | ')'))
        .filter_map(|token| {
            let token = token.trim_matches(|ch: char| matches!(ch, '"' | '\'' | ',' | '`'));
            token.rsplit('/').next()
        })
        .any(|name| {
            let name = name.trim_end_matches(".exe").to_ascii_lowercase();
            RUST_BUILD_TOOLS.contains(&name.as_str())
        })
}

/// Retag cargo-running fanout items into [`CARGO_SERIAL_ROLE`].
///
/// Checks both the branch input's `item` object (where fanout wraps the plan
/// item) and the input root (where bare items land), so the tag survives
/// either layout. The input JSON itself is deliberately untouched — item input
/// hashes drive branch-outcome reuse across resumes, and a stamp inside the
/// input would invalidate every cached verification outcome once.
pub fn tag_cargo_serial_roles(items: Vec<WorkflowV2FanoutItem>) -> Vec<WorkflowV2FanoutItem> {
    items
        .into_iter()
        .map(|mut item| {
            let in_item = item.input.get("item").is_some_and(item_has_cargo_commands);
            if in_item || item_has_cargo_commands(&item.input) {
                item.role = CARGO_SERIAL_ROLE.to_string();
            }
            item
        })
        .collect()
}

/// The role limits a read-only fanout scheduler should run with: cargo
/// branches strictly serial, everything else at the configured width.
pub fn cargo_serial_role_limits() -> BTreeMap<String, usize> {
    BTreeMap::from([(CARGO_SERIAL_ROLE.to_string(), 1)])
}

#[cfg(test)]
#[path = "cargo_serial_tests.rs"]
mod tests;
