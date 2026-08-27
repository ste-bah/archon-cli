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
use regex::Regex;

#[test]
fn coverage_calls_the_shared_obligation_extractor() {
    let source = include_str!("coverage.rs");
    assert!(
        source.contains("use archon_workflow::obligation_ids::obligation_ids;")
            && source.matches("obligation_ids(prd)").count() >= 2,
        "topology coverage must call the shared obligation extractor for set coverage and reporting"
    );
    assert!(
        !source.contains("fn obligation_ids("),
        "a private extractor can drift from trace and freeze semantics"
    );
}

/// Compiled once. Building it inside the per-file loop recompiled the same
/// constant pattern for every fixture task, which is the whole cost of this
/// helper twice over.
fn requirement_id_pattern() -> &'static Regex {
    static PATTERN: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"REQ-[A-Z0-9]+-[0-9]{3}").expect("id pattern"));
    &PATTERN
}

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
        for id in requirement_id_pattern().find_iter(implements_line(&raw)) {
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
        report.contains("93 obligation(s)") && report.contains("93 claimed across 17 task(s)"),
        "{report}"
    );
    assert!(
        report.contains("every obligation is claimed by at least one task."),
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
        report.contains("1 obligation(s) claimed by no task"),
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
        !report.contains("every obligation is claimed"),
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
        report.contains("every obligation is claimed"),
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
    let ids = archon_workflow::obligation_ids::obligation_ids(concat!(
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

/// A PRD states obligations in tables as well as bullets. Nine acceptance
/// criteria were once invisible to this check because only `REQ-` bullets were
/// ever scanned, so an obligation with no owner went through a whole
/// decomposition unnoticed.
#[test]
fn table_stated_obligations_are_detected_by_family() {
    let prd = "\
| ID | Acceptance criterion |\n\
|---|---|\n\
| AC-DL-001 | the first thing holds |\n\
| AC-DL-003 | ingestion stores a validation report |\n\
| NFR-002 | it is fast enough |\n\
\n\
- REQ-DL-010: a bullet requirement, owned by the bullet pattern\n";
    let ids = archon_workflow::obligation_ids::obligation_ids(prd);
    assert_eq!(
        ids.into_iter().collect::<Vec<_>>(),
        vec!["AC-DL-001", "AC-DL-003", "NFR-002", "REQ-DL-010"]
    );
}

/// `REQ` is excluded so one requirement never counts twice: the bullet pattern
/// already owns it, and a PRD that also tabulates its requirements would
/// otherwise double-report every one.
#[test]
fn requirement_ids_are_not_counted_twice_when_also_tabulated() {
    let prd = "| REQ-DL-010 | also in a table |\n- REQ-DL-010: the bullet\n";
    assert_eq!(
        archon_workflow::obligation_ids::obligation_ids(prd),
        BTreeSet::from(["REQ-DL-010".to_string()])
    );
}

/// Prose that merely mentions an id is not an obligation: only a leading table
/// cell counts, for the same reason the bullet pattern requires a line start.
#[test]
fn an_id_mentioned_mid_table_is_not_an_obligation() {
    let prd = "| thing | as required by AC-DL-003 |\n";
    assert!(archon_workflow::obligation_ids::obligation_ids(prd).is_empty());
}

/// The regression: `render` returns early when every cited ID is known, and the
/// obligations report was appended only to the other branch — so on a CLEAN
/// corpus, the exact case it exists to examine, it printed nothing at all. Real
/// output caught this; no unit test would have, because both branches build the
/// same string and only one was exercised.
#[test]
fn obligations_are_reported_on_the_clean_branch_too() {
    let prd = "- REQ-DL-010: a claimed requirement\n| ID | Acceptance criterion |\n| AC-DL-003 | nobody owns this |\n";
    let claims = vec![crate::command::topology_task_graph::TaskRequirementClaims {
        task_id: "TASK-A".to_string(),
        source_path: "TASK-A.md".to_string(),
        implements: vec!["REQ-DL-010".to_string()],
    }];
    let rendered = super::render(std::path::Path::new("PRD.md"), prd, &claims);
    assert!(
        rendered.contains("every ID cited by a task is defined in the PRD"),
        "this must be the clean branch: {rendered}"
    );
    assert!(
        rendered.contains("AC-DL-003"),
        "the uncited obligation must still be reported: {rendered}"
    );
}

/// A non-goal with no owning task is the correct state. The first real run
/// reported six of them beside five genuine findings — noise at that ratio is
/// how a lint stops being read.
#[test]
fn obligations_under_a_negating_heading_are_not_gaps() {
    let prd = "\
## 3. Goals\n\
| ID | Goal |\n\
| G-DL-001 | a goal, single-letter prefix, never an obligation |\n\
\n\
## 4. Non-Goals\n\
| ID | Non-goal |\n\
| NG-DL-001 | deliberately not done |\n\
\n\
## 12. Acceptance Criteria\n\
| ID | Acceptance criterion |\n\
| AC-DL-003 | this one is a real obligation |\n";
    let ids = archon_workflow::obligation_ids::obligation_ids(prd);
    assert_eq!(
        ids,
        BTreeSet::from(["AC-DL-003".to_string()]),
        "only the acceptance criteria are obligations: {ids:?}"
    );
}

/// The exclusion ends with its section: an obligation after a non-goals block
/// is still an obligation.
#[test]
fn the_exclusion_does_not_leak_past_its_own_section() {
    let prd = "## Out of scope\n| ID | Acceptance criterion |\n| NG-001 | not this |\n\
## Criteria\n| ID | Acceptance criterion |\n| AC-X-001 | but this |\n";
    let ids = archon_workflow::obligation_ids::obligation_ids(prd);
    assert_eq!(ids, BTreeSet::from(["AC-X-001".to_string()]));
}

/// A PRD tabulates reference data too, and those rows carry ids. The first real
/// run reported five timeframes as unowned obligations. What separates a
/// timeframe from an acceptance criterion is the column header, not the prefix.
#[test]
fn a_reference_data_table_is_not_an_obligation_table() {
    let prd = "\
## Required native timeframes\n\
| ID | Timeframe | Production rule |\n\
|---|---|---|\n\
| TF-001 | 1W | Must be fetched as native weekly candles. |\n\
\n\
## Acceptance Criteria\n\
| ID | Acceptance criterion |\n\
|---|---|\n\
| AC-DL-003 | ingestion stores a validation report |\n";
    let ids = archon_workflow::obligation_ids::obligation_ids(prd);
    assert_eq!(
        ids,
        BTreeSet::from(["AC-DL-003".to_string()]),
        "a reference-data row is not an obligation: {ids:?}"
    );
}

/// The header verdict applies to the whole table and no further: two tables in
/// one section are judged separately.
#[test]
fn each_table_is_judged_by_its_own_header() {
    let prd = "\
## Section\n\
| ID | Symbol |\n\
| SY-001 | not an obligation |\n\
\n\
| ID | Acceptance criterion |\n\
| AC-X-001 | an obligation |\n";
    let ids = archon_workflow::obligation_ids::obligation_ids(prd);
    assert_eq!(ids, BTreeSet::from(["AC-X-001".to_string()]));
}

/// One malformed spec used to take the entire section down: a decomposition
/// wrote `task_id: TASK-DL-010-gap-audit` (the whole filename stem) and the
/// coverage check reported `could not read task claims`, so nothing was said
/// about the eighteen files that parsed fine.
#[test]
fn one_unparseable_spec_does_not_blind_the_whole_section() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    std::fs::write(
        dir.join("TASK-DL-010-gap-audit.md"),
        "# TASK-DL-010-gap-audit\n\n```yaml\ntask_id: TASK-DL-010-gap-audit\ntitle: \"broken\"\ncomplexity: small\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("TASK-DL-020-good.md"),
        "# TASK-DL-020-good\n\n```yaml\ntask_id: TASK-DL-020\ntitle: \"fine\"\ncomplexity: small\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: [REQ-DL-001]\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n",
    )
    .unwrap();

    let rendered = super::section(Some(dir));
    assert!(
        rendered.contains("did not parse and were EXCLUDED"),
        "the skipped file must be named, not silently dropped: {rendered}"
    );
    assert!(
        rendered.contains("TASK-DL-010-gap-audit.md"),
        "the reader must know WHICH file was skipped: {rendered}"
    );
    assert!(
        !rendered.contains("could not read task claims"),
        "one bad file must not abort the section: {rendered}"
    );
}

/// Zero parsed files is not a clean bill of health. A whole decomposition once
/// printed "every declared contract is satisfiable as written" while all
/// fifteen of its specs were unreadable, and the gate exited zero.
#[test]
fn nothing_parsed_is_reported_as_not_a_pass() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("TASK-DL-010-broken.md"),
        "# TASK-DL-010-broken\n\n```yaml\ntask_id: TASK-DL-010\ntitle: \"x\"\ncomplexity: small\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts:\n  - artifact_path: a/b.json\n```\n",
    )
    .unwrap();

    let rendered = super::super::contracts::section(Some(temp.path()));
    assert!(
        rendered.contains("this is not a pass"),
        "a section that examined nothing must say so: {rendered}"
    );
    assert!(
        !rendered.contains("every declared contract is satisfiable"),
        "it must never claim a pass over zero files: {rendered}"
    );

    // and the gate must block on it
    let blocking = super::super::contracts::blocking_findings(Some(temp.path()));
    assert!(
        !blocking.is_empty(),
        "a spec the runtime's own parser cannot read must block the gate"
    );
    assert!(
        blocking[0].contains("missing field `kind`"),
        "the reason must name the defect: {blocking:?}"
    );
}

/// A decomposition once numbered its tasks 010, 020, 040, 050 — leaving the 030
/// slot empty and never writing that task at all. Four requirements were
/// orphaned by the omission, the lint reported them, and the gate exited zero.
/// Work nobody claims is work nobody does, and the run reports success without
/// it.
#[test]
fn a_requirement_no_task_claims_blocks_the_gate() {
    let temp = tempfile::tempdir().unwrap();
    let tasks = temp.path().join("tasks").join("PRD-X");
    let prds = temp.path().join("prds");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::create_dir_all(&prds).unwrap();
    std::fs::write(
        prds.join("PRD-X.md"),
        "- REQ-X-010: the claimed one\n- REQ-X-020: the orphan\n",
    )
    .unwrap();
    std::fs::write(
        tasks.join("TASK-X-010-only.md"),
        "# TASK-X-010-only\n\n```yaml\ntask_id: TASK-X-010\ntitle: \"t\"\ncomplexity: small\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: [REQ-X-010]\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Focused Tests\n\n- `cargo test -p x thing`\n",
    )
    .unwrap();

    let orphans = super::unclaimed_requirements(Some(&tasks));
    assert_eq!(orphans, vec!["REQ-X-020".to_string()], "{orphans:?}");
}

#[test]
fn malformed_prd_ids_and_phantom_task_citations_are_gate_findings() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tasks = dir.path().join("PRD-X");
    fs::create_dir_all(&tasks).unwrap();
    fs::write(
        dir.path().join("PRD-X.md"),
        "## Requirements\n- REQ-X-001: valid\n- REQ-X2-002: malformed\n",
    )
    .unwrap();
    fs::write(
        tasks.join("TASK-X-010-body.md"),
        "# Body\n\n```yaml\ntask_id: TASK-X-010\ntitle: Body\ncomplexity: medium\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: [REQ-X-001, REQ-X-999]\nrequired_env_keys: []\nrequired_tools: [sh]\ndeliverable_contracts: []\n```\n\n## Focused Tests\n- `sh -c 'exit 1'`\n",
    )
    .unwrap();

    let findings = policy_findings(Some(&tasks));
    assert!(
        findings.iter().any(|finding| {
            finding.text.contains("REQ-X2-002")
                && finding.text.contains("REQ-<LETTERS>-<NNN>")
                && finding.text.contains("rename")
        }),
        "{findings:?}"
    );
    assert!(
        findings.iter().any(|finding| {
            finding.text.contains("TASK-X-010")
                && finding.text.contains("REQ-X-999")
                && finding.text.contains("remove it from")
        }),
        "{findings:?}"
    );
}

#[test]
fn coverage_policy_findings_retain_exact_backing_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tasks = dir.path().join("PRD-X");
    fs::create_dir_all(&tasks).unwrap();
    let prd = dir.path().join("PRD-X.md");
    fs::write(&prd, "- REQ-X-001: valid\n").unwrap();
    let task = tasks.join("TASK-X-010-body.md");
    fs::write(
        &task,
        "# Body\n\n```yaml\ntask_id: TASK-X-010\ntitle: Body\ncomplexity: medium\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: [REQ-X-999]\nrequired_env_keys: []\nrequired_tools: [sh]\ndeliverable_contracts: []\n```\n\n## Focused Tests\n- `sh -c 'exit 1'`\n",
    )
    .unwrap();

    let findings = policy_findings(Some(&tasks));
    let phantom = findings
        .iter()
        .find(|finding| finding.text.contains("REQ-X-999"))
        .expect("phantom finding");
    assert_eq!(phantom.subject, "TASK-X-010");
    assert_eq!(phantom.source_path, task);
    let unclaimed = findings
        .iter()
        .find(|finding| finding.text.contains("REQ-X-001"))
        .expect("unclaimed finding");
    assert_eq!(unclaimed.source_path, prd.clone());

    let evaluation = crate::command::topology_lint::evaluate_lint(
        dir.path(),
        &crate::command::topology_lint::LintSource::Tasks(tasks),
        archon_core::config::GateMode::Observe,
    )
    .unwrap();
    let wired_phantom = evaluation
        .findings
        .iter()
        .find(|finding| finding.text.contains("REQ-X-999"))
        .expect("wired phantom finding");
    assert_eq!(wired_phantom.source_path.as_deref(), Some(task.as_path()));
    let wired_unclaimed = evaluation
        .findings
        .iter()
        .find(|finding| finding.text.contains("REQ-X-001"))
        .expect("wired unclaimed finding");
    assert_eq!(wired_unclaimed.source_path.as_deref(), Some(prd.as_path()));
}
