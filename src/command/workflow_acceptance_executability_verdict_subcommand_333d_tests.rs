//! Issue 333 round 4: the listing never reads the check's tree, so it only
//! ever withholds a verdict; it runs on the site's own search path; a tool
//! that could not run or was killed is no answer, never remembered; a line
//! printed again is not progress; and a directory it could not remove is
//! reported.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use super::super::no_verdict;
use super::tests::{Bin, SIBLING, host_cargo, result};
use super::tests_333::{STALL, calls, counting_tool, warm};
use super::unresolved_on_path;

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
const NOWHERE: &str = "passes only if the deliverable adds it";

#[test]
fn a_tree_alias_the_listing_cannot_see_goes_back_to_its_author() {
    let bin = Bin::new(&or_skip!(host_cargo(), "cargo"), &[SIBLING]);
    let guard = "`cargo --list | grep -qw archon333tree && cargo archon333tree ...`";
    // The tree defines it, but the site rejected it: the listing, which
    // never reads the tree, cannot tell, so it is never a proof.
    let odd = result(Some(101), b"", b"error: no such command: `archon333tree`\n");
    let why = no_verdict("cargo archon333tree", &odd, &bin.context()).expect("never a proof");
    assert!(why.contains(NOWHERE) && why.contains(guard), "{why}");
    let warned = bin.warned("cargo archon333tree");
    assert!(
        warned.len() == 1 && warned[0].contains(NOWHERE) && warned[0].contains(guard),
        "{warned:?}"
    );
    // When the host has a program of that name, it is an environment tool,
    // which no guard makes passable: it must not be depended on.
    let installed = [("cargo-archon333tree", PLAIN)];
    let bin = Bin::with_host(&or_skip!(host_cargo(), "cargo"), &[SIBLING], &installed);
    let why = no_verdict("cargo archon333tree", &odd, &bin.context()).expect("never a proof");
    assert!(
        why.contains("environment lacks")
            && why.contains("must not depend")
            && !why.contains(guard),
        "{why}"
    );
    let warned = bin.warned("cargo archon333tree");
    assert!(
        warned.len() == 1 && warned[0].contains("must not depend") && !warned[0].contains(guard),
        "{warned:?}"
    );
}

/// A site whose only tool is `name`, `script`; nothing on the host.
fn alone(name: &str, script: &str) -> Bin {
    let bin = Bin::new(Path::new("/bin/sh"), &[(name, script)]);
    warm(&bin, &[name]);
    bin
}

#[test]
fn an_env_shebang_shim_is_listed_on_the_sites_search_path() {
    // As asdf, npm and pnpm shims start: their interpreter by name.
    let shim = "#!/usr/bin/env sh\n[ \"$1\" = --list ] && { printf 'Commands:\\n    build    Build\\n'; exit 0; }\necho \"archon333shim: '$1' is not a archon333shim command\" >&2\nexit 2\n";
    let bin = alone("archon333shim", shim);
    let run = bin.run("archon333shim lint");
    assert_eq!(run.exit_code, Some(2), "{run:?}");
    let why = no_verdict("archon333shim lint", &run, &bin.context()).expect("never a proof");
    assert!(
        why.contains(NOWHERE) && why.contains("not a command built into"),
        "it listed its commands: {why}"
    );
}

#[test]
fn a_tool_that_could_not_run_or_was_killed_is_no_answer_and_never_remembered() {
    let records = tempfile::tempdir().unwrap();
    for (name, listing, said) in [
        ("archon333gone", "exit 127", "could not run (exit 127)"),
        ("archon333noexec", "exit 126", "could not run (exit 126)"),
        ("archon333killed", "kill -9 $$", "was killed by signal 9"),
    ] {
        let calls_file = records.path().join(name);
        let bin = alone(name, &counting_tool(name, &calls_file, listing));
        let run = bin.run(&format!("{name} sub"));
        for asked in 1..=2 {
            let why = no_verdict(&format!("{name} sub"), &run, &bin.context())
                .unwrap_or_else(|| panic!("{name}: never a proof"));
            assert!(
                why.contains("could not tell") && why.contains(said),
                "{name}: {why}"
            );
            assert_eq!(calls(&calls_file), asked, "{name}: asked again");
        }
    }
}

/// A tool whose `--list` records its HOME in `record`, then runs
/// `listing`; the host has `{name}-sub`.
fn recording(name: &str, record: &Path, listing: &str) -> Bin {
    let calls = record.with_file_name("calls");
    let listing = format!("echo \"$HOME\" > '{}'; {listing}", record.display());
    let script = counting_tool(name, &calls, &listing);
    let plugin = format!("{name}-sub");
    let bin = Bin::with_host(
        Path::new("/bin/sh"),
        &[(name, &script)],
        &[(&plugin, PLAIN)],
    );
    warm(&bin, &[name]);
    bin
}

#[test]
fn a_line_printed_again_and_again_is_not_progress() {
    let records = tempfile::tempdir().unwrap();
    let record = records.path().join("home");
    // The same line for longer than the bound, then a listing.
    let listing = "for i in 1 2 3 4 5 6 7 8 9 10 11 12; do echo 'Waiting...'; /bin/sleep 0.25; done; printf 'Commands:\\n    sub    Built in\\n'; exit 0";
    let bin = recording("archon333again", &record, listing);
    let mut at = bin.context();
    at.list_stall = STALL;
    let warned = unresolved_on_path(&["archon333again sub"], &at);
    assert!(
        warned[0].len() == 1 && warned[0][0].contains("printed nothing new for 1500 ms"),
        "{warned:?}"
    );
}

#[test]
fn a_directory_the_listing_could_not_remove_is_reported() {
    let records = tempfile::tempdir().unwrap();
    let record = records.path().join("home");
    let listing = "mkdir \"$TMPDIR/locked\" && : > \"$TMPDIR/locked/file\" && chmod 500 \"$TMPDIR/locked\"; printf 'Commands:\\n    sub    Built in\\n'; exit 0";
    let bin = recording("archon333locked", &record, listing);
    let warned = bin.warned("archon333locked sub");
    let home = std::fs::read_to_string(&record).unwrap();
    let root = Path::new(home.trim()).parent().unwrap().to_path_buf();
    let locked = root.join("home/locked");
    if locked.exists() {
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let _ = std::fs::remove_dir_all(&root);
    assert!(!root.exists(), "the test leaves nothing behind");
    assert!(
        warned.len() == 1 && warned[0].contains("could not be removed"),
        "{warned:?}"
    );
}
