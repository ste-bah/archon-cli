//! Heading matching, and the report that keeps a missed section from being
//! silent.
//!
//! The spellings exercised here are the three a real fifteen-task
//! decomposition actually used, in the proportions it used them: nine files
//! wrote the canonical heading, four abbreviated it, two elaborated it.

use super::{declared_task_section_items, heading_matches_section, heading_near_misses, stem};

/// The stem only has to do one thing: bring two spellings of the same word to
/// the same place. Asserted directly because a stem that reduces one of a pair
/// further than the other is invisible through the near-miss tests above — they
/// simply report nothing, which is also what a correct stem does for unrelated
/// headings.
#[test]
fn inflections_of_one_word_reach_the_same_stem() {
    for (left, right) in [
        ("change", "changed"),
        ("change", "changes"),
        ("file", "files"),
        ("expect", "expected"),
        ("implement", "implementing"),
    ] {
        assert_eq!(stem(left), stem(right), "{left} and {right} disagree");
    }
}

/// And must not collapse words that mean different things.
#[test]
fn unrelated_words_keep_different_stems() {
    for (left, right) in [("expected", "forbidden"), ("files", "criteria")] {
        assert_ne!(stem(left), stem(right), "{left} and {right} collapsed");
    }
}

const CANONICAL: &str = "files expected to change";

fn section_with_heading(heading: &str) -> String {
    format!("# TASK-X\n\n{heading}\n\n- `src/a.rs` — the adapter.\n- `src/b.rs`\n")
}

#[test]
fn the_canonical_heading_matches() {
    assert!(heading_matches_section(
        "Files Expected to Change",
        CANONICAL
    ));
}

/// Four of fifteen task files abbreviated it this way, and every one parsed to
/// zero declared target files.
#[test]
fn an_abbreviated_heading_matches() {
    assert!(heading_matches_section("Files Expected", CANONICAL));
}

/// Two more elaborated it this way, with the same result.
#[test]
fn an_elaborated_heading_matches() {
    assert!(heading_matches_section(
        "Files Expected to Change During Implementation",
        CANONICAL
    ));
}

#[test]
fn case_and_trailing_punctuation_do_not_matter() {
    assert!(heading_matches_section("FILES EXPECTED:", CANONICAL));
    assert!(heading_matches_section("  files   expected  ", CANONICAL));
}

/// The sibling section in the same files. Widening the rule must not merge two
/// sections that mean opposite things.
#[test]
fn the_forbidden_section_is_not_the_expected_section() {
    assert!(!heading_matches_section("Files Forbidden", CANONICAL));
    assert!(!heading_matches_section(
        "Files Forbidden to Change",
        CANONICAL
    ));
}

/// Word-wise comparison, not string prefixing: `files expected` must not open a
/// section called `files expectedly changed`.
#[test]
fn a_longer_word_is_not_a_prefix() {
    assert!(!heading_matches_section(
        "Files Expectedly Changed",
        CANONICAL
    ));
}

#[test]
fn an_unrelated_section_does_not_match() {
    assert!(!heading_matches_section("Acceptance Criteria", CANONICAL));
    assert!(!heading_matches_section("Purpose", CANONICAL));
}

#[test]
fn every_observed_spelling_yields_the_declared_items() {
    for heading in [
        "## Files Expected to Change",
        "## Files Expected",
        "## Files Expected to Change During Implementation",
    ] {
        let raw = section_with_heading(heading);
        let items = declared_task_section_items(&raw, CANONICAL);
        assert_eq!(
            items,
            vec![
                "`src/a.rs` — the adapter.".to_string(),
                "`src/b.rs`".to_string()
            ],
            "heading {heading} parsed to {items:?}"
        );
    }
}

/// A section that matched needs no report, whichever spelling won it.
#[test]
fn a_matched_section_reports_nothing() {
    for heading in [
        "## Files Expected to Change",
        "## Files Expected",
        "## Files Expected to Change During Implementation",
    ] {
        let raw = section_with_heading(heading);
        assert!(
            heading_near_misses(&raw, CANONICAL).is_empty(),
            "heading {heading} was reported despite matching"
        );
    }
}

/// The case the widened rule cannot reach: a reordering. It must be reported
/// rather than silently yielding nothing, because that silence is what made the
/// same class of defect expensive three times over.
#[test]
fn a_reordered_heading_is_reported_as_a_near_miss() {
    let raw = section_with_heading("## Expected Files");
    assert!(declared_task_section_items(&raw, CANONICAL).is_empty());
    assert_eq!(heading_near_misses(&raw, CANONICAL), vec!["Expected Files"]);
}

#[test]
fn a_renamed_qualifier_is_reported_as_a_near_miss() {
    let raw = section_with_heading("## Files Expected to Be Changed by Implementation");
    assert_eq!(
        heading_near_misses(&raw, CANONICAL),
        vec!["Files Expected to Be Changed by Implementation"]
    );
}

/// The opposite section shares one significant word and must not be reported as
/// a candidate for this one — a false report is a false alarm every run.
#[test]
fn the_forbidden_section_is_not_a_near_miss() {
    let raw = section_with_heading("## Files Forbidden to Change");
    assert!(heading_near_misses(&raw, CANONICAL).is_empty());
}

#[test]
fn an_unrelated_document_reports_nothing() {
    let raw = "# TASK-X\n\n## Purpose\n\n- do the thing\n\n## Acceptance Criteria\n\n1. it works\n";
    assert!(heading_near_misses(raw, CANONICAL).is_empty());
}

/// A task that genuinely has no such section, and no heading resembling one, is
/// not a defect — absent must stay distinguishable from misspelled.
#[test]
fn a_genuinely_absent_section_is_not_a_near_miss() {
    let raw = "# TASK-X\n\n## Purpose\n\n- audit only, changes nothing\n";
    assert!(declared_task_section_items(raw, CANONICAL).is_empty());
    assert!(heading_near_misses(raw, CANONICAL).is_empty());
}

/// The whole parse path, not the heading helper alone.
///
/// The committed PRD fixture writes the canonical heading in all seventeen of
/// its files, which is why this defect survived every test the repository had:
/// the reference corpus is tidier than the decompositions people actually
/// produce. This builds a task file the way a real one drifted and takes it
/// through the public entry point, so a regression is caught at the layer every
/// consumer reads from rather than at the helper underneath it.
fn task_file(heading: &str) -> String {
    format!(
        "# TASK-ABC-001 — Drifted Heading\n\n\
         ```yaml\n\
         task_id: TASK-ABC-001\n\
         title: Drifted Heading\n\
         complexity: medium\n\
         status: pending\n\
         depends_on: []\n\
         blocks: []\n\
         implements: []\n\
         required_env_keys: []\n\
         required_tools: [cargo]\n\
         deliverable_contracts: []\n\
         ```\n\n\
         ## Purpose\n\n\
         Prove the section is read.\n\n\
         {heading}\n\n\
         - `src/first.rs` — the adapter.\n\
         - `src/second.rs`\n\n\
         ## Files Forbidden to Change\n\n\
         - `src/untouchable.rs`\n\n\
         ## Acceptance Criteria\n\n\
         1. It parses.\n"
    )
}

#[test]
fn a_drifted_heading_survives_the_whole_parse_path() {
    for heading in [
        "## Files Expected",
        "## Files Expected to Change During Implementation",
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("TASK-ABC-001.md"), task_file(heading)).expect("write task");
        let task = format!(
            "Implement the decomposed PRD task files under {} for PRD-ABC",
            dir.path().display()
        );
        let universe = crate::task_universe::extract_task_universe_for_generated_run(&task)
            .expect("the task universe extracts without error")
            .expect("a decomposed-PRD task description yields a task universe");
        let parsed = universe
            .tasks
            .iter()
            .find(|t| t.canonical_task_id == "TASK-ABC-001")
            .expect("the task is in the universe");
        assert_eq!(
            parsed.files_expected_to_change,
            vec![
                "`src/first.rs` — the adapter.".to_string(),
                "`src/second.rs`".to_string()
            ],
            "heading {heading} reached the universe as {:?}",
            parsed.files_expected_to_change
        );
        assert!(
            parsed.section_heading_issues.is_empty(),
            "a heading that matched was still reported: {:?}",
            parsed.section_heading_issues
        );
    }
}

/// And a heading the widened rule cannot reach must arrive at the universe
/// carrying its own explanation, rather than as an innocent empty list.
#[test]
fn an_unreachable_heading_reaches_the_universe_as_a_reported_issue() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("TASK-ABC-001.md"),
        task_file("## Expected Files"),
    )
    .expect("write task");
    let task = format!(
        "Implement the decomposed PRD task files under {} for PRD-ABC",
        dir.path().display()
    );
    let universe = crate::task_universe::extract_task_universe_for_generated_run(&task)
        .expect("the task universe extracts without error")
        .expect("a decomposed-PRD task description yields a task universe");
    let parsed = universe
        .tasks
        .iter()
        .find(|t| t.canonical_task_id == "TASK-ABC-001")
        .expect("the task is in the universe");
    assert!(parsed.files_expected_to_change.is_empty());
    assert_eq!(parsed.section_heading_issues.len(), 1, "{parsed:?}");
    assert!(
        parsed.section_heading_issues[0].contains("Expected Files")
            && parsed.section_heading_issues[0].contains("files expected to change"),
        "the report must name both the heading and the section it missed: {:?}",
        parsed.section_heading_issues
    );
}

#[test]
fn focused_subsections_reach_runtime_without_sibling_commands() {
    let raw = "```yaml\ntask_id: TASK-EXAMPLE-001\ntitle: Example\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Focused Tests\n### Tool calls\n- `mcp__example__fetch` with `{}`\n### Build\n```sh\ncargo build\ncargo nextest run\n```\n#### Content\n- `grep -q expected result.txt`\n## Other\n```sh\nexit 99\n```\n- `wrong_command`\n";
    let task = crate::task_universe::parsing::parse_task_file(
        std::path::Path::new("TASK-EXAMPLE-001.md"),
        raw,
    )
    .unwrap();
    for expected in [
        "mcp__example__fetch",
        "cargo build",
        "cargo nextest run",
        "grep -q expected",
    ] {
        assert!(
            task.focused_tests.iter().any(|s| s.contains(expected)),
            "missing {expected}: {:?}",
            task.focused_tests
        );
    }
    assert!(
        !task
            .focused_tests
            .iter()
            .any(|s| s.contains("exit 99") || s.contains("wrong_command"))
    );
}
