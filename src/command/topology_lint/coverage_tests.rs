//! Requirement-coverage tests.
//!
//! These run over the 17 checked-in task files of
//! `PRD-TRADING-DATA-LAKE-AHDM-001` and the 93 requirement IDs they claim
//! between them — a check exercised only against hand-made two-requirement
//! fixtures tells you nothing about the corpus it was written for.
//!
//! **What the PRD side is.** The fixture tree checks in the tasks but not the
//! PRD, so these tests synthesize the sibling PRD from the union of the tasks'
//! own claims. That makes the two clean directions in the first test a plumbing
//! result, not evidence about the real document: the real
//! `PRD-TRADING-DATA-LAKE-AHDM-001.md` defines exactly the same 93 IDs — checked
//! against the real file, which is why the count here is 93 and not a number
//! this fixture could have produced on its own — but this test cannot reach it.
//! The two mutation tests below are what prove each direction actually fires.

use super::*;

fn fixture_tasks() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/prd-trading-data-lake-ahdm-001")
}

/// The fixture PRD, assembled so the fixture task directory has the sibling
/// `<PRD-ID>.md` §3.1 requires. The fixture tree checks in the tasks but not the
/// PRD, so the PRD's requirement bullets are reconstructed from the union of
/// what the tasks claim — which is exactly the corpus the by-hand count of 93
/// was taken over.
fn write_corpus(dir: &Path) -> PathBuf {
    write_corpus_into(
        &dir.join("PRD-TRADING-DATA-LAKE-AHDM-001"),
        &dir.join("PRD-TRADING-DATA-LAKE-AHDM-001.md"),
    )
}

/// The same corpus, with the task directory and the PRD placed explicitly.
///
/// Split out so a test can put them in the two-root layout `/workflow-prd`
/// writes (`tasks/` and `prds/`) rather than only the §3.1 adjacent one.
fn write_corpus_into(tasks: &Path, prd_path: &Path) -> PathBuf {
    fs::create_dir_all(tasks).expect("create task dir");
    if let Some(parent) = prd_path.parent() {
        fs::create_dir_all(parent).expect("create prd dir");
    }
    let mut ids = BTreeSet::new();
    for entry in fs::read_dir(fixture_tasks()).expect("read fixtures") {
        let path = entry.expect("fixture entry").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if !name.starts_with("TASK-") || !name.ends_with(".md") {
            continue;
        }
        let raw = fs::read_to_string(&path).expect("read fixture task");
        fs::write(tasks.join(name), &raw).expect("copy fixture task");
        for id in Regex::new(r"REQ-[A-Z0-9]+-[0-9]{3}")
            .expect("id pattern")
            .find_iter(implements_line(&raw))
        {
            ids.insert(id.as_str().to_string());
        }
    }
    let mut prd = String::from("# PRD\n\n## 8. Requirements\n\n");
    for id in &ids {
        prd.push_str(&format!("- {id}: declared by the fixture corpus.\n"));
    }
    fs::write(prd_path, prd).expect("write prd");
    prd_path.to_path_buf()
}

fn implements_line(raw: &str) -> &str {
    raw.lines()
        .find(|line| line.trim_start().starts_with("implements:"))
        .unwrap_or_default()
}

#[test]
fn the_real_corpus_is_covered_in_both_directions() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_corpus(dir.path());
    let report = section(Some(&dir.path().join("PRD-TRADING-DATA-LAKE-AHDM-001")));
    assert!(
        report.contains("93 requirement(s)") && report.contains("93 claimed across 17 task(s)"),
        "{report}"
    );
    assert!(
        report.contains("every requirement is claimed by at least one task."),
        "{report}"
    );
    assert!(
        report.contains("every ID cited by a task is defined in the PRD."),
        "{report}"
    );
}

/// A requirement the PRD defines and no task claims is the decomposition gap
/// the check exists for, and it is named rather than counted.
#[test]
fn an_unclaimed_requirement_is_reported_by_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    let prd_path = write_corpus(dir.path());
    let mut prd = fs::read_to_string(&prd_path).expect("read prd");
    prd.push_str("- REQ-DL-999: nothing implements this.\n");
    fs::write(&prd_path, prd).expect("rewrite prd");
    let report = section(Some(&dir.path().join("PRD-TRADING-DATA-LAKE-AHDM-001")));
    assert!(
        report.contains("1 requirement(s) claimed by no task"),
        "{report}"
    );
    assert!(report.contains("REQ-DL-999"), "{report}");
}

/// The other direction: an ID a task cites that the PRD never defines. The
/// finding names the citing task, because the fix is in that file.
#[test]
fn an_id_no_prd_defines_names_the_task_that_cited_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_corpus(dir.path());
    let tasks = dir.path().join("PRD-TRADING-DATA-LAKE-AHDM-001");
    let task = tasks.join("TASK-TDL-001-data-lake-gap-audit.md");
    let raw = fs::read_to_string(&task).expect("read task");
    fs::write(
        &task,
        raw.replace("implements: []", "implements: [REQ-DL-404]"),
    )
    .expect("rewrite task");
    let report = section(Some(&tasks));
    assert!(report.contains("1 ID(s) cited by a task"), "{report}");
    assert!(
        report.contains("REQ-DL-404 cited by TASK-TDL-001"),
        "{report}"
    );
}

/// No PRD beside the directory: the section says what it looked for and stops.
/// It must not fail the lint, and must not silently print a clean report.
#[test]
fn an_unresolvable_prd_is_skipped_with_the_paths_it_tried() {
    let report = section(Some(&fixture_tasks()));
    assert!(report.contains("skipped"), "{report}");
    assert!(
        report.contains("PRD-TRADING-DATA-LAKE-AHDM-001.md"),
        "the `prd:` declaration should be one of the candidates: {report}"
    );
    assert!(
        !report.contains("every requirement is claimed"),
        "a skipped section must not read as a pass: {report}"
    );
}

/// The two-root layout: tasks under `tasks/`, the PRD under `prds/`.
///
/// This is what `/workflow-prd` and `/workflow-prd-spec` write, and it is the
/// case the §3.1-only candidate list could not resolve — the PRD is a sibling
/// of the `tasks/` root, not of the task directory. The section must produce a
/// real coverage report, not the skip it used to.
#[test]
fn a_prd_under_the_prds_root_resolves_for_a_task_dir_under_tasks() {
    let base = tempfile::tempdir().expect("tempdir");
    let name = "PRD-TRADING-DATA-LAKE-AHDM-001";
    let tasks = base.path().join("tasks").join(name);
    write_corpus_into(
        &tasks,
        &base
            .path()
            .join("prds")
            .join(name)
            .join(format!("{name}.md")),
    );

    let report = section(Some(&tasks));
    assert!(
        !report.contains("skipped"),
        "a PRD under prds/ must resolve: {report}"
    );
    assert!(
        report.contains("every requirement is claimed"),
        "the corpus covers itself in both directions: {report}"
    );
}

/// The flat and skills-chain shapes under `prds/` resolve too. Same root, three
/// filenames, because the two pipelines do not agree on how the file is named.
#[test]
fn the_other_prds_root_filenames_also_resolve() {
    let name = "PRD-TRADING-DATA-LAKE-AHDM-001";
    for relative in [
        PathBuf::from(format!("{name}.md")),
        PathBuf::from(name).join("PRD.md"),
    ] {
        let base = tempfile::tempdir().expect("tempdir");
        let tasks = base.path().join("tasks").join(name);
        write_corpus_into(&tasks, &base.path().join("prds").join(&relative));
        let report = section(Some(&tasks));
        assert!(
            !report.contains("skipped"),
            "prds/{} must resolve: {report}",
            relative.display()
        );
    }
}

/// A spec or recorded graph carries no claims. Saying so is the point: a
/// section that vanished would be indistinguishable from one that passed.
#[test]
fn a_non_task_source_says_the_check_does_not_apply() {
    let report = section(None);
    assert!(report.contains("only computed for --tasks"), "{report}");
}

/// The bullet form §3.3 mandates is the whole grammar. An ID inside a sentence
/// is not extracted — the guide says so, and the alternative is a check that
/// counts cross-references in prose as definitions.
#[test]
fn only_line_leading_bullets_define_a_requirement() {
    let ids = requirement_ids(concat!(
        "- REQ-DL-001: a real one.\n",
        "  * REQ-DL-002: indented, still a bullet.\n",
        "See REQ-DL-900 for context, which is prose.\n",
        "- REQ-dl-003: lowercase area is not the ID shape.\n",
        "- REQ-DL-04: two digits is not the ID shape.\n",
    ));
    assert_eq!(
        ids.into_iter().collect::<Vec<_>>(),
        ["REQ-DL-001", "REQ-DL-002"]
    );
}
