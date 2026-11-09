/// `KNOWN_RUNNERS` listed only language toolchains, so a task whose PRD requires
/// portable checks had every bullet classified as prose. The task universe then
/// recorded no focused tests, the v3 author brief said "no task declares any
/// focused test", and the authored workflow passed no focusedTests at all —
/// observed on run wf-eb79b47d, whose own residual gap named the brief line.
#[test]
fn posix_checks_are_declared_focused_tests_not_prose() {
    let body = "\
# TASK-X-010

## Focused Tests

- `test -s src/alpha.txt`
- `grep -qx 'alpha ready' src/alpha.txt`
- `! grep -q 'CANARY' src/alpha.txt`
";
    let commands: Vec<String> = collected(body, "")
        .into_iter()
        .filter_map(|entry| match entry {
            super::super::FocusedTestEntry::Command(command) => Some(command),
            super::super::FocusedTestEntry::Prose(_) => None,
        })
        .collect();
    assert_eq!(
        commands.len(),
        3,
        "every portable check must be recorded as a command: {commands:?}"
    );
    assert!(
        commands.iter().any(|c| c.starts_with("test ")),
        "{commands:?}"
    );
}

/// An unavailable program remains prose even inside a code span.
#[test]
fn an_unavailable_cli_fragment_in_prose_is_not_a_command() {
    let body = "\
# TASK-X-010

## Focused Tests

- the operator runs `data list --json` and eyeballs it
";
    let commands: Vec<String> = collected(body, "")
        .into_iter()
        .filter_map(|entry| match entry {
            super::super::FocusedTestEntry::Command(command) => Some(command),
            super::super::FocusedTestEntry::Prose(_) => None,
        })
        .collect();
    assert!(commands.is_empty(), "{commands:?}");
}

/// Diagnostic: parse a real task file and print what it declares.
///
/// ARCHON_TASK_FILE=<path> cargo test -p archon-knowledge --lib \
///   focused_tests_declared_by_a_real_task_file -- --ignored --nocapture
#[test]
#[ignore = "diagnostic; requires ARCHON_TASK_FILE"]
fn focused_tests_declared_by_a_real_task_file() {
    let path = std::env::var("ARCHON_TASK_FILE").expect("set ARCHON_TASK_FILE");
    let raw = std::fs::read_to_string(&path).expect("read task file");
    for entry in collected(&raw, &path) {
        match entry {
            super::super::FocusedTestEntry::Command(c) => println!("COMMAND: {c}"),
            super::super::FocusedTestEntry::Prose(p) => {
                println!("prose  : {}", &p[..p.len().min(70)])
            }
        }
    }
}

fn collected(raw: &str, source: &str) -> Vec<super::super::FocusedTestEntry> {
    let (site, path) = super::super::focused_commands::site_tests::command_site();
    let file = site.path().join("grep");
    std::fs::write(&file, "").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    super::super::collect_focused_tests(raw, &[], source, &path)
}
