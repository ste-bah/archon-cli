use super::*;
fn result(text: &str, code: Option<i32>) -> CheckResult {
    let mut r = super::super::deferred("check");
    r.operational_error = None;
    r.exit_code = code;
    r.stderr = text.as_bytes().to_vec();
    r
}
#[test]
fn diagnostic_shapes_and_unresolved_errors() {
    let resolve = |p: &str| (p != "12").then(|| (p.into(), LocatedClass::Mutated));
    for text in [
        "a:3:4: undefined",
        "[ERROR] /x/A:[5,10] cannot find symbol",
        "a(3,5): error missing",
        "a:3:5 - error missing",
        "a:3:4: error broken\r\nFAILED",
        "C:\\r\\a:5:2: undefined",
    ] {
        assert!(
            matches!(
                decide::decide(&result(text, Some(1)), 1, false, 2, &resolve, &|| None),
                RunOutcome::Unviable { .. }
            ),
            "{text}"
        );
    }
    for text in [
        "a:3:4: warning ignore",
        "12:30:45 run",
        "  File \"x\", line 2, in f\nValueError: broken",
        "AssertionError: false",
        "panic at a:3:4",
    ] {
        assert!(
            matches!(
                decide::decide(&result(text, Some(1)), 1, false, 2, &resolve, &|| None),
                RunOutcome::Killed { .. }
            ),
            "{text}"
        );
    }
    for text in [
        "a:3: error missing",
        "error: missing\n --> a:3:4",
        "  File \"a\", line 3\nSyntaxError: invalid",
    ] {
        let locations = decide::locations(text, &|_| None);
        assert!(!locations.is_empty(), "{text}");
        assert_eq!(locations[0].class, LocatedClass::Unresolved);
    }
    assert!(verdict::located_lines("./a.go:5:2: undefined: f").is_empty());
}
#[test]
fn status_timeout_and_zero_work_are_not_kills() {
    assert_eq!(
        decide::decide(&result("", None), 1, false, 2, &|_| None, &|| None),
        RunOutcome::NoExitStatus
    );
    let mut r = result("", None);
    r.operational_error = Some("stopped".into());
    assert_eq!(
        decide::decide(&r, 1, true, 2, &|_| None, &|| None),
        RunOutcome::TimedOut { wall_secs: 2 }
    );
    assert!(matches!(
        decide::decide(&r, 1, false, 2, &|_| None, &|| None),
        RunOutcome::Unrun { .. }
    ));
    let r = result(
        "running 0 tests\ntest result: ok. 0 passed; 0 failed",
        Some(0),
    );
    assert!(
        matches!(decide::decide(&r,1,false,2,&|_|None,&||None), RunOutcome::Unrun {code,..} if code == "zero_work")
    );
}
#[test]
fn captured_tool_outputs() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../tests/fixtures/breakage/captured.json")).unwrap();
    for skip in fixture["skips"].as_array().unwrap() {
        eprintln!("SKIP: {}", skip.as_str().unwrap());
    }
    let rows = fixture["rows"].as_array().unwrap();
    assert!(rows.len() >= 9);
    for language in ["rust", "go", "python"] {
        for kind in ["compile", "runtime", "assertion"] {
            assert!(
                rows.iter()
                    .any(|row| row["language"] == language && row["kind"] == kind)
            );
        }
    }
    for row in rows {
        assert!(row["exit_code"].as_i64().is_some_and(|code| code > 0));
        let class = match row["language"].as_str().unwrap() {
            "rust" => LocatedClass::Mutated,
            "go" => LocatedClass::CheckNamed,
            _ => LocatedClass::Elsewhere,
        };
        let mut r = result(
            row["stderr"].as_str().unwrap(),
            Some(row["exit_code"].as_i64().unwrap() as i32),
        );
        r.stdout = row["stdout"].as_str().unwrap().as_bytes().to_vec();
        let outcome = decide::decide(&r, 1, false, 2, &|p| Some((p.into(), class)), &|| None);
        if row["kind"] == "compile" {
            assert!(
                matches!(&outcome, RunOutcome::Unviable { located } if located.iter().all(|l| l.class == class)),
                "{}: {outcome:?}",
                row["language"]
            );
        } else {
            assert!(
                matches!(outcome, RunOutcome::Killed { .. }),
                "{} {}: {outcome:?}",
                row["language"],
                row["kind"]
            );
        }
    }
}
#[test]
fn syntax_gutters_and_runtime_frames_remain_distinct() {
    for class in ["SyntaxError", "IndentationError", "TabError"] {
        for prefix in ["", "E   "] {
            let text = format!("{prefix}  File \"a\", line 3\n{prefix}{class}: invalid");
            let locations =
                decide::locations(&text, &|p| Some((p.into(), LocatedClass::CheckNamed)));
            assert!(
                locations.iter().any(|l| l.path == "a" && l.line == 3),
                "{text}: {locations:?}"
            );
        }
        let text = format!("  File \"a\", line 3, in f\n{class}: runtime");
        assert!(
            decide::locations(&text, &|p| Some((p.into(), LocatedClass::CheckNamed))).is_empty()
        );
    }
    assert!(
        decide::locations("  File \"a\", line 3", &|p| Some((
            p.into(),
            LocatedClass::Mutated
        )))
        .iter()
        .any(|l| l.line == 3)
    );
}
#[cfg(unix)]
#[test]
fn stored_output_uses_bounded_public_stream_bytes() {
    let root = tempfile::tempdir().unwrap();
    let p = HostProbe::new(root.path().into(), root.path().into(), Site::Hermetic);
    let mut r = result("", Some(1));
    r.stdout = vec![0xab; 40 * 1024];
    r.stderr = vec![0xcd; 40 * 1024];
    r.keep_verdict_streams(None);
    r.stdout = vec![0xff; 40 * 1024];
    r.stderr = vec![0xfe; 40 * 1024];
    let bytes = store::output(&p, &r);
    let expected = [vec![0xff; 32 * 1024], vec![0xfe; 32 * 1024]].concat();
    assert_eq!(bytes, expected);
}
#[test]
fn unresolved_locations_keep_their_actual_line_numbers() {
    for path in ["l", "r", "File"] {
        let text = format!("  File \"{path}\", line 17\nSyntaxError: invalid");
        let locations = decide::locations(&text, &|_| None);
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].line, 17);
        assert_eq!(locations[0].class, LocatedClass::Unresolved);
    }
}

#[test]
fn explicit_generic_errors_remain_unviable_without_resolved_paths() {
    for text in [
        "a.ts:3:5 - error TS2304: missing",
        "[ERROR] /x/A.java:[5,10] cannot find symbol",
        "a:3:5: fatal broken",
    ] {
        let outcome = decide::decide(&result(text, Some(1)), 1, false, 2, &|_| None, &|| None);
        assert!(
            matches!(outcome, RunOutcome::Unviable { located } if located.iter().any(|l| l.class == LocatedClass::Unresolved)),
            "{text}"
        );
    }
}
#[test]
fn prefixed_advisory_locations_are_not_build_errors() {
    for prefix in ["WARNING", "INFO", "NOTE", "HINT", "HELP"] {
        for diagnostic in [
            "/x/A.java:[10,5] [deprecation] x",
            "a:3:5: error ignored",
            "a(3,5): error ignored",
        ] {
            let text = format!("[{prefix}] {diagnostic}");
            let outcome = decide::decide(
                &result(&text, Some(1)),
                1,
                false,
                2,
                &|p| Some((p.into(), LocatedClass::Elsewhere)),
                &|| None,
            );
            assert!(
                matches!(outcome, RunOutcome::Killed { .. }),
                "{text}: {outcome:?}"
            );
        }
    }
}
