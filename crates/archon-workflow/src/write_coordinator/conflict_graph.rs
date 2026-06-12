//! TASK-WC-004 — Conflict graph + deterministic safe-wave scheduler (PRD-012 §10).
//!
//! Given WritePlan items in declared order, produce ordered waves where no wave
//! has resource-key conflicts and item-level dependencies land in earlier waves.
//! All ordering is via BTreeMap/BTreeSet for byte-identical determinism.

use std::collections::{BTreeMap, BTreeSet};

use super::write_plan::{TargetFilesSource, WritePlan, keys_conflict};
use super::{ItemId, WaveId};

/// Floor of every concurrency cap that bounds a wave's width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaveCaps {
    pub run: usize,
    pub policy: usize,
    pub stage: Option<usize>,
    pub runner: Option<usize>,
    pub subagent: Option<usize>,
}

impl WaveCaps {
    /// Effective max wave width: floor of all caps, never below 1.
    pub fn effective(&self) -> usize {
        let mut m = self.run.min(self.policy);
        if let Some(s) = self.stage {
            m = m.min(s);
        }
        if let Some(r) = self.runner {
            m = m.min(r);
        }
        if let Some(sa) = self.subagent {
            m = m.min(sa);
        }
        m.max(1)
    }

    pub fn from_sources(
        run_u32: u32,
        policy_u32: u32,
        stage_u32: Option<u32>,
        runner: Option<usize>,
        subagent: Option<usize>,
    ) -> Self {
        Self {
            run: run_u32 as usize,
            policy: policy_u32 as usize,
            stage: stage_u32.map(|v| v as usize),
            runner,
            subagent,
        }
    }
}

/// One scheduled wave of non-conflicting items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wave {
    pub wave_id: WaveId,
    pub items: Vec<ItemId>,
}

/// Ordered waves for one implementation fanout stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    pub stage_id: String,
    pub waves: Vec<Wave>,
}

/// Status-reporting summary (consumed by TASK-WC-008).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleSummary {
    pub wave_count: usize,
    pub max_width: usize,
    pub total_items: usize,
    pub items_in_largest_wave: Vec<ItemId>,
}

/// Adjacency: item -> set of items it conflicts with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConflictGraph {
    pub adjacency: BTreeMap<ItemId, BTreeSet<ItemId>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleError {
    CyclicDependency { items: Vec<ItemId> },
    UnknownDependency { item: ItemId, missing: ItemId },
    MissingTargets { item: ItemId },
    EmptyPlans,
}

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CyclicDependency { items } => {
                write!(f, "cyclic item dependency: {items:?}")
            }
            Self::UnknownDependency { item, missing } => {
                write!(f, "item '{item}' depends on unknown item '{missing}'")
            }
            Self::MissingTargets { item } => {
                write!(f, "item '{item}' declares no target files")
            }
            Self::EmptyPlans => write!(f, "no plans to schedule"),
        }
    }
}

impl std::error::Error for ScheduleError {}

/// Build the conflict graph: edge iff resource keys overlap OR both items draw
/// targets from the shared stage-level fallback (PRD §8.1). SOLE adjacency source.
pub fn build_conflict_graph(plans: &[WritePlan]) -> ConflictGraph {
    let mut adjacency: BTreeMap<ItemId, BTreeSet<ItemId>> = BTreeMap::new();
    for plan in plans {
        adjacency.entry(plan.item_id.clone()).or_default();
    }
    for (i, left) in plans.iter().enumerate() {
        for right in &plans[i + 1..] {
            if items_conflict(left, right) {
                adjacency
                    .entry(left.item_id.clone())
                    .or_default()
                    .insert(right.item_id.clone());
                adjacency
                    .entry(right.item_id.clone())
                    .or_default()
                    .insert(left.item_id.clone());
            }
        }
    }
    ConflictGraph { adjacency }
}

fn items_conflict(left: &WritePlan, right: &WritePlan) -> bool {
    if left.target_files_source == TargetFilesSource::StageLevel
        && right.target_files_source == TargetFilesSource::StageLevel
    {
        return true;
    }
    left.resource_keys
        .iter()
        .any(|a| right.resource_keys.iter().any(|b| keys_conflict(a, b)))
}

/// Schedule items into ordered, conflict-free, dependency-respecting waves.
pub fn build_schedule(
    stage_id: &str,
    plans: &[WritePlan],
    deps: &BTreeMap<ItemId, BTreeSet<ItemId>>,
    caps: &WaveCaps,
) -> Result<Schedule, ScheduleError> {
    if plans.is_empty() {
        return Err(ScheduleError::EmptyPlans);
    }
    for plan in plans {
        if plan.target_files.is_empty() {
            return Err(ScheduleError::MissingTargets {
                item: plan.item_id.clone(),
            });
        }
    }
    let known: BTreeSet<&str> = plans.iter().map(|p| p.item_id.as_str()).collect();
    for plan in plans {
        if let Some(targets) = deps.get(&plan.item_id) {
            for target in targets {
                if !known.contains(target.as_str()) {
                    return Err(ScheduleError::UnknownDependency {
                        item: plan.item_id.clone(),
                        missing: target.clone(),
                    });
                }
            }
        }
    }
    let ordered = kahn_topo_sort(plans, deps)?;
    let graph = build_conflict_graph(&ordered);
    Ok(place_waves(stage_id, &ordered, deps, &graph, caps.effective()))
}

/// Place items into the earliest legal wave (PRD §10.2). Pure adjacency lookup.
fn place_waves(
    stage_id: &str,
    ordered: &[WritePlan],
    deps: &BTreeMap<ItemId, BTreeSet<ItemId>>,
    graph: &ConflictGraph,
    max_width: usize,
) -> Schedule {
    let mut waves: Vec<Wave> = Vec::new();
    let mut placement: BTreeMap<ItemId, usize> = BTreeMap::new();
    for plan in ordered {
        let dep_floor = deps
            .get(&plan.item_id)
            .and_then(|ds| ds.iter().filter_map(|d| placement.get(d)).copied().max())
            .map(|w| w + 1)
            .unwrap_or(0);
        let neighbors = graph.adjacency.get(&plan.item_id).cloned().unwrap_or_default();
        let mut target = dep_floor;
        loop {
            if target == waves.len() {
                let wave_id = u32::try_from(target).expect("wave count fits in u32");
                waves.push(Wave {
                    wave_id,
                    items: Vec::new(),
                });
            }
            let wave = &waves[target];
            let conflicts = wave.items.iter().any(|other| neighbors.contains(other));
            if !conflicts && wave.items.len() < max_width {
                waves[target].items.push(plan.item_id.clone());
                placement.insert(plan.item_id.clone(), target);
                break;
            }
            target += 1;
        }
    }
    Schedule {
        stage_id: stage_id.to_string(),
        waves,
    }
}

/// Summarize a schedule for status reporting.
pub fn schedule_summary(schedule: &Schedule) -> ScheduleSummary {
    let max_width = schedule
        .waves
        .iter()
        .map(|w| w.items.len())
        .max()
        .unwrap_or(0);
    let items_in_largest_wave = schedule
        .waves
        .iter()
        .find(|w| w.items.len() == max_width)
        .map(|w| w.items.clone())
        .unwrap_or_default();
    ScheduleSummary {
        wave_count: schedule.waves.len(),
        max_width,
        total_items: schedule.waves.iter().map(|w| w.items.len()).sum(),
        items_in_largest_wave,
    }
}

/// Topological order with the original declared order as a stable tiebreak,
/// so independent items keep their declared sequence. Detects cycles via
/// WHITE/GRAY/BLACK DFS coloring before returning.
fn kahn_topo_sort(
    plans: &[WritePlan],
    deps: &BTreeMap<ItemId, BTreeSet<ItemId>>,
) -> Result<Vec<WritePlan>, ScheduleError> {
    detect_cycle(plans, deps)?;
    let mut indegree: BTreeMap<&str, usize> = plans
        .iter()
        .map(|p| (p.item_id.as_str(), deps.get(&p.item_id).map_or(0, BTreeSet::len)))
        .collect();
    let mut placed: BTreeSet<&str> = BTreeSet::new();
    let mut result = Vec::with_capacity(plans.len());
    // Pick the earliest-declared item whose deps are all placed, repeat.
    while result.len() < plans.len() {
        let Some(next) = plans
            .iter()
            .find(|p| !placed.contains(p.item_id.as_str()) && indegree[p.item_id.as_str()] == 0)
        else {
            // Unreachable: detect_cycle already proved a valid order exists.
            return Err(ScheduleError::CyclicDependency {
                items: plans
                    .iter()
                    .filter(|p| !placed.contains(p.item_id.as_str()))
                    .map(|p| p.item_id.clone())
                    .collect(),
            });
        };
        placed.insert(next.item_id.as_str());
        result.push(next.clone());
        for plan in plans {
            if deps.get(&plan.item_id).is_some_and(|t| t.contains(&next.item_id)) {
                *indegree.get_mut(plan.item_id.as_str()).unwrap() -= 1;
            }
        }
    }
    Ok(result)
}

fn detect_cycle(
    plans: &[WritePlan],
    deps: &BTreeMap<ItemId, BTreeSet<ItemId>>,
) -> Result<(), ScheduleError> {
    let mut color: BTreeMap<&str, Color> = plans
        .iter()
        .map(|p| (p.item_id.as_str(), Color::White))
        .collect();
    let mut stack = Vec::new();
    for plan in plans {
        if color[plan.item_id.as_str()] == Color::White
            && let Some(cycle) = visit(plan.item_id.as_str(), deps, &mut color, &mut stack)
        {
            return Err(ScheduleError::CyclicDependency { items: cycle });
        }
    }
    Ok(())
}

fn visit<'a>(
    node: &'a str,
    deps: &'a BTreeMap<ItemId, BTreeSet<ItemId>>,
    color: &mut BTreeMap<&'a str, Color>,
    stack: &mut Vec<&'a str>,
) -> Option<Vec<ItemId>> {
    color.insert(node, Color::Gray);
    stack.push(node);
    if let Some(targets) = deps.get(node) {
        for target in targets {
            match color.get(target.as_str()).copied() {
                Some(Color::Gray) => {
                    return Some(stack.iter().map(|s| s.to_string()).collect());
                }
                Some(Color::White) | None => {
                    if let Some(cycle) = visit(target.as_str(), deps, color, stack) {
                        return Some(cycle);
                    }
                }
                Some(Color::Black) => {}
            }
        }
    }
    stack.pop();
    color.insert(node, Color::Black);
    None
}

#[derive(Clone, Copy, PartialEq)]
enum Color {
    White,
    Gray,
    Black,
}

#[cfg(test)]
#[path = "conflict_graph_tests.rs"]
mod tests;
