//! Issue 333, on real runs of real tools: the run-start listing is asked
//! only of tools some check runs with a plugin the host has, once, all at
//! once, under a no-progress bound (1); a rustup proxy lists as the site
//! runs it (2); an option's value before the subcommand is no subcommand
//! (3); a subcommand no program anywhere provides, such as an alias the
//! deliverable adds, is a verdict (4).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::super::super::verdict_shell::simple_commands;
use super::super::no_verdict;
use super::tests::{Bin, NEXTEST, SIBLING, host_cargo, result};
use super::{candidates, unresolved_on_path};

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

pub(super) const PLAIN: &str = "#!/bin/sh\nexit 0\n";

/// A test's no-progress bound: long enough for a new script to start.
pub(super) const STALL: Duration = Duration::from_millis(1500);

/// A tool that dispatches `$1` to `tool-$1`, and whose `--list` records
/// each call in `calls`, then runs `listing` (shell text).
pub(super) fn counting_tool(name: &str, calls: &Path, listing: &str) -> String {
    format!(
        "#!/bin/sh\nif [ \"$1\" = --list ]; then echo x >> '{}'; {listing}; fi\necho \"{name}: '$1' is not a {name} command\" >&2\nexit 2\n",
        calls.display()
    )
}

/// Run each of `tools` once, so a new script's first start (which the
/// system may check at length) is not part of what a test times.
pub(super) fn warm(bin: &Bin, tools: &[&str]) {
    for tool in tools {
        bin.run(&format!("{tool} warm"));
    }
}

pub(super) fn calls(file: &Path) -> usize {
    std::fs::read_to_string(file).map_or(0, |text| text.lines().count())
}

// ---- Item 1: what the run start lists, how often, and for how long.

#[test]
fn a_hanging_listing_is_asked_once_for_every_check_and_command() {
    let calls_file = tempfile::NamedTempFile::new().unwrap();
    let tool = counting_tool("archon333hang", calls_file.path(), "exec /bin/sleep 30");
    let bin = Bin::with_host(
        Path::new("/bin/sh"),
        &[("archon333hang", &tool)],
        &[("archon333hang-sub", PLAIN)],
    );
    warm(&bin, &["archon333hang"]);
    let mut at = bin.context();
    at.list_stall = STALL;
    let texts = [
        "archon333hang sub",
        "archon333hang sub x && archon333hang sub y",
    ];
    let started = Instant::now();
    let warned = unresolved_on_path(&texts, &at);
    assert_eq!(calls(calls_file.path()), 1, "asked once: {warned:?}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    for found in &warned {
        assert!(
            found.len() == 1 && found[0].contains("printed nothing new for 1500 ms"),
            "{warned:?}"
        );
    }
}

#[test]
fn a_tool_with_no_dispatched_program_anywhere_is_never_listed() {
    let calls_file = tempfile::NamedTempFile::new().unwrap();
    let listing = "printf 'Commands:\\n    build    Build\\n'; exit 0";
    let tool = counting_tool("archon333count", calls_file.path(), listing);
    // No `archon333count-*` program on the check's path or the host's.
    let bin = Bin::new(Path::new("/bin/sh"), &[("archon333count", tool.as_str())]);
    let texts = [
        "archon333count status --short",
        "archon333count -v run",
        "archon333count --color never lint",
    ];
    let warned = unresolved_on_path(&texts, &bin.context());
    assert!(warned.iter().all(Vec::is_empty), "{warned:?}");
    assert_eq!(calls(calls_file.path()), 0, "never asked");
}

#[test]
fn a_listing_that_keeps_printing_is_not_cut_off() {
    let calls_file = tempfile::NamedTempFile::new().unwrap();
    // Silent for less than the bound at a time, but for longer in all.
    let listing = "for i in 1 2 3 4 5 6; do echo \"Loading $i...\"; /bin/sleep 0.5; done; printf 'Commands:\\n    build    Build\\n'; exit 0";
    let tool = counting_tool("archon333slow", calls_file.path(), listing);
    let bin = Bin::with_host(
        Path::new("/bin/sh"),
        &[("archon333slow", &tool)],
        &[
            ("archon333slow-lint", PLAIN),
            ("archon333slow-build", PLAIN),
        ],
    );
    warm(&bin, &["archon333slow"]);
    let mut at = bin.context();
    at.list_stall = STALL;
    let started = Instant::now();
    let warned = unresolved_on_path(&["archon333slow lint && archon333slow build"], &at);
    assert_eq!(
        warned[0].len(),
        1,
        "a listing that makes progress is read to its end: {warned:?}"
    );
    assert!(started.elapsed() > STALL, "{:?}", started.elapsed());
    assert!(
        warned[0][0].starts_with("`archon333slow lint` (not built into"),
        "{warned:?}"
    );
}

#[test]
fn distinct_tools_are_listed_at_once() {
    let (one, two) = (
        tempfile::NamedTempFile::new().unwrap(),
        tempfile::NamedTempFile::new().unwrap(),
    );
    let first = counting_tool("archon333a", one.path(), "exec /bin/sleep 30");
    let second = counting_tool("archon333b", two.path(), "exec /bin/sleep 30");
    let bin = Bin::with_host(
        Path::new("/bin/sh"),
        &[("archon333a", &first), ("archon333b", &second)],
        &[("archon333a-sub", PLAIN), ("archon333b-sub", PLAIN)],
    );
    warm(&bin, &["archon333a", "archon333b"]);
    let mut at = bin.context();
    at.list_stall = STALL;
    let started = Instant::now();
    let warned = unresolved_on_path(&["archon333a sub", "archon333b sub"], &at);
    let took = started.elapsed();
    assert!(
        took < 2 * STALL - Duration::from_millis(100),
        "not one after the other: {took:?}"
    );
    assert_eq!((calls(one.path()), calls(two.path())), (1, 1), "{warned:?}");
}

// ---- Item 2: a rustup proxy lists as the site runs it.

/// The host's rustup `cargo` proxy and its RUSTUP_HOME, if it has rustup.
pub(super) fn rustup_proxy() -> Option<(PathBuf, String)> {
    let path = std::env::var("PATH").ok()?;
    let proxy = std::env::split_paths(&path)
        .find(|dir| dir.join("rustup").is_file() && dir.join("cargo").is_file())?
        .join("cargo");
    let home = std::process::Command::new(proxy.with_file_name("rustup"))
        .args(["show", "home"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())?;
    Some((proxy, home))
}

#[test]
fn a_rustup_proxy_is_judged_with_the_sites_rustup_home() {
    let (proxy, home) = or_skip!(rustup_proxy(), "rustup");
    let bin = Bin::with_host(&proxy, &[SIBLING], &[NEXTEST]);
    let site = [("RUSTUP_HOME", home.as_str())];
    let run = bin.run_with("cargo nextest run", &site);
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(stderr.contains("no such command: `nextest`"), "{stderr}");
    let why = no_verdict("cargo nextest run", &run, &bin.context_with(&site))
        .expect("the proxy resolves the toolchain the site runs");
    assert!(why.contains("`cargo nextest`"), "{why}");
    let warned = unresolved_on_path(
        &["cargo nextest run && cargo test"],
        &bin.context_with(&site),
    );
    assert!(
        warned[0].len() == 1 && warned[0][0].starts_with("`cargo nextest` (not built into"),
        "{warned:?}"
    );
}

#[test]
fn the_operators_rustup_home_is_never_consulted() {
    let (proxy, _) = or_skip!(rustup_proxy(), "rustup");
    let bin = Bin::with_host(&proxy, &[SIBLING], &[NEXTEST]);
    // The site binds no RUSTUP_HOME: there the proxy finds no toolchain, so
    // the host says so rather than list from the operator's own home.
    let warned = bin.warned("cargo nextest run");
    assert!(
        warned.len() == 1 && warned[0].contains("could not tell") && warned[0].contains("rustup"),
        "{warned:?}"
    );
    // Nor is that a proof: a listing that could not choose a toolchain is
    // no answer.
    let odd = result(Some(101), b"", b"error: no such command: `nextest`\n");
    let why = no_verdict("cargo nextest", &odd, &bin.context()).expect("no verdict");
    assert!(why.contains("could not tell"), "{why}");
}

#[test]
fn the_sites_toolchain_choice_is_followed() {
    let (proxy, home) = or_skip!(rustup_proxy(), "rustup");
    let bin = Bin::with_host(&proxy, &[SIBLING], &[NEXTEST]);
    let site = [
        ("RUSTUP_HOME", home.as_str()),
        ("RUSTUP_TOOLCHAIN", "archon-333-none"),
    ];
    let warned = unresolved_on_path(&["cargo nextest run"], &bin.context_with(&site));
    assert!(
        warned[0].len() == 1 && warned[0][0].contains("archon-333-none"),
        "the site's own toolchain, never a default: {warned:?}"
    );
}

#[test]
fn a_site_that_runs_in_its_own_home_lists_what_that_home_installs() {
    let cargo = or_skip!(host_cargo(), "cargo");
    let home = tempfile::tempdir().unwrap();
    let installed = home.path().join(".cargo/bin");
    std::fs::create_dir_all(&installed).unwrap();
    let plugin = "#!/bin/sh\necho 'archon333home: the feature is absent' >&2\nexit 1\n";
    let script = installed.join("cargo-archon333home");
    std::fs::write(&script, plugin).unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let bin = Bin::with_host(&cargo, &[SIBLING], &[("cargo-archon333home", PLAIN)]);
    let rustup = rustup_proxy().map(|(_, rustup)| rustup);
    let mut site = vec![("HOME", home.path().to_str().unwrap())];
    site.extend(rustup.as_deref().map(|rustup| ("RUSTUP_HOME", rustup)));
    let at = bin.context_with(&site);
    assert!(
        unresolved_on_path(&["cargo archon333home"], &at)[0].is_empty(),
        "cargo finds it in the site's own home"
    );
    let odd = result(Some(101), b"", b"error: no such command: `archon333home`\n");
    assert_eq!(no_verdict("cargo archon333home", &odd, &at), None);
}

// ---- Item 3: an option's value before the subcommand.

#[test]
fn a_subcommand_after_an_options_value_is_judged() {
    let bin = Bin::with_host(&or_skip!(host_cargo(), "cargo"), &[SIBLING], &[NEXTEST]);
    for command in [
        "cargo --color never nextest run",
        "cargo --config net.offline=true nextest run",
        "cargo -v nextest run",
    ] {
        let run = bin.run(command);
        assert_eq!(run.exit_code, Some(101), "{command}: {run:?}");
        let why = no_verdict(command, &run, &bin.context()).unwrap_or_else(|| panic!("{command}"));
        assert!(why.contains("`cargo nextest`"), "{command}: {why}");
        let warned = bin.warned(command);
        assert!(
            warned.len() == 1 && warned[0].starts_with("`cargo nextest` (not built into"),
            "{command}: {warned:?}"
        );
    }
}

#[test]
fn an_options_value_before_a_built_in_keeps_its_verdict() {
    let bin = Bin::with_host(&or_skip!(host_cargo(), "cargo"), &[SIBLING], &[NEXTEST]);
    let command = "cargo --color never test -q";
    let run = bin.run(command);
    assert_eq!(run.exit_code, Some(101), "{run:?}");
    assert_eq!(no_verdict(command, &run, &bin.context()), None);
    assert!(bin.warned(command).is_empty());
    // A built-in that comes first is the subcommand: a later word is its.
    assert!(bin.warned("cargo -v test nextest").is_empty());
}

#[test]
fn the_words_that_may_be_the_subcommand() {
    let cases: [(&str, &[&str]); 7] = [
        ("tool sub x", &["sub"]),
        ("tool --color never sub x", &["never", "sub"]),
        ("tool --color=never sub x", &["sub"]),
        ("tool -v sub x", &["sub", "x"]),
        ("tool +nightly sub", &["sub"]),
        ("tool --config a.b=1 sub x", &["sub"]),
        ("tool -- sub", &[]),
    ];
    for (text, expected) in cases {
        let commands = simple_commands(text);
        assert_eq!(candidates(&commands[0]), expected, "{text}");
    }
}

// ---- Item 4: a subcommand the deliverable adds.

/// A subcommand no program anywhere provides goes back to its author,
/// until the check shows first that the tree provides it: then its failure
/// before the implementation is its own assertion, a proof.
#[test]
fn an_alias_the_deliverable_adds_counts_once_the_check_shows_it() {
    let cargo = or_skip!(host_cargo(), "cargo");
    let bin = Bin::new(&cargo, &[SIBLING]);
    let plain = "cargo archon333alias";
    let run = bin.run(plain);
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        stderr.contains("no such command: `archon333alias`"),
        "{stderr}"
    );
    let why = no_verdict(plain, &run, &bin.context()).expect("never a proof");
    assert!(
        why.contains("passes only if the deliverable adds it"),
        "{why}"
    );
    let warned = bin.warned(plain);
    assert!(
        warned.len() == 1 && warned[0].contains("passes only if the deliverable adds it"),
        "{warned:?}"
    );
    let shown = "cargo --list | grep -qw archon333alias && cargo archon333alias";
    let run = bin.run(shown);
    assert_eq!(run.exit_code, Some(1), "{run:?}");
    assert_eq!(no_verdict(shown, &run, &bin.context()), None, "a proof");
    // Once the tree defines it, both checks pass.
    let tree = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tree.path().join(".cargo")).unwrap();
    let config = "[alias]\narchon333alias = \"version\"\n";
    std::fs::write(tree.path().join(".cargo/config.toml"), config).unwrap();
    for command in [plain, shown] {
        let home = tempfile::tempdir().unwrap();
        let mut implemented = std::process::Command::new("/bin/sh");
        implemented
            .args(["-c", command])
            .current_dir(tree.path())
            .env_clear();
        implemented.env("PATH", bin.path()).env("HOME", home.path());
        if let Some((_, rustup)) = rustup_proxy() {
            implemented.env("RUSTUP_HOME", rustup);
        }
        let out = implemented.output().unwrap();
        assert!(
            out.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn the_same_subcommand_the_host_installs_is_its_environments() {
    let cargo = or_skip!(host_cargo(), "cargo");
    let bin = Bin::with_host(&cargo, &[SIBLING], &[("cargo-archon333alias", PLAIN)]);
    let run = bin.run("cargo archon333alias");
    let why = no_verdict("cargo archon333alias", &run, &bin.context()).expect("unproven");
    assert!(
        why.contains(
            &bin.host
                .path()
                .canonicalize()
                .unwrap()
                .display()
                .to_string()
        ),
        "{why}"
    );
    assert_eq!(bin.warned("cargo archon333alias").len(), 1);
}
