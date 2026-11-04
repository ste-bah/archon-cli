#![cfg(unix)]

use super::*;

fn rejection(
    command: &str,
    output: &str,
    plugin: bool,
) -> Option<(String, super::super::UnprovenCause)> {
    let tool = "#!/bin/sh\nprintf 'Commands:\n    test    builtin\n    run    builtin\n'\n";
    let mut tools = vec![("make", tool)];
    if plugin {
        tools.push(("cargo-nextest", "#!/bin/sh\nexit 0\n"));
    }
    let bin = Bin::new(Path::new("/bin/sh"), &tools);
    std::fs::remove_file(bin.dir.path().join("cargo")).unwrap();
    let cargo = bin.dir.path().join("cargo");
    std::fs::write(&cargo, tool).unwrap();
    std::fs::set_permissions(cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    super::super::missing(
        &super::super::simple_commands(command),
        output,
        &bin.context(),
        &[],
    )
}
#[cfg(unix)]
#[test]
#[allow(non_snake_case)]
fn R2_nextest_separator_without_plugin_gives_no_proof() {
    assert!(
        rejection(
            "cargo nextest run -- --exact x",
            "error: no such command: `nextest`",
            false
        )
        .is_some()
    );
}
#[cfg(unix)]
#[test]
#[allow(non_snake_case)]
fn R2_nextest_separator_with_plugin_is_product() {
    assert!(
        rejection(
            "cargo nextest run -- --exact x",
            "error: no such command: `nextest`",
            true
        )
        .is_none()
    );
}
#[cfg(unix)]
#[test]
#[allow(non_snake_case)]
fn R2_cargo_run_separator_is_product() {
    assert!(
        rejection(
            "cargo run -- x",
            "error: unrecognized subcommand 'run'",
            false
        )
        .is_none()
    );
}
#[cfg(unix)]
#[test]
#[allow(non_snake_case)]
fn R2_make_test_separator_does_not_prove_product_ran() {
    assert!(
        rejection(
            "make test -- x",
            "error: unrecognized subcommand 'test'",
            false
        )
        .is_some()
    );
}
#[cfg(unix)]
#[test]
#[allow(non_snake_case)]
fn R5_D9_address_is_not_an_add_instruction() {
    let bin = Bin::new(
        Path::new("/bin/sh"),
        &[(
            "tool",
            "#!/bin/sh\nprintf 'Commands:\n    run    builtin\n'\n",
        )],
    );
    let (why, _) = super::super::missing(
        &super::super::simple_commands("tool status"),
        "tool: unknown command 'status'",
        &bin.context(),
        &[
            "Address the status line".into(),
            "The report provides a status line".into(),
        ],
    )
    .unwrap();
    assert!(why.contains("replace `tool status`"));
    assert!(!why.contains("grep -qw"));
}

#[cfg(unix)]
#[test]
#[allow(non_snake_case)]
fn B4_unavailable_plugin_has_the_same_remedy_at_run_start() {
    // Input: missing plugin at both verdict and run start -> same truthful remedy, no guard.
    let bin = Bin::new(
        Path::new("/bin/sh"),
        &[(
            "tool",
            "#!/bin/sh\nprintf 'Commands:\n    run    builtin\n'\n",
        )],
    );
    assert!(super::super::list::last_listing(&bin.dir.path().join("tool")).is_none());
    let command = "tool status";
    let warning = bin.warned(command).join(";");
    let (why, _) = super::super::missing(
        &super::super::simple_commands(command),
        "tool: unknown command 'status'",
        &bin.context(),
        &[],
    )
    .unwrap();
    let remedy = super::super::remedy::unavailable("tool", "status");
    assert!(why.contains(&remedy), "{why}");
    assert!(warning.contains(&remedy), "{warning}");
    assert!(!warning.contains("grep -qw"));
}
#[cfg(unix)]
#[test]
#[allow(non_snake_case)]
fn D9_addition_in_another_clause_does_not_offer_guard() {
    // Input: unrelated add clause and command-name mention -> no addition guard.
    assert!(
        !super::super::remedy::absent_remedy("tool", "X", &["add logging; X must exist".into()])
            .contains("grep -qw")
    );
    assert!(
        super::super::remedy::absent_remedy("tool", "X", &["add X".into()]).contains("grep -qw")
    );
}

#[test]
#[allow(non_snake_case)]
fn R8_2_dispatch_evidence_is_shared_by_verdict_and_warning() {
    // Input: unknown command of a tool with an installed sibling plugin.
    // Expected: verdict and run-start both name the unavailable candidate.
    let bin = Bin::new(
        Path::new("/bin/sh"),
        &[
            (
                "tool",
                "#!/bin/sh\nprintf 'Commands:\n    run    builtin\n'\n",
            ),
            ("tool-other", "#!/bin/sh\nexit 0\n"),
        ],
    );
    let remedy = super::super::remedy::unavailable("tool", "status");
    assert!(bin.warned("tool status").join(";").contains(&remedy));
    let (why, _) = super::super::missing(
        &super::super::simple_commands("tool status"),
        "tool: unknown command 'status'",
        &bin.context(),
        &[],
    )
    .unwrap();
    assert!(why.contains(&remedy), "{why}");
}

#[test]
#[allow(non_snake_case)]
fn S5_fresh_run_start_computes_listing_without_plugin_programs() {
    // Input: fresh listing-capable tool, no tool-* programs, no prior verdict.
    // Expected: built-in is available; missing command gets the shared warning.
    let bin = Bin::new(
        Path::new("/bin/sh"),
        &[(
            "tool",
            "#!/bin/sh\nprintf 'Commands:\n    inspect    builtin\n'\n",
        )],
    );
    assert!(super::super::list::last_listing(&bin.dir.path().join("tool")).is_none());
    let remedy = super::super::remedy::unavailable("tool", "status");
    assert!(bin.warned("tool status").join(";").contains(&remedy));
    assert!(bin.warned("tool inspect").is_empty());
    let (why, _) = super::super::missing(
        &super::super::simple_commands("tool status"),
        "tool: unknown command 'status'",
        &bin.context(),
        &[],
    )
    .unwrap();
    assert!(why.contains(&remedy));
}

#[test]
#[allow(non_snake_case)]
fn N5_documented_missing_plugin_remedy_matches_the_verdict() {
    // Input: unavailable plugin without an addition requirement -> truthful B4 remedy.
    let remedy = super::super::remedy::unavailable("tool", "word");
    let expected = "the check must use an available program or the deliverable must add it";
    assert!(remedy.contains(expected));
    let docs = include_str!("workflow_acceptance_executability_verdict_subcommand.rs");
    let row = docs
        .lines()
        .find(|line| line.starts_with("/// | lists commands without `word` | no | no"))
        .unwrap();
    assert!(row.contains(expected), "{row}");
    assert!(!row.contains("passes only if"));
}
