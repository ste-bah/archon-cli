//! Two checks on the same generated plan:
//!
//! 1. Does the wave layering agree with what the PRD itself said the
//!    decomposition should look like (§15 order, §14 phase gates)?
//! 2. Do the four ways a task set can be malformed each fail the run, naming
//!    the file that caused it?
//!
//! Neither the PRD nor the generated plan is treated as authoritative over the
//! other. §15 says "recommended"; the plan is derived from what the task files
//! declare. Where they disagree the test reports both orderings and fails, so a
//! human decides which one is wrong — that is the only honest verdict a test
//! can reach about a disagreement between a document and a graph.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::Deserialize;

use super::{fixture_root, fixture_universe, plan_task_text, wave_index_by_task, wave_layering};
use crate::command::workflow_live::workflow_live_task_universe::extract_task_universe_for_generated_run;

#[derive(Debug, Deserialize)]
struct Guidance {
    decomposition_order: Vec<String>,
    tasks_absent_from_section_15: Vec<String>,
    phases: Vec<Phase>,
}

#[derive(Debug, Deserialize)]
struct Phase {
    phase: u32,
    scope: String,
    tasks: Vec<String>,
}

fn guidance() -> Guidance {
    let path = fixture_root().join("prd-decomposition-guidance.json");
    let raw = fs::read_to_string(&path).expect("read the transcribed §14/§15 guidance");
    serde_json::from_str(&raw).expect("guidance table parses")
}

/// §15 lists 15 tasks; the shipped decomposition has 17. That is a real
/// divergence between the PRD and what was authored from it, and it is pinned
/// here rather than papered over: if a third task ever appears with no §15
/// entry, this fails and someone has to say whether §15 or the task set is
/// stale.
#[test]
fn the_generated_task_set_diverges_from_section_15_only_by_the_recorded_two_tasks() {
    let guidance = guidance();
    let universe = fixture_universe();
    let generated = universe
        .tasks
        .iter()
        .map(|task| task.canonical_task_id.clone())
        .collect::<BTreeSet<_>>();
    let recommended = guidance
        .decomposition_order
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let recorded = guidance
        .tasks_absent_from_section_15
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();

    let extra = generated
        .difference(&recommended)
        .cloned()
        .collect::<BTreeSet<_>>();
    let missing = recommended
        .difference(&generated)
        .cloned()
        .collect::<Vec<_>>();

    assert!(
        missing.is_empty(),
        "§15 recommends task(s) with no TASK-*.md file: {missing:?}"
    );
    assert_eq!(
        extra,
        recorded,
        "the shipped decomposition has task(s) §15 never mentioned.\n  \
         §15 order ({}): {:?}\n  generated ({}): {:?}",
        guidance.decomposition_order.len(),
        guidance.decomposition_order,
        generated.len(),
        generated
    );
}

/// §15 states a single total order. The generated plan states a partial order
/// with parallel waves. The plan agrees with §15 exactly when its wave indices
/// are non-decreasing along §15's sequence — a refinement, not a contradiction.
/// Any pair that goes backwards is a genuine disagreement and is reported with
/// both orderings.
#[test]
fn generated_wave_order_refines_the_section_15_recommended_order() {
    let guidance = guidance();
    let universe = fixture_universe();
    let waves = wave_layering(&universe);
    let index = wave_index_by_task(&waves);

    let mut inversions = Vec::new();
    for pair in guidance.decomposition_order.windows(2) {
        let (earlier, later) = (&pair[0], &pair[1]);
        let (Some(&a), Some(&b)) = (index.get(earlier), index.get(later)) else {
            continue;
        };
        if a > b {
            inversions.push(format!(
                "§15 puts {earlier} before {later}; the plan puts {earlier} in wave {a} and {later} in wave {b}"
            ));
        }
    }
    assert!(
        inversions.is_empty(),
        "generated wave order contradicts §15.\n  §15 order: {:?}\n  generated waves: {}\n  {}",
        guidance.decomposition_order,
        render_waves(&waves),
        inversions.join("\n  ")
    );
}

/// §14's phases are gates, not labels: phase N+1's scope presupposes phase N's
/// exit gate. If the plan lets any phase-N+1 task start in a wave that a
/// phase-N task has not cleared, the encoded dependency graph has lost a gate
/// the PRD declared.
#[test]
fn generated_waves_respect_the_section_14_phase_gates() {
    let guidance = guidance();
    let universe = fixture_universe();
    let waves = wave_layering(&universe);
    let index = wave_index_by_task(&waves);

    let mut covered = BTreeSet::new();
    let mut spans: Vec<(u32, &str, usize, usize)> = Vec::new();
    for phase in &guidance.phases {
        let mut lo = usize::MAX;
        let mut hi = 0usize;
        for task in &phase.tasks {
            let wave = *index
                .get(task)
                .unwrap_or_else(|| panic!("§14 phase {} names unknown task {task}", phase.phase));
            assert!(
                covered.insert(task.clone()),
                "{task} is claimed by more than one §14 phase"
            );
            lo = lo.min(wave);
            hi = hi.max(wave);
        }
        spans.push((phase.phase, phase.scope.as_str(), lo, hi));
    }
    assert_eq!(
        covered.len(),
        universe.tasks.len(),
        "the §14 phase mapping does not cover all 17 tasks: {covered:?}"
    );

    let mut breaches = Vec::new();
    for window in spans.windows(2) {
        let (earlier, later) = (&window[0], &window[1]);
        if earlier.3 >= later.2 {
            breaches.push(format!(
                "phase {} ({}) spans waves {}..={} but phase {} ({}) starts at wave {}",
                earlier.0, earlier.1, earlier.2, earlier.3, later.0, later.1, later.2
            ));
        }
    }
    assert!(
        breaches.is_empty(),
        "§14 phase gates are not honoured by the generated layering.\n  waves: {}\n  {}",
        render_waves(&waves),
        breaches.join("\n  ")
    );
}

/// The workstream each task file declares for itself, cross-checked the same
/// way. Unlike the §14 mapping this is not transcribed by hand — the task
/// files say it — so a breach here is unambiguously the graph's fault.
#[test]
fn declared_workstreams_are_gates_in_the_generated_layering() {
    let universe = fixture_universe();
    let waves = wave_layering(&universe);
    let index = wave_index_by_task(&waves);

    let mut spans: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for task in &universe.tasks {
        let raw = fs::read_to_string(&task.source_path).expect("re-read the task file");
        let workstream = raw
            .lines()
            .find_map(|line| line.trim().strip_prefix("workstream:"))
            .map(|value| value.trim().to_string())
            .unwrap_or_else(|| panic!("{} declares no workstream", task.canonical_task_id));
        let wave = index[&task.canonical_task_id];
        let span = spans.entry(workstream).or_insert((usize::MAX, 0));
        span.0 = span.0.min(wave);
        span.1 = span.1.max(wave);
    }

    // Keys sort as "W0 …" < "W1 …" < … so BTreeMap order is workstream order.
    let ordered = spans.iter().collect::<Vec<_>>();
    let mut breaches = Vec::new();
    for window in ordered.windows(2) {
        let ((a_name, a_span), (b_name, b_span)) = (window[0], window[1]);
        if a_span.1 >= b_span.0 {
            breaches.push(format!(
                "'{a_name}' spans waves {}..={} but '{b_name}' starts at wave {}",
                a_span.0, a_span.1, b_span.0
            ));
        }
    }
    assert!(
        breaches.is_empty(),
        "declared workstreams overlap in the generated layering:\n  {}",
        breaches.join("\n  ")
    );
}

pub(super) fn render_waves(waves: &[Vec<String>]) -> String {
    waves
        .iter()
        .enumerate()
        .map(|(wave, ids)| format!("w{wave}[{}]", ids.join(" ")))
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// Failure modes
// ---------------------------------------------------------------------------

/// One `(from, to)` substitution applied to one named task file.
type Edit<'a> = (&'a str, &'a str, &'a str);

/// Copy the 17 real task files into a temp directory, applying the given
/// substitutions. Every edit must land, so a fixture change that invalidates a
/// failure-mode test fails loudly instead of quietly testing nothing.
fn mutated_fixture(edits: &[Edit<'_>]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut applied = vec![false; edits.len()];
    for entry in fs::read_dir(fixture_root()).expect("read fixture") {
        let path = entry.expect("fixture entry").path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("TASK-") || !name.ends_with(".md") {
            continue;
        }
        let mut raw = fs::read_to_string(&path).expect("read fixture task");
        for (index, (file, from, to)) in edits.iter().enumerate() {
            if *file != name {
                continue;
            }
            assert!(raw.contains(from), "{name} does not contain {from:?}");
            raw = raw.replacen(from, to, 1);
            applied[index] = true;
        }
        fs::write(dir.path().join(name), raw).expect("write mutated task");
    }
    for (index, done) in applied.iter().enumerate() {
        assert!(
            done,
            "edit {index} targets a file that is not in the fixture"
        );
    }
    dir
}

fn extraction_error(dir: &Path) -> String {
    match extract_task_universe_for_generated_run(&plan_task_text(dir)) {
        Ok(_) => panic!("a malformed task set must not plan; it produced a universe"),
        Err(error) => error.to_string(),
    }
}

/// The file path a message must name. Compared as the platform renders it,
/// because that is what a user reads in the failure.
fn task_path(dir: &Path, file: &str) -> String {
    dir.join(file).display().to_string()
}

#[test]
fn a_task_file_with_no_yaml_fence_fails_the_run_naming_the_file() {
    const FILE: &str = "TASK-TDL-030-provider-capability-interface.md";
    let dir = mutated_fixture(&[(FILE, "```yaml\n", "")]);
    let message = extraction_error(dir.path());
    assert!(
        message.contains(&task_path(dir.path(), FILE)),
        "message does not name the offending file: {message}"
    );
    assert!(
        message.contains("no fenced") && message.contains("yaml"),
        "message does not say why: {message}"
    );
}

#[test]
fn a_dangling_depends_on_fails_the_run_naming_the_file() {
    const FILE: &str = "TASK-TDL-010-registry-schema-v1.md";
    let dir = mutated_fixture(&[(
        FILE,
        "depends_on: ['TASK-TDL-001']",
        "depends_on: ['TASK-TDL-001', 'TASK-TDL-999']",
    )]);
    let message = extraction_error(dir.path());
    assert!(
        message.contains(&task_path(dir.path(), FILE)),
        "message does not name the offending file: {message}"
    );
    assert!(
        message.contains("TASK-TDL-999") && message.contains("unresolved"),
        "message does not name the unresolved reference: {message}"
    );
}

/// A pair claiming both orders of one edge from the SAME file: TASK-TDL-020
/// already declares `depends_on: ['TASK-TDL-010']`, so adding TASK-TDL-010 to
/// its `blocks:` makes it say "010 must precede me" and "I must precede 010" at
/// once. TASK-TDL-010's own `blocks: ['TASK-TDL-020']` is cleared in the same
/// mutation, because leaving it would trip the mutual-block check first (see
/// the next test) and this one would never reach its branch.
#[test]
fn a_blocks_versus_depends_on_contradiction_fails_the_run_naming_the_file() {
    const FILE: &str = "TASK-TDL-020-ohlcv-validation-reports.md";
    let dir = mutated_fixture(&[
        (
            FILE,
            "blocks: ['TASK-TDL-030', 'TASK-TDL-090']",
            "blocks: ['TASK-TDL-010', 'TASK-TDL-030', 'TASK-TDL-090']",
        ),
        (
            "TASK-TDL-010-registry-schema-v1.md",
            "blocks: ['TASK-TDL-020']",
            "blocks: []",
        ),
    ]);
    let message = extraction_error(dir.path());
    assert!(
        message.contains(&task_path(dir.path(), FILE)),
        "message does not name the offending file: {message}"
    );
    assert!(
        message.contains("both blocks and depends_on"),
        "message does not name the contradiction: {message}"
    );
    assert!(
        message.contains("TASK-TDL-020") && message.contains("TASK-TDL-010"),
        "message does not name both ends of the edge: {message}"
    );
}

/// The other contradiction the union cannot absorb: two files each claiming to
/// block the other. Reported by name with BOTH file paths, which is the whole
/// point — neither file is individually wrong.
#[test]
fn two_tasks_blocking_each_other_fails_the_run_naming_both_files() {
    const A: &str = "TASK-TDL-010-registry-schema-v1.md";
    const B: &str = "TASK-TDL-020-ohlcv-validation-reports.md";
    let dir = mutated_fixture(&[(
        B,
        "blocks: ['TASK-TDL-030', 'TASK-TDL-090']",
        "blocks: ['TASK-TDL-010', 'TASK-TDL-030', 'TASK-TDL-090']",
    )]);
    let message = extraction_error(dir.path());
    assert!(
        message.contains("each declare that they block the other"),
        "message does not name the contradiction: {message}"
    );
    for file in [A, B] {
        assert!(
            message.contains(&task_path(dir.path(), file)),
            "message does not name {file}: {message}"
        );
    }
}

/// A cycle is reported as a closed path of task ids, not as a file path — the
/// cycle is a property of the graph and no single file owns it. The last
/// assertion pins that gap rather than wishing it away: a reader has to map
/// ids back to files themselves, which for a 17-task set is a real cost.
#[test]
fn a_dependency_cycle_fails_the_run_naming_every_task_on_the_cycle() {
    const FILE: &str = "TASK-TDL-001-data-lake-gap-audit.md";
    let dir = mutated_fixture(&[(FILE, "depends_on: []", "depends_on: ['TASK-TDL-140']")]);
    let message = extraction_error(dir.path());
    assert!(
        message.contains("dependency cycle detected"),
        "message does not report a cycle: {message}"
    );
    let path = message
        .rsplit("dependency cycle detected: ")
        .next()
        .expect("cycle path is present")
        .split(" -> ")
        .map(str::trim)
        .collect::<Vec<_>>();
    assert!(
        path.len() >= 3,
        "cycle path is too short to act on: {message}"
    );
    assert_eq!(
        path.first(),
        path.last(),
        "reported cycle does not close: {message}"
    );
    // The edge the mutation introduced must be on the reported path.
    assert!(
        path.windows(2)
            .any(|pair| pair == ["TASK-TDL-001", "TASK-TDL-140"]),
        "the injected edge is not on the reported cycle: {message}"
    );
    assert!(
        !message.contains(&task_path(dir.path(), FILE)),
        "the cycle diagnostic now names a file; update this test and the report"
    );
}
