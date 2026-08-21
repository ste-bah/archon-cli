use super::*;

fn wave() -> Vec<WaveClaim> {
    vec![
        WaveClaim::new(
            "impl-tdl-020",
            ["crates/archon-trading/src/data_store/coverage.rs".to_string()],
        ),
        WaveClaim::new(
            "impl-tdl-030",
            ["crates/archon-trading/src/data_lake/contracts.rs".to_string()],
        ),
    ]
}

/// The live case: TDL-020 needed `ahdm_test_support_a.rs`, nothing else in the
/// wave claimed it, and an hour of correct work was discarded anyway.
#[test]
fn an_unclaimed_path_is_granted() {
    let outcome = resolve_scope_extension(
        "impl-tdl-020",
        "crates/archon-trading/src/data_store/ahdm_test_support_a.rs",
        &wave(),
    );

    assert!(outcome.is_granted(), "{outcome:?}");
}

/// The disjoint-ownership invariant is preserved: a path another item owns is
/// never handed over, and the refusal names the holder.
#[test]
fn a_path_another_item_owns_is_contested_and_names_the_holder() {
    let outcome = resolve_scope_extension(
        "impl-tdl-020",
        "crates/archon-trading/src/data_lake/contracts.rs",
        &wave(),
    );

    assert_eq!(
        outcome,
        WriteScopeExtension::Contested {
            path: "crates/archon-trading/src/data_lake/contracts.rs".to_string(),
            holder: "impl-tdl-030".to_string(),
        }
    );
}

/// An item never contests itself — re-declaring a path it already owns is a
/// no-op grant, not a deadlock against its own claim.
#[test]
fn an_item_does_not_contest_its_own_claim() {
    let outcome = resolve_scope_extension(
        "impl-tdl-020",
        "crates/archon-trading/src/data_store/coverage.rs",
        &wave(),
    );

    assert!(outcome.is_granted(), "{outcome:?}");
}

/// Directory claims cover the files beneath them, so a granted extension cannot
/// tunnel under another item's scope.
#[test]
fn a_directory_claim_contests_a_file_beneath_it() {
    let wave = vec![WaveClaim::new(
        "impl-tdl-010",
        ["crates/archon-trading/src/data_lake".to_string()],
    )];

    let outcome = resolve_scope_extension(
        "impl-tdl-040",
        "crates/archon-trading/src/data_lake/identity.rs",
        &wave,
    );

    assert!(
        matches!(outcome, WriteScopeExtension::Contested { .. }),
        "{outcome:?}"
    );
}

/// A prefix that is not a path boundary is a different path, not a parent.
#[test]
fn a_coincidental_prefix_is_not_a_claim() {
    let wave = vec![WaveClaim::new(
        "impl-tdl-010",
        ["crates/archon-trading/src/data_la".to_string()],
    )];

    let outcome = resolve_scope_extension(
        "impl-tdl-040",
        "crates/archon-trading/src/data_lake/identity.rs",
        &wave,
    );

    assert!(outcome.is_granted(), "{outcome:?}");
}

/// The batch form splits a patch's out-of-scope paths into what may be kept and
/// what must be raised as a gap.
#[test]
fn a_mixed_patch_splits_into_grants_and_contests() {
    let (granted, contested) = resolve_scope_extensions(
        "impl-tdl-020",
        [
            "crates/archon-trading/src/data_store/ahdm_test_support_a.rs",
            "crates/archon-trading/src/data_lake/contracts.rs",
            "crates/archon-trading/src/data_store/util.rs",
        ],
        &wave(),
    );

    assert_eq!(
        granted,
        vec![
            "crates/archon-trading/src/data_store/ahdm_test_support_a.rs".to_string(),
            "crates/archon-trading/src/data_store/util.rs".to_string(),
        ]
    );
    assert_eq!(contested.len(), 1);
    assert_eq!(
        contested[0].path(),
        "crates/archon-trading/src/data_lake/contracts.rs"
    );
}

/// A wave of one has nobody to contest with — every extension is granted.
#[test]
fn a_solo_wave_grants_everything() {
    let wave = vec![WaveClaim::new("impl-tdl-040", ["a.rs".to_string()])];
    let (granted, contested) = resolve_scope_extensions("impl-tdl-040", ["b.rs", "c.rs"], &wave);

    assert_eq!(granted, vec!["b.rs".to_string(), "c.rs".to_string()]);
    assert!(contested.is_empty());
}
