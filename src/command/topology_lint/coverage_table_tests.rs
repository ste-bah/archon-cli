//! Which tables state obligations: the header decides, the section can
//! exclude, and a goal counts while a non-goal never does.
//!
//! Split from `coverage_tests.rs` so each file stays under the size cap.

use std::collections::BTreeSet;

/// A non-goal with no owning task is the correct state. The first real run
/// reported six of them beside five genuine findings — noise at that ratio is
/// how a lint stops being read.
///
/// A goal, by contrast, IS a gap when nobody claims it: a live task set whose
/// every task passed left the goal "ingest into the shared registry" undone
/// because the goals table was never read (Issue-31), so goals now count.
#[test]
fn obligations_under_a_negating_heading_are_not_gaps() {
    let prd = "\
## 3. Goals\n\
| ID | Goal |\n\
| G-DL-001 | a goal, single-letter prefix, an obligation like any other |\n\
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
        BTreeSet::from(["AC-DL-003".to_string(), "G-DL-001".to_string()]),
        "goals and acceptance criteria are obligations; non-goals never are: {ids:?}"
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
