//! Dependency waves, computed from the task universe the host already parsed.
//!
//! The authoring agent used to be told to "decide sequential vs parallel FROM
//! THE TASK DATA", which meant re-deriving a dependency graph by reading every
//! task file — work the host had already done exactly. Observed live: the
//! authored script emitted 60 sequential `agent()` calls and zero batches,
//! including four provider-ingest tasks that share no dependency and write to
//! four separate directories. A model with no reliable basis for the judgement
//! defaults to the safe answer, and the safe answer costs hours.
//!
//! So the host computes it instead. `dependency_ids` gives the edges,
//! `files_expected_to_change` and `shared_append_target_files` give the write
//! conflicts, and both are already parsed. The result is stated in the brief as
//! fact rather than left as an exercise.

use crate::task_universe::WorkflowV2TaskUniverse;
use std::collections::{BTreeMap, BTreeSet};

/// One batch of tasks that may run concurrently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorWaveGroup {
    pub wave: usize,
    pub task_ids: Vec<String>,
}

/// Group every task into dependency waves, splitting each wave further so that
/// no group holds two tasks whose declared writes overlap.
///
/// A task lands in the wave after the latest of its dependencies. Tasks whose
/// dependencies fall outside the universe are treated as satisfied — the
/// universe is the authority on what exists, and an unresolvable id is a
/// reconciliation problem, not a scheduling one.
pub fn author_wave_groups(universe: &WorkflowV2TaskUniverse) -> Vec<AuthorWaveGroup> {
    let known: BTreeSet<&str> = universe
        .tasks
        .iter()
        .map(|task| task.canonical_task_id.as_str())
        .collect();

    let mut depth: BTreeMap<&str, usize> = BTreeMap::new();
    // Repeated relaxation rather than a topological sort: a cycle would make a
    // sort fail outright, and refusing to schedule is worse than scheduling a
    // cycle's members in the same wave and letting the dependency gates catch
    // it. Bounded by the task count, so a cycle cannot spin here.
    for _ in 0..universe.tasks.len().max(1) {
        let mut changed = false;
        for task in &universe.tasks {
            let want = task
                .dependency_ids
                .iter()
                .filter(|id| known.contains(id.as_str()))
                .map(|id| depth.get(id.as_str()).copied().unwrap_or(0) + 1)
                .max()
                .unwrap_or(0);
            let slot = depth.entry(task.canonical_task_id.as_str()).or_insert(0);
            if want > *slot {
                *slot = want;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let mut by_wave: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    for task in &universe.tasks {
        let wave = depth
            .get(task.canonical_task_id.as_str())
            .copied()
            .unwrap_or(0);
        by_wave
            .entry(wave)
            .or_default()
            .push(&task.canonical_task_id);
    }

    let mut groups = Vec::new();
    for (wave, ids) in by_wave {
        for group in split_by_write_conflict(universe, &ids) {
            groups.push(AuthorWaveGroup {
                wave,
                task_ids: group,
            });
        }
    }
    groups
}

/// Split one wave into groups no two of which write the same file.
///
/// First-fit rather than optimal packing: the goal is to stop obviously
/// independent work from serialising, not to find the minimum number of
/// batches. A file claimed by two tasks keeps them in separate groups, which
/// preserves the "parallel writes are forbidden when targets overlap" rule the
/// dialect already states.
fn split_by_write_conflict(universe: &WorkflowV2TaskUniverse, ids: &[&str]) -> Vec<Vec<String>> {
    let mut groups: Vec<(BTreeSet<String>, Vec<String>)> = Vec::new();
    for id in ids {
        let writes = declared_writes(universe, id);
        match groups
            .iter_mut()
            .find(|(claimed, _)| writes.is_disjoint(claimed))
        {
            Some((claimed, members)) => {
                claimed.extend(writes);
                members.push((*id).to_string());
            }
            None => groups.push((writes, vec![(*id).to_string()])),
        }
    }
    groups.into_iter().map(|(_, members)| members).collect()
}

/// The paths a task claims EXCLUSIVELY, which are the only ones that can stop
/// two tasks batching.
///
/// `shared_append_target_files` is deliberately excluded. That field is the
/// declaration that a path is safe to write concurrently — `ResourceKey::
/// SharedAppend` exists so the write coordinator will *not* schedule around it
/// — so counting it as a conflict inverts its meaning. It cost the batch it
/// was supposed to allow: the four provider-ingest tasks each declare the two
/// module-declaration files there, because each appends its own `mod` line,
/// and treating that as exclusive kept all four running one at a time.
fn declared_writes(universe: &WorkflowV2TaskUniverse, task_id: &str) -> BTreeSet<String> {
    universe
        .tasks
        .iter()
        .find(|task| task.canonical_task_id == task_id)
        .map(|task| {
            let shared: BTreeSet<String> = task
                .shared_append_target_files
                .iter()
                .filter_map(|entry| declared_path(entry))
                .collect();
            task.files_expected_to_change
                .iter()
                .filter_map(|entry| declared_path(entry))
                .filter(|path| !shared.contains(path))
                .collect()
        })
        .unwrap_or_default()
}

/// The path out of one declared-files entry.
///
/// These entries are prose as often as they are paths: task files write them as
/// "`crates/a/b.rs` — only the module declaration and narrow exports", and the
/// parser keeps the bullet whole. Comparing those sentences for overlap finds
/// nothing, because two tasks touching one file describe it differently. Take
/// the backticked path when there is one, else the first token, and compare
/// those.
fn declared_path(entry: &str) -> Option<String> {
    let trimmed = entry.trim();
    if trimmed.is_empty() {
        return None;
    }
    let candidate = match trimmed.strip_prefix('`') {
        Some(rest) => rest.split('`').next().unwrap_or(rest),
        None => trimmed.split_whitespace().next().unwrap_or(trimmed),
    };
    let candidate = candidate.trim().trim_matches('`').trim();
    (!candidate.is_empty()).then(|| candidate.to_string())
}

/// The waves, rendered for the author brief.
pub fn render_author_waves(universe: &WorkflowV2TaskUniverse) -> String {
    let groups = author_wave_groups(universe);
    if groups.is_empty() {
        return "<none>".to_string();
    }
    groups
        .iter()
        .map(|group| {
            let shape = if group.task_ids.len() > 1 {
                "BATCH — one `await agents([...])` call"
            } else {
                "single agent() call"
            };
            format!(
                "- wave {}: {} ({shape})",
                group.wave + 1,
                group.task_ids.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The example wave literal used when there is no task universe to stamp from.
const PLACEHOLDER_EXAMPLE_WAVES: &str =
    "  const waves = [\n    ['TASK-X-001'],\n    ['TASK-X-002', 'TASK-X-003'],\n  ]";

/// The worked example's `const waves = [...]`, stamped with this run's real
/// wave groups.
///
/// The example used to carry `TASK-X-001` placeholders while the wave data
/// beside it carried the real ids, so the author had to bridge fiction to fact
/// — and a bridge is a judgement, which is where it wanders off. The host has
/// already computed these groups from the declared dependencies and target
/// files, so handing them to the example is determined data, not a
/// PRD-specific assumption. A universe with no tasks keeps the placeholders.
pub fn render_example_wave_literal(universe: &WorkflowV2TaskUniverse) -> String {
    let groups = author_wave_groups(universe);
    if groups.is_empty() {
        return PLACEHOLDER_EXAMPLE_WAVES.to_string();
    }
    let rows = groups
        .iter()
        .map(|group| {
            let ids = group
                .task_ids
                .iter()
                .map(|id| format!("'{id}'"))
                .collect::<Vec<_>>()
                .join(", ");
            let note = if group.task_ids.len() > 1 {
                format!("  // wave {}: independent, run TOGETHER", group.wave + 1)
            } else {
                format!("  // wave {}", group.wave + 1)
            };
            format!("    [{ids}],{note}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("  const waves = [\n{rows}\n  ]")
}

/// The dialect reference with its worked example stamped for this run.
///
/// The reference is handed to the brief as the value of `{reference}`, and
/// `compose_author_brief` never rescans substituted values (so run-derived text
/// cannot inject placeholders). The example therefore has to be filled in here,
/// before it is passed.
pub fn render_dialect_reference(universe: Option<&WorkflowV2TaskUniverse>) -> String {
    let example = match universe {
        Some(universe) => render_example_wave_literal(universe),
        None => PLACEHOLDER_EXAMPLE_WAVES.to_string(),
    };
    super::v3_author_a::V3_PRIMITIVE_REFERENCE.replace("{example_waves}", &example)
}

#[cfg(test)]
#[path = "v3_author_waves_tests.rs"]
mod tests;
