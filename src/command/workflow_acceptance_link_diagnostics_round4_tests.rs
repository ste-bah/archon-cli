use super::*;

#[path = "workflow_acceptance_link_diagnostics_read_sets_tests.rs"]
mod read_sets;

#[test]
fn quoted_words_never_become_unowned_and_plain_occurrences_still_do() {
    let repo = repository();
    let root = repo.path();
    for command in [
        "printf 'ok\\n'",
        "grep -q 'foo\\.bar' f",
        "sed -n 's/x\\.y/z/p' f",
        "grep -q 'data/missing.json' f",
        "test -f \"data/in.json\"",
        "inspect 'data\\x.json'",
        "test -f 'present.txt'",
        "test -f 'owned.json'",
        "p=data/in.json; test -f \"$p\"",
        "grep -q 'data.json' f; echo data.json",
        "grep -q 'data/x.json' f; echo data/x.json",
    ] {
        let d = discover(
            &skeleton("owned.json"),
            &contract(command),
            root,
            Some(root),
            root,
        );
        assert!(d.unowned.is_empty(), "{command}: {d:?}");
        if command.contains("present.txt")
            || command.contains("owned.json")
            || ["printf 'ok", "grep -q 'foo", "sed -n 's/x"]
                .iter()
                .any(|prefix| command.starts_with(prefix))
        {
            assert!(d.unconfirmed.is_empty(), "{d:?}");
        }
    }
    for command in [
        "cat data.json; grep -q 'data.json' log",
        "cat data/x.json; grep -q 'data/x.json' log",
    ] {
        let d = discover(
            &skeleton("owned.json"),
            &contract(command),
            root,
            Some(root),
            root,
        );
        let name = if command.contains("data/x") {
            root.join("data").join("x.json")
        } else {
            root.join("data.json")
        };
        assert_eq!(d.unowned, [("AC-X-001".into(), name)], "{d:?}");
        assert!(d.unconfirmed.is_empty(), "{d:?}");
    }
}

#[test]
fn link_output_is_bounded_independently_of_link_count() {
    let d = Diagnostics {
        links: (0..22500)
            .map(|_| Link {
                task: "t".into(),
                check: "c".into(),
                membership: true,
                reads_deliverable: true,
            })
            .collect(),
        ..Default::default()
    };
    assert_eq!(lines(&d), ["input-link: links=22500"]);
}

#[test]
fn r18_plan_bench_greenfield() {
    let s: TaskSkeleton = serde_json::from_str(include_str!(
        "../../tests/fixtures/r18_plan/task-skeleton.json"
    ))
    .unwrap();
    let c: AcceptanceContract = serde_json::from_str(include_str!(
        "../../tests/fixtures/r18_plan/acceptance-contract.json"
    ))
    .unwrap();
    let repo = repository_with(&[
        (
            "prd.md",
            include_str!("../../tests/fixtures/r18_plan/prd.md"),
        ),
        (
            "acceptance-contract.json",
            include_str!("../../tests/fixtures/r18_plan/acceptance-contract.json"),
        ),
        (
            "task-skeleton.json",
            include_str!("../../tests/fixtures/r18_plan/task-skeleton.json"),
        ),
    ]);
    let root = repo.path();
    for artifact in s.tasks.iter().flat_map(|task| &task.deliverable_contracts) {
        assert!(!absolute(root, &artifact.artifact_path).exists());
    }
    let d = discover(&s, &c, root, Some(root), root);
    assert!(d.base_unavailable.is_none(), "{d:?}");
    let mut unowned = d.unowned.clone();
    read_sets::sort(&mut unowned);
    assert_eq!(unowned, read_sets::expected(root, false));
    let mut unconfirmed = d.unconfirmed.clone();
    read_sets::sort(&mut unconfirmed);
    assert_eq!(unconfirmed, read_sets::expected(root, true));
    println!(
        "R18 greenfield read sets: {} unowned, {} unconfirmed",
        unowned.len(),
        unconfirmed.len()
    );
    for check in c.acceptance.iter().chain(&c.supplementary) {
        let reads = input_discovery_for_check(check, root, root, None);
        assert!(
            reads.unshaped_missing.is_empty(),
            "{}: {:?}",
            check.id,
            reads.unshaped_missing
        );
        assert!(
            reads.absolute_reads.is_empty(),
            "{}: {:?}",
            check.id,
            reads.absolute_reads
        );
        let mut expected = read_sets::expected(root, true)
            .into_iter()
            .filter(|(id, _)| id == &check.id)
            .map(|(_, path)| path)
            .collect::<Vec<_>>();
        let mut actual = reads
            .unconfirmed_reads
            .iter()
            .map(|path| absolute(root, path))
            .collect::<Vec<_>>();
        actual.sort_by(|left, right| left.components().cmp(right.components()));
        expected.sort_by(|left, right| left.components().cmp(right.components()));
        assert_eq!(actual, expected, "{}", check.id);
        quote_assertions::assert_no_quote_only_unowned(check, &d, root);
    }
    assert!(d.unconfirmed.contains(&(
        "AC-DL-001".into(),
        root.join(".archon").join("trading-lab").join("data")
    )));
    for task in ["TASK-TDL-012", "TASK-TDL-013"] {
        assert!(d.membership_only.iter().any(|id| id == task));
    }
    assert!(!acceptance_criteria(include_str!("../../tests/fixtures/r18_plan/prd.md")).is_empty());
}

#[path = "workflow_acceptance_link_diagnostics_quote_tests.rs"]
mod quote_assertions;

#[test]
fn r18_plan_bench_real_base() {
    let s: TaskSkeleton = serde_json::from_str(include_str!(
        "../../tests/fixtures/r18_plan/task-skeleton.json"
    ))
    .unwrap();
    let c: AcceptanceContract = serde_json::from_str(include_str!(
        "../../tests/fixtures/r18_plan/acceptance-contract.json"
    ))
    .unwrap();
    // Generated from 6cd0a28bd's recursive path inventory, filtered to reads.
    // Empty content models presence only; no acceptance commands are executed.
    let paths: Vec<_> = include_str!("../../tests/fixtures/r18_plan/base-paths.txt")
        .lines()
        .collect();
    assert_eq!(paths.len(), 116);
    assert!(paths.windows(2).all(|pair| pair[0] < pair[1]));
    let files: Vec<_> = paths.iter().map(|path| (*path, "")).collect();
    let repo = repository_with(&files);
    let root = repo.path();
    let mut discovered = BTreeSet::new();
    for check in c.acceptance.iter().chain(&c.supplementary) {
        let reads = input_discovery_for_check(check, root, root, None);
        discovered.extend(
            reads
                .candidates
                .into_iter()
                .map(|candidate| PathBuf::from(candidate.path)),
        );
        discovered.extend(reads.unshaped_missing.into_iter().map(PathBuf::from));
        discovered.extend(reads.unconfirmed_reads.into_iter().map(PathBuf::from));
    }
    for path in &paths {
        assert!(
            discovered.contains(Path::new(path)),
            "base fixture is not a read: {path}"
        );
        assert_eq!(std::fs::metadata(root.join(path)).unwrap().len(), 0);
    }
    let d = discover(&s, &c, root, Some(root), root);
    assert!(d.base_unavailable.is_none(), "{d:?}");
    assert!(d.outside.is_empty(), "{d:?}");
    let mut unowned = d.unowned.clone();
    read_sets::sort(&mut unowned);
    // One dead file operand and three occurrences of a test-target operand.
    let expected = [
        ("AC-DL-006", "validation_report/emit_revalidate"),
        ("SUP-REQ-AHDM-021", "prds/PRD-TRADING-DATA-LAKE-AHDM-001.md"),
        ("SUP-REQ-DL-056", "validation_report/emit_revalidate"),
        ("SUP-REQ-DL-130", "validation_report/emit_revalidate"),
    ]
    .map(|(check, path)| (check.to_string(), absolute(root, path)));
    assert_eq!(unowned, expected);
    let mut unconfirmed = d.unconfirmed.clone();
    read_sets::sort(&mut unconfirmed);
    assert_eq!(unconfirmed, read_sets::expected(root, true));
    for check in c.acceptance.iter().chain(&c.supplementary) {
        quote_assertions::assert_no_quote_only_unowned(check, &d, root);
    }
    println!(
        "R18 real-base read sets: {} unowned, {} unconfirmed",
        unowned.len(),
        unconfirmed.len()
    );
}
