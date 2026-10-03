//! Guard for #233: in a debug build, building and parsing the full `archon`
//! command tree must fit in a fixed thread stack.
//!
//! The parse runs in a thread with an explicit stack size, so the result does
//! not depend on `RUST_MIN_STACK`. It runs in a child process (this test
//! binary, run again with a filter), because a stack overflow aborts the
//! process and would otherwise give no failure message.

use clap::Parser;

use super::Cli;

/// Measured on macOS arm64 (debug): the parse needs about 760 KiB.
const STACK_BUDGET: usize = 1024 * 1024;
const CHILD_ENV: &str = "ARCHON_CLI_STACK_BUDGET_CHILD";

fn parse_in_budget() {
    let worker = std::thread::Builder::new()
        .stack_size(STACK_BUDGET)
        .spawn(|| {
            // Every parse builds the whole tree. This path has the deepest
            // nesting: Commands -> WorkflowAction -> AuditAction.
            Cli::try_parse_from([
                "archon",
                "workflow",
                "audit",
                "extend-budget",
                "wf-guard",
                "--extra-refreshes",
                "2",
                "--reason",
                "stack budget guard",
            ])
            .map(|cli| cli.command.is_some())
        })
        .expect("spawn parse thread");
    let parsed = worker.join().expect("parse thread panicked");
    assert!(
        parsed.expect("guard argv must parse"),
        "no subcommand parsed"
    );
}

#[test]
fn full_cli_parse_fits_debug_stack_budget() {
    if std::env::var_os(CHILD_ENV).is_some() {
        parse_in_budget();
        return;
    }
    let module = module_path!().split_once("::").map_or("", |(_, rest)| rest);
    let name = format!("{module}::full_cli_parse_fits_debug_stack_budget");
    let output = std::process::Command::new(std::env::current_exe().expect("test binary path"))
        .args([name.as_str(), "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, "1")
        .output()
        .expect("run child test process");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "building and parsing the CLI did not fit in a {STACK_BUDGET}-byte thread stack (#233). \
         A clap derive frame got larger; give large struct variants their own `Args` struct. \
         Child status: {:?}\nstderr:\n{stderr}",
        output.status
    );
}
