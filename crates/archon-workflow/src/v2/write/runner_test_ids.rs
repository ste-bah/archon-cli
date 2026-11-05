//! Stable per-test verdict adapters. Unknown output supplies no invented ids.
//! TAP supplies an ecosystem-neutral interchange form.

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Status {
    Passed,
    Failed,
    Skipped,
}

pub(super) fn ids(output: &str, status: Status) -> Vec<String> {
    let mut ids = Vec::new();
    let mut suite = String::new();
    let mut go_ids = Vec::new();
    for raw in output.lines() {
        let line = raw.trim();
        if line.starts_with("PASS ") || line.starts_with("FAIL ") {
            suite = line[5..].trim().to_string();
        }
        let mut summary = line.split_whitespace();
        let timed = line.split_whitespace().nth(2).is_some_and(|word| {
            word == "(cached)"
                || word
                    .strip_suffix('s')
                    .is_some_and(|n| n.parse::<f64>().is_ok())
        });
        if (!go_ids.is_empty() || timed)
            && matches!(summary.next(), Some("ok" | "FAIL"))
            && let Some(package) = summary
                .next()
                .filter(|word| !word.chars().all(|ch| ch.is_ascii_digit()))
        {
            for index in go_ids.drain(..) {
                ids[index] = format!("{package}::{}", ids[index]);
            }
            continue;
        }
        if let Some((id, found)) = result(line, &suite)
            && found == status
            && !ids.contains(&id)
        {
            if line.starts_with("--- ") {
                go_ids.push(ids.len());
            }
            ids.push(id);
        }
    }
    ids.sort();
    ids
}

fn result(line: &str, suite: &str) -> Option<(String, Status)> {
    use Status::*;
    // pytest verbose output: path::Class::test[param] STATUS [ progress ].
    let mut words = line.split_whitespace();
    if let (Some(id), Some(verdict)) = (words.next(), words.next())
        && id.contains("::")
        && id.split("::").next()?.ends_with(".py")
    {
        let status = match verdict {
            "PASSED" | "XPASS" => Passed,
            "FAILED" | "ERROR" => Failed,
            "SKIPPED" | "XFAIL" => Skipped,
            _ => return None,
        };
        return Some((id.into(), status));
    }
    // Go verbose verdicts include nested test names.
    for (prefix, status) in [
        ("--- PASS: ", Passed),
        ("--- FAIL: ", Failed),
        ("--- SKIP: ", Skipped),
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some((rest.split_whitespace().next()?.into(), status));
        }
    }
    // unittest verbose verdicts.
    if let Some((id, verdict)) = line.rsplit_once(" ... ") {
        let status = match verdict {
            "ok" => Passed,
            "FAIL" | "ERROR" => Failed,
            _ if verdict.starts_with("skipped ") => Skipped,
            _ => return None,
        };
        if id.contains(" (") && id.ends_with(')') {
            return Some((id.into(), status));
        }
    }
    // TAP is an ecosystem-neutral interchange protocol. Use the description,
    // never the ordinal, which shifts when a test is removed.
    let (rest, status) = if let Some(rest) = line.strip_prefix("not ok ") {
        (rest, Failed)
    } else if let Some(rest) = line.strip_prefix("ok ") {
        (rest, Passed)
    } else {
        return jest(line, suite);
    };
    let rest = rest
        .trim_start_matches(|ch: char| ch.is_ascii_digit())
        .trim();
    let rest = rest.strip_prefix('-').unwrap_or(rest).trim();
    let (id, directive) = rest.split_once(" # ").unwrap_or((rest, ""));
    if id.is_empty() {
        return None;
    }
    let status = if directive.to_ascii_lowercase().starts_with("skip")
        || directive.to_ascii_lowercase().starts_with("todo")
    {
        Skipped
    } else {
        status
    };
    Some((id.into(), status))
}

fn jest(line: &str, suite: &str) -> Option<(String, Status)> {
    let (status, rest) =
        if let Some(rest) = line.strip_prefix('✓').or_else(|| line.strip_prefix('✔')) {
            (Status::Passed, rest)
        } else if let Some(rest) = line.strip_prefix('✕').or_else(|| line.strip_prefix('✖')) {
            (Status::Failed, rest)
        } else if let Some(rest) = line.strip_prefix("○ skipped ") {
            (Status::Skipped, rest)
        } else {
            return None;
        };
    let id = rest
        .trim()
        .rsplit_once(" (")
        .map_or(rest.trim(), |(id, _)| id);
    Some((format!("{suite}::{id}"), status))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_names_and_skip_status_across_runner_formats() {
        for (before, after, id) in [
            (
                "tests/test_a.py::test_one PASSED [50%]\ntests/test_a.py::test_two PASSED [100%]",
                "tests/test_a.py::test_one PASSED [100%]",
                "tests/test_a.py::test_two",
            ),
            (
                "--- PASS: TestOne (0.00s)\n--- PASS: TestTwo (0.00s)",
                "--- PASS: TestOne (0.00s)",
                "TestTwo",
            ),
            ("ok 1 - first\nok 2 - second", "ok 1 - first", "second"),
            (
                "PASS tests/a.test.js\n✓ first (1 ms)\n✓ second (1 ms)",
                "PASS tests/a.test.js\n✓ first (1 ms)",
                "tests/a.test.js::second",
            ),
            (
                "test_one (a.Suite) ... ok\ntest_two (a.Suite) ... ok",
                "test_one (a.Suite) ... ok",
                "test_two (a.Suite)",
            ),
        ] {
            assert!(ids(before, Status::Passed).contains(&id.into()));
            assert!(!ids(after, Status::Passed).contains(&id.into()));
        }
        assert_eq!(
            ids("ok 2 - second # SKIP unavailable", Status::Skipped),
            ["second"]
        );
        assert_eq!(
            ids("tests/test_a.py::test_two SKIPPED [100%]", Status::Skipped),
            ["tests/test_a.py::test_two"]
        );
        assert!(ids("we mentioned a passed test", Status::Passed).is_empty());
    }
}

#[cfg(test)]
mod package_tests {
    use super::*;
    #[test]
    fn same_test_name_in_two_packages_has_distinct_identity() {
        let output = "--- PASS: TestExample (0.00s)\nok example/one 0.01s\n--- PASS: TestExample (0.00s)\nok example/two 0.01s";
        assert_eq!(
            ids(output, Status::Passed),
            ["example/one::TestExample", "example/two::TestExample"]
        );
    }
}
