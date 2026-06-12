//! TASK-WC-004 unit tests (child module via #[path]; file-size guard).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use super::*;
use crate::write_coordinator::write_plan::{NormalizedPath, ResourceKey, TargetFilesSource};
use crate::write_coordinator::ItemId;

/// Minimal WritePlan with explicit resource keys + provenance.
fn plan(id: &str, keys: &[ResourceKey], source: TargetFilesSource) -> WritePlan {
    WritePlan {
        run_id: "run".into(),
        stage_id: "impl".into(),
        item_id: ItemId::from(id),
        canonical_root: PathBuf::from("/repo"),
        isolated_root: PathBuf::from("/repo/.archon/wc").join(id),
        target_files: vec![NormalizedPathStub::new(id)],
        target_files_source: source,
        read_context_files: vec![],
        verify_inputs: vec![],
        baseline_id: "git:HEAD".into(),
        workspace_boundary_required: true,
        resource_keys: keys.iter().cloned().collect(),
    }
}

/// We cannot construct NormalizedPath directly (private field), so derive one
/// through the public normalizer against a tempdir is overkill for these pure
/// scheduler tests. Instead use a stable in-repo relative path via the public
/// API surface: NormalizedPath is only needed non-empty here, so build it from
/// a path we know normalizes cleanly.
struct NormalizedPathStub;
impl NormalizedPathStub {
    fn new(id: &str) -> NormalizedPath {
        let dir = std::env::temp_dir().join(format!("cg-{}-{}", std::process::id(), id));
        std::fs::create_dir_all(&dir).expect("tempdir");
        crate::write_coordinator::write_plan::normalize_target("f.rs", &dir)
            .expect("normalize")
    }
}

fn file_key(p: &str) -> ResourceKey {
    ResourceKey::File(p.to_string())
}
fn dir_key(p: &str) -> ResourceKey {
    ResourceKey::Dir(p.to_string())
}

fn no_deps() -> BTreeMap<ItemId, BTreeSet<ItemId>> {
    BTreeMap::new()
}

fn caps(width: usize) -> WaveCaps {
    WaveCaps {
        run: width,
        policy: width,
        stage: None,
        runner: None,
        subagent: None,
    }
}

#[test]
fn two_disjoint_items_one_wave() {
    let plans = vec![
        plan("a", &[file_key("src/a.rs")], TargetFilesSource::Item),
        plan("b", &[file_key("src/b.rs")], TargetFilesSource::Item),
    ];
    let s = build_schedule("impl", &plans, &no_deps(), &caps(2)).expect("schedule");
    assert_eq!(s.waves.len(), 1);
    assert_eq!(s.waves[0].items, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn two_overlapping_items_two_waves() {
    let plans = vec![
        plan("a", &[file_key("src/lib.rs")], TargetFilesSource::Item),
        plan("b", &[file_key("src/lib.rs")], TargetFilesSource::Item),
    ];
    let s = build_schedule("impl", &plans, &no_deps(), &caps(4)).expect("schedule");
    assert_eq!(s.waves.len(), 2);
    assert_eq!(s.waves[0].items, vec!["a".to_string()]);
    assert_eq!(s.waves[1].items, vec!["b".to_string()]);
}

#[test]
fn file_under_dir_serializes() {
    let plans = vec![
        plan("a", &[file_key("a/b.rs")], TargetFilesSource::Item),
        plan("b", &[dir_key("a")], TargetFilesSource::Item),
    ];
    let s = build_schedule("impl", &plans, &no_deps(), &caps(4)).expect("schedule");
    assert_eq!(s.waves.len(), 2);
}

#[test]
fn three_items_a_c_share_dir_b_disjoint() {
    let plans = vec![
        plan("a", &[dir_key("shared")], TargetFilesSource::Item),
        plan("b", &[file_key("other/b.rs")], TargetFilesSource::Item),
        plan("c", &[dir_key("shared")], TargetFilesSource::Item),
    ];
    let s = build_schedule("impl", &plans, &no_deps(), &caps(4)).expect("schedule");
    assert_eq!(s.waves.len(), 2);
    assert_eq!(s.waves[0].items, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(s.waves[1].items, vec!["c".to_string()]);
}

#[test]
fn dependency_chain_three_sequential_waves() {
    let plans = vec![
        plan("a", &[file_key("a.rs")], TargetFilesSource::Item),
        plan("b", &[file_key("b.rs")], TargetFilesSource::Item),
        plan("c", &[file_key("c.rs")], TargetFilesSource::Item),
    ];
    let mut deps = BTreeMap::new();
    deps.insert("b".to_string(), BTreeSet::from(["a".to_string()]));
    deps.insert("c".to_string(), BTreeSet::from(["b".to_string()]));
    let s = build_schedule("impl", &plans, &deps, &caps(4)).expect("schedule");
    assert_eq!(s.waves.len(), 3);
    assert_eq!(s.waves[0].items, vec!["a".to_string()]);
    assert_eq!(s.waves[1].items, vec!["b".to_string()]);
    assert_eq!(s.waves[2].items, vec!["c".to_string()]);
}

#[test]
fn dependency_respected_despite_disjoint_keys() {
    let plans = vec![
        plan("a", &[file_key("a.rs")], TargetFilesSource::Item),
        plan("b", &[file_key("b.rs")], TargetFilesSource::Item),
    ];
    let mut deps = BTreeMap::new();
    deps.insert("b".to_string(), BTreeSet::from(["a".to_string()]));
    let s = build_schedule("impl", &plans, &deps, &caps(4)).expect("schedule");
    assert_eq!(s.waves.len(), 2);
    assert_eq!(s.waves[0].items, vec!["a".to_string()]);
    assert_eq!(s.waves[1].items, vec!["b".to_string()]);
}

#[test]
fn forward_reference_dependency_no_overflow() {
    // B declared FIRST, depends on A declared SECOND.
    let plans = vec![
        plan("b", &[file_key("b.rs")], TargetFilesSource::Item),
        plan("a", &[file_key("a.rs")], TargetFilesSource::Item),
    ];
    let mut deps = BTreeMap::new();
    deps.insert("b".to_string(), BTreeSet::from(["a".to_string()]));
    let s = build_schedule("impl", &plans, &deps, &caps(4)).expect("schedule");
    assert_eq!(s.waves.len(), 2);
    assert_eq!(s.waves[0].items, vec!["a".to_string()]);
    assert_eq!(s.waves[1].items, vec!["b".to_string()]);
}

#[test]
fn stage_level_items_serialize_even_when_keys_disjoint() {
    let plans = vec![
        plan("a", &[file_key("a.rs")], TargetFilesSource::StageLevel),
        plan("b", &[file_key("b.rs")], TargetFilesSource::StageLevel),
    ];
    let s = build_schedule("impl", &plans, &no_deps(), &caps(4)).expect("schedule");
    assert_eq!(s.waves.len(), 2, "StageLevel items must serialize per PRD 8.1");
}

#[test]
fn stage_level_edge_lives_in_graph_not_inline() {
    let plans = vec![
        plan("a", &[file_key("a.rs")], TargetFilesSource::StageLevel),
        plan("b", &[file_key("b.rs")], TargetFilesSource::StageLevel),
    ];
    let graph = build_conflict_graph(&plans);
    assert!(graph.adjacency.get("a").unwrap().contains("b"));
    assert!(graph.adjacency.get("b").unwrap().contains("a"));
}

#[test]
fn cycle_detected() {
    let plans = vec![
        plan("a", &[file_key("a.rs")], TargetFilesSource::Item),
        plan("b", &[file_key("b.rs")], TargetFilesSource::Item),
    ];
    let mut deps = BTreeMap::new();
    deps.insert("a".to_string(), BTreeSet::from(["b".to_string()]));
    deps.insert("b".to_string(), BTreeSet::from(["a".to_string()]));
    match build_schedule("impl", &plans, &deps, &caps(4)) {
        Err(ScheduleError::CyclicDependency { items }) => {
            assert!(items.contains(&"a".to_string()));
            assert!(items.contains(&"b".to_string()));
        }
        other => panic!("expected CyclicDependency, got {other:?}"),
    }
}

#[test]
fn unknown_dependency_rejected() {
    let plans = vec![plan("a", &[file_key("a.rs")], TargetFilesSource::Item)];
    let mut deps = BTreeMap::new();
    deps.insert("a".to_string(), BTreeSet::from(["ghost".to_string()]));
    match build_schedule("impl", &plans, &deps, &caps(4)) {
        Err(ScheduleError::UnknownDependency { item, missing }) => {
            assert_eq!(item, "a");
            assert_eq!(missing, "ghost");
        }
        other => panic!("expected UnknownDependency, got {other:?}"),
    }
}

#[test]
fn empty_plans_rejected() {
    let plans: Vec<WritePlan> = vec![];
    assert_eq!(
        build_schedule("impl", &plans, &no_deps(), &caps(4)),
        Err(ScheduleError::EmptyPlans)
    );
}

#[test]
fn missing_targets_rejected() {
    let mut p = plan("a", &[file_key("a.rs")], TargetFilesSource::Item);
    p.target_files.clear();
    match build_schedule("impl", &[p], &no_deps(), &caps(4)) {
        Err(ScheduleError::MissingTargets { item }) => assert_eq!(item, "a"),
        other => panic!("expected MissingTargets, got {other:?}"),
    }
}

#[test]
fn cap_one_gives_each_item_its_own_wave() {
    let plans = vec![
        plan("a", &[file_key("a.rs")], TargetFilesSource::Item),
        plan("b", &[file_key("b.rs")], TargetFilesSource::Item),
        plan("c", &[file_key("c.rs")], TargetFilesSource::Item),
    ];
    let s = build_schedule("impl", &plans, &no_deps(), &caps(1)).expect("schedule");
    assert_eq!(s.waves.len(), 3);
    assert!(s.waves.iter().all(|w| w.items.len() == 1));
}

#[test]
fn stage_id_is_explicit_input() {
    let plans = vec![plan("a", &[file_key("a.rs")], TargetFilesSource::Item)];
    let s = build_schedule("impl_stage", &plans, &no_deps(), &caps(4)).expect("schedule");
    assert_eq!(s.stage_id, "impl_stage");
}

#[test]
fn schedule_is_deterministic_across_runs() {
    let plans = vec![
        plan("a", &[dir_key("shared")], TargetFilesSource::Item),
        plan("b", &[file_key("other/b.rs")], TargetFilesSource::Item),
        plan("c", &[dir_key("shared")], TargetFilesSource::Item),
    ];
    let first = build_schedule("impl", &plans, &no_deps(), &caps(4)).expect("schedule");
    for _ in 0..10 {
        let again = build_schedule("impl", &plans, &no_deps(), &caps(4)).expect("schedule");
        assert_eq!(first, again);
    }
}

#[test]
fn wave_caps_mixed_source_floor() {
    let caps = WaveCaps::from_sources(8, 4, Some(2), Some(1), None);
    assert_eq!(caps.effective(), 1);
}

#[test]
fn wave_caps_floor_at_one_when_all_zero() {
    let caps = WaveCaps::from_sources(0, 0, None, None, None);
    assert_eq!(caps.effective(), 1);
}

#[test]
fn schedule_summary_matches_waves() {
    let plans = vec![
        plan("a", &[dir_key("shared")], TargetFilesSource::Item),
        plan("b", &[file_key("other/b.rs")], TargetFilesSource::Item),
        plan("c", &[dir_key("shared")], TargetFilesSource::Item),
    ];
    let s = build_schedule("impl", &plans, &no_deps(), &caps(4)).expect("schedule");
    let summary = schedule_summary(&s);
    assert_eq!(summary.wave_count, s.waves.len());
    assert_eq!(
        summary.max_width,
        s.waves.iter().map(|w| w.items.len()).max().unwrap()
    );
    assert_eq!(summary.total_items, 3);
    let largest = s
        .waves
        .iter()
        .find(|w| w.items.len() == summary.max_width)
        .unwrap();
    assert_eq!(summary.items_in_largest_wave, largest.items);
    assert!(summary.max_width >= 1);
    assert!(summary.max_width <= caps(4).effective());
}
