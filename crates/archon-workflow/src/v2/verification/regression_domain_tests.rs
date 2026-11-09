//! Cartesian decision table, mirrored in the round's domain map.
use super::*;

#[test]
fn regression_baseline_domain_table() {
    check_rows(include_str!("regression_domain_rows.txt"));
}

#[test]
fn regression_baseline_domain_table_with_crlf() {
    let rows = include_str!("regression_domain_rows.txt")
        .replace("\r\n", "\n")
        .replace('\n', "\r\n");
    check_rows(&rows);
}

fn check_rows(rows: &str) {
    let first = include_str!("../../../../../tests/fixtures/diagnostic_outputs/rustc-a.txt")
        .lines()
        .next()
        .unwrap();
    let other = include_str!("../../../../../tests/fixtures/diagnostic_outputs/rustc-b.txt")
        .lines()
        .next()
        .unwrap();
    let both = format!("{first}\n{other}");
    let root = std::path::Path::new(first.split_once("/src/").expect("recorded path").0);
    let mut count = 0;
    for row in rows
        .lines()
        .flat_map(|line| line.split(';'))
        .filter(|s| !s.is_empty())
    {
        count += 1;
        let c: Vec<_> = row.split('\t').collect();
        let [
            base,
            tip,
            identity,
            exit,
            named,
            deleted,
            claim,
            owner,
            expected,
        ] = c.as_slice()
        else {
            panic!("{row}")
        };
        let signature = match *identity {
            "equal" => Some(first),
            "subset" => Some(both.as_str()),
            "disjoint" | "base-only" => Some(other),
            _ => None,
        };
        let after = if matches!(*identity, "absent" | "base-only") {
            None
        } else {
            Some(first)
        };
        let verdict = |state: &str, sig: Option<&str>, at_tip: bool| {
            let mut run = HostRunVerdict {
                exit_code: Some(if state == "pass" {
                    0
                } else if at_tip && *exit == "different" {
                    2
                } else {
                    1
                }),
                ids_kept: true,
                ..Default::default()
            };
            if state == "fail" {
                if *named == "named" {
                    run.failing_tests.push("case".into());
                    run.failed_count = Some(1);
                }
                if let Some(sig) = sig {
                    run.signatures.insert(
                        if *named == "named" { "case" } else { "" }.into(),
                        sig.into(),
                    );
                }
            }
            if state == "pass"
                || (!at_tip && matches!(*base, "pass" | "fail") && *deleted == "deleted")
            {
                run.passed_tests.push(
                    if !at_tip && *deleted == "deleted" {
                        "removed"
                    } else {
                        "kept"
                    }
                    .into(),
                );
            }
            run
        };
        let before = if *base == "absent" {
            BTreeMap::new()
        } else {
            [("check".into(), verdict(base, signature, false))].into()
        };
        let now = if *tip == "absent" {
            BTreeMap::new()
        } else {
            [("check".into(), verdict(tip, after, true))].into()
        };
        let findings = compare(&["check".into()], &before, &now);
        let blocked = findings.iter().any(RegressionFinding::blocks);
        let actual = if blocked {
            if *owner == "owner" {
                "blocks-owner"
            } else {
                "blocks-unowned fix round"
            }
        } else if *tip == "fail" {
            "reported"
        } else {
            "Accepted"
        };
        assert_eq!(actual, *expected, "{row}: {findings:?}");
        // A claim never changes the evidence. Check the host-output subset predicate too.
        if *claim == "present" && *tip == "fail" {
            let stamp = super::super::baseline_rule::BaselineStamp {
                pre_existing_diagnostics: vec![super::super::baseline_rule::PreExistingCommand {
                    command: "check".into(),
                    files: if *base == "fail" {
                        crate::v2::write::test_baseline_parse::diagnostic_files_for(
                            signature.unwrap_or(""),
                            root,
                            "check",
                        )
                    } else {
                        vec![]
                    },
                    repository_root: root.to_path_buf(),
                    identity: if *base == "fail" {
                        signature.map(|s| (Some(1), s.into()))
                    } else {
                        None
                    },
                }],
                ..Default::default()
            };
            let covered = stamp.pre_existing_diagnostics_cover(
                "check",
                after.unwrap_or(""),
                Some(if *exit == "different" { 2 } else { 1 }),
            );
            let expected_cover =
                *base == "fail" && *exit == "same" && matches!(*identity, "equal" | "subset");
            assert_eq!(covered, expected_cover, "{row}");
        }
    }
    assert_eq!(count, 2304);
}
