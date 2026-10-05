//! Issue 333 round 5: a listing runs only with its site's own context,
//! never the host's environment (Issue 282), and originals are judged at
//! their site; each cell of `missing`'s table says what its author does;
//! a listing is "none" only when the tool says it has none; and a
//! rejection is the tool's only when no other program of the check is
//! named instead.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use archon_workflow::acceptance_scratch::ScratchPolicy;

use super::super::super::{HostProbe, Original};
use super::super::no_verdict;
use super::tests::{Bin, SIBLING, host_cargo, result};
use super::tests_333::{rustup_proxy, warm};
use super::tests_333b::contract;

macro_rules! or_skip {
    ($found:expr, $what:literal) => {
        match $found {
            Some(found) => found,
            None => {
                eprintln!(concat!("skipped: no ", $what, " on the host"));
                return;
            }
        }
    };
}

const PLAIN: &str = "#!/bin/sh\nexit 0\n";
const SECRET: &str = "ARCHON_333_FAKE_SECRET";
const MUST_NOT: &str = "must not depend on it";
const GUARD: &str = "--list | grep -qw";

/// A tool at an absolute path whose `--list` writes its environment to
/// `record` and lists `build`; it rejects every other word.
fn spy(dir: &Path, record: &Path) -> PathBuf {
    let tool = dir.join("archon333spy");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = --list ]; then /usr/bin/env > '{}'; printf 'Commands:\\n    build    Build\\n'; exit 0; fi\necho \"archon333spy: '$1' is not a archon333spy command\" >&2\nexit 2\n",
        record.display()
    );
    std::fs::write(&tool, script).unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    tool
}

fn scratch(
    repository: &Path,
    toolchain_path: &str,
) -> crate::command::acceptance_scratch_policy::NativeBinding {
    crate::command::acceptance_scratch_policy::NativeBinding {
        policy: ScratchPolicy {
            repository: repository.to_path_buf(),
            project: repository.to_path_buf(),
            task_root: repository.to_path_buf(),
            scratch_parent: repository.to_path_buf(),
            project_inputs: Vec::new(),
            project_input_excludes: Vec::new(),
            combined: false,
            toolchain_path: toolchain_path.to_string(),
            environment: Default::default(),
            environment_allowlist: Vec::new(),
            cargo_seed: None,
            timeout_secs: 60,
            output_bytes: 4096,
            scratch_bytes: 1 << 20,
            build_cache: None,
        },
        source_commit: "0".repeat(40),
        external_data_roots: Vec::new(),
    }
}

#[tokio::test]
async fn a_listing_never_sees_the_hosts_environment() {
    // SAFETY: a variable of this test's own; no other test reads it.
    unsafe { std::env::set_var(SECRET, "s3cr3t-333") };
    let dir = tempfile::tempdir().unwrap();
    let operator_home = std::env::var("HOME").unwrap_or_default();
    for (label, binding) in [
        ("scratch", Some(scratch(dir.path(), "/usr/bin:/bin"))),
        ("hermetic", None),
    ] {
        let record = dir.path().join(format!("{label}.env"));
        let tool = spy(dir.path(), &record);
        let check = contract(&format!("{} lint", tool.display()));
        let probe = HostProbe::at(dir.path().into(), dir.path().into(), binding);
        let mut odd = result(
            Some(2),
            b"",
            b"archon333spy: 'lint' is not a archon333spy command\n",
        );
        odd.acceptance_id = "AC-333".into();
        // The originals a repair holds: listed at the site, never the host.
        let held =
            super::super::super::baseline::originals(&probe.check_site(&check), &check, vec![odd])
                .await;
        assert_eq!(held["AC-333"], Original::Defect, "{label}");
        let seen = std::fs::read_to_string(&record).expect("it was listed");
        assert!(
            !seen.contains(SECRET) && !seen.contains("s3cr3t-333"),
            "{label}: {seen}"
        );
        let home = (seen.lines())
            .find_map(|line| line.strip_prefix("HOME="))
            .unwrap();
        assert!(
            home != operator_home && home.contains("archon-command-list-"),
            "{label}: {home}"
        );
        if label == "scratch" {
            assert!(
                seen.lines().any(|line| line == "PATH=/usr/bin:/bin"),
                "{seen}"
            );
        }
    }
}

#[tokio::test]
async fn originals_are_judged_on_their_sites_search_path_never_the_hosts() {
    let dir = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    // `ls` is on the host's path, not the site's: the site could not start
    // it, which is no verdict there, not a failed assertion.
    let site = empty.path().to_string_lossy().into_owned();
    let probe = HostProbe::at(
        dir.path().into(),
        dir.path().into(),
        Some(scratch(dir.path(), &site)),
    );
    let check = contract("ls -la missing-file");
    let mut run = result(Some(127), b"", b"sh: ls: command not found\n");
    run.acceptance_id = "AC-333".into();
    let held =
        super::super::super::baseline::originals(&probe.check_site(&check), &check, vec![run])
            .await;
    assert_eq!(held["AC-333"], Original::Defect);
}

/// A site with `tool` (`script`) and `tool-other`, and the host with
/// `tool-word` when `host` is set.
fn site(tool: &str, script: &str, host: bool) -> Bin {
    let other = format!("{tool}-other");
    let installed = format!("{tool}-lint");
    let host: &[(&str, &str)] = if host { &[(&installed, PLAIN)] } else { &[] };
    let bin = Bin::with_host(
        Path::new("/bin/sh"),
        &[(tool, script), (&other, PLAIN)],
        host,
    );
    warm(&bin, &[tool]);
    bin
}

fn script(tool: &str, listing: &str) -> String {
    format!(
        "#!/bin/sh\nif [ \"$1\" = --list ]; then {listing}; fi\necho \"{tool}: unknown command '$1'\" >&2\nexit 2\n"
    )
}

#[test]
fn each_cell_tells_its_author_what_to_do() {
    let lists = "printf 'Commands:\\n    build    Build\\n'; exit 0";
    let lists_lint = "printf 'Commands:\\n    lint    Lint\\n'; exit 0";
    let rejects_list = "echo 'unknown option: --list' >&2; exit 129";
    let fails = "exit 1";
    // (tool, listing, host has tool-lint, no verdict and guard expected)
    let cells: [(&str, &str, bool, Option<bool>); 8] = [
        ("archon333ca", lists, true, Some(false)),
        ("archon333cb", rejects_list, true, Some(false)),
        ("archon333cc", fails, true, Some(false)),
        ("archon333cd", fails, false, Some(false)),
        ("archon333ce", lists, false, Some(true)),
        ("archon333cf", lists_lint, false, Some(false)),
        ("archon333cg", rejects_list, false, None),
        ("archon333ch", lists_lint, true, Some(false)),
    ];
    for (tool, listing, host, expected) in cells {
        let bin = site(tool, &script(tool, listing), host);
        let text = format!("{tool} lint");
        let run = bin.run(&text);
        let why = no_verdict(&text, &run, &bin.context());
        match expected {
            None => assert_eq!(why, None, "{tool}: a verdict (Issue 328)"),
            Some(guard) => {
                let why = why.unwrap_or_else(|| panic!("{tool}: never a proof"));
                assert_eq!(why.contains(GUARD), guard, "{tool}: {why}");
                assert_eq!(why.contains(MUST_NOT), !guard, "{tool}: {why}");
            }
        }
        // The run start says the same, where it names the command at all.
        let warned = bin.warned(&text);
        if let (Some(guard), [warned]) = (expected, warned.as_slice()) {
            assert_eq!(warned.contains(GUARD), guard, "{tool}: {warned}");
            assert_eq!(warned.contains(MUST_NOT), !guard, "{tool}: {warned}");
        }
    }
}

#[test]
fn a_listing_is_none_only_when_the_tool_says_so() {
    // Commands, then a failure: no answer, so never the verdict a tool
    // with no listing gets (Issue 328).
    let printed = "printf 'Commands:\\n    build    Build\\n'; exit 1";
    let bin = site("archon333half", &script("archon333half", printed), false);
    let run = bin.run("archon333half lint");
    let why = no_verdict("archon333half lint", &run, &bin.context()).expect("never a proof");
    assert!(
        why.contains("could not tell") && why.contains("exited 1"),
        "{why}"
    );
    // A clap tool that rejects `--list` as a subcommand has no listing.
    let clap = "echo \"error: unrecognized subcommand '--list'\" >&2; exit 2";
    let bin = site("archon333clap", &script("archon333clap", clap), false);
    let run = bin.run("archon333clap lint");
    assert_eq!(no_verdict("archon333clap lint", &run, &bin.context()), None);
}

#[test]
fn a_toolchain_proxy_that_could_not_choose_a_toolchain_is_no_answer() {
    let (proxy, _) = or_skip!(rustup_proxy(), "rustup");
    // No `cargo-nextest` on the host: only the listing could have said.
    let bin = Bin::new(&proxy, &[SIBLING]);
    let empty = tempfile::tempdir().unwrap();
    let at = bin.context_with(&[("RUSTUP_HOME", empty.path().to_str().unwrap())]);
    let odd = result(Some(101), b"", b"error: no such command: `nextest`\n");
    let why = no_verdict("cargo nextest run", &odd, &at).expect("never a proof");
    assert!(
        why.contains("could not tell") && why.contains("rustup"),
        "{why}"
    );
}

#[test]
fn a_rejection_another_program_printed_is_not_the_tools() {
    let cli = "#!/bin/sh\n[ \"$1\" = --list ] && exit 0\necho \"archon333cli: unknown command '$1'\" >&2\nexit 1\n";
    let bin = Bin::new(
        &or_skip!(host_cargo(), "cargo"),
        &[SIBLING, ("archon333cli", cli)],
    );
    warm(&bin, &["archon333cli"]);
    let text = "cargo build && archon333cli build";
    let odd = result(Some(1), b"", b"archon333cli: unknown command 'build'\n");
    // `build` is cargo's; the rejection names archon333cli, whose own
    // command the deliverable adds: a verdict.
    assert_eq!(no_verdict(text, &odd, &bin.context()), None);
    // Cargo's own rejection of a word it lists names cargo: no verdict.
    let own =
        b"error: no such command: `build`\n\n\tView all installed commands with `cargo --list`\n";
    let own = result(Some(101), b"", own);
    let why = no_verdict("cargo build", &own, &bin.context()).expect("never a proof");
    assert!(
        why.contains("that site lacks it") && why.contains(MUST_NOT),
        "{why}"
    );
}

#[test]
fn nothing_a_listing_runs_with_is_read_from_the_hosts_environment() {
    // The hard rule: a listing's variables come only from its site's
    // context (`Context::new` takes them); these never read the host's.
    for (file, text) in [
        (
            "list",
            include_str!("workflow_acceptance_executability_verdict_subcommand_list.rs"),
        ),
        (
            "verdict",
            include_str!("workflow_acceptance_executability_verdict.rs"),
        ),
    ] {
        assert!(
            !text.contains("env::vars"),
            "{file} reads the host's environment"
        );
    }
}
