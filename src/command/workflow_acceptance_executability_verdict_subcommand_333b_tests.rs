//! Issue 333 round 2, on real runs of real tools: a subcommand no program
//! anywhere provides, and one the host has but its tool cannot list, give
//! no verdict (R1, R2); deciding never holds an async worker (R3); a
//! listing's whole process group dies (R4); a changed tool is asked again,
//! a stalled one is never remembered (R5); a listing reads the committed
//! configuration of the check's tree (R6).

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use archon_workflow::task_set_contract::AcceptanceContract;

use super::super::no_verdict;
use super::tests::{Bin, NEXTEST, SIBLING, host_cargo, result};
use super::tests_333::{PLAIN, STALL, calls, counting_tool, rustup_proxy, warm};
use super::{SiteTree, unresolved_on_path};

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

const NOWHERE: &str = "passes only if the deliverable adds it";

// ---- R1: a subcommand no program anywhere provides.

#[test]
fn a_plugin_installed_nowhere_is_never_a_proof() {
    let bin = Bin::new(&or_skip!(host_cargo(), "cargo"), &[SIBLING]);
    for (command, warned_as) in [
        (
            "cargo nextest run",
            "`cargo nextest` (not built into `cargo`",
        ),
        (
            "cargo --color never nextest run",
            "`cargo nextest` (whichever of `never`, `nextest` is its subcommand)",
        ),
        (
            "cargo nextest run --workspace && true",
            "`cargo nextest` (not built",
        ),
    ] {
        let run = bin.run(command);
        assert_eq!(run.exit_code, Some(101), "{command}: {run:?}");
        let why = no_verdict(command, &run, &bin.context()).unwrap_or_else(|| panic!("{command}"));
        assert!(
            why.contains("`cargo nextest`") && why.contains(NOWHERE),
            "{why}"
        );
        assert!(why.contains("grep -qw nextest"), "how to show it: {why}");
        let warned = bin.warned(command);
        assert!(
            warned.len() == 1 && warned[0].starts_with(warned_as) && warned[0].contains(NOWHERE),
            "{command}: {warned:?}"
        );
    }
    // A built-in that fails is still its check's verdict.
    let run = bin.run("cargo test -q");
    assert_eq!(no_verdict("cargo test -q", &run, &bin.context()), None);
    assert!(bin.warned("cargo test -q").is_empty());
}

// ---- R2: the host has the program, but the tool lists nothing.

/// A tool with no `--list`, like git: it rejects that as an option.
const UNLISTED: &str = "#!/bin/sh\ncase \"$1\" in\n--list) echo \"unknown option: $1\" >&2; exit 129 ;;\nbuild) exit 1 ;;\n*) echo \"archon333git: '$1' is not a archon333git command.\" >&2; exit 1 ;;\nesac\n";

#[test]
fn a_plugin_the_host_has_gives_no_verdict_whatever_the_listing_says() {
    let tools = [("archon333git", UNLISTED), ("archon333git-other", PLAIN)];
    let bin = Bin::with_host(Path::new("/bin/sh"), &tools, &[("archon333git-lfs", PLAIN)]);
    let run = bin.run("archon333git lfs pull");
    let why = no_verdict("archon333git lfs pull", &run, &bin.context()).expect("no verdict");
    assert!(
        why.contains("is not known") && why.contains("exited 129"),
        "{why}"
    );
    // A word the host has no program for keeps its verdict (Issue 328).
    let run = bin.run("archon333git status");
    assert_eq!(
        no_verdict("archon333git status", &run, &bin.context()),
        None
    );
    // A listing that names the word turns it back into a verdict.
    let lists = "#!/bin/sh\n[ \"$1\" = --list ] && { printf 'Commands:\\n    lfs    Built in\\n'; exit 0; }\nexit 1\n";
    let bin = Bin::with_host(
        Path::new("/bin/sh"),
        &[("archon333lister", lists)],
        &[("archon333lister-lfs", PLAIN)],
    );
    let odd = result(Some(1), b"", b"error: unknown command 'lfs'\n");
    assert_eq!(
        no_verdict("archon333lister lfs", &odd, &bin.context()),
        None
    );
}

#[test]
fn a_listing_with_no_answer_gives_no_verdict() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let hangs = counting_tool("archon333hung", file.path(), "exec /bin/sleep 30");
    let bin = Bin::new(Path::new("/bin/sh"), &[("archon333hung", &hangs)]);
    warm(&bin, &["archon333hung"]);
    let mut at = bin.context();
    at.list_stall = STALL;
    let run = bin.run("archon333hung lint");
    let why = no_verdict("archon333hung lint", &run, &at).expect("the host could not tell");
    assert!(
        why.contains("could not tell") && why.contains("printed nothing"),
        "{why}"
    );
}

// ---- R3: deciding a verdict never holds an async worker.

fn contract(command: &str) -> AcceptanceContract {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "prd": {"path": "p.md", "digest": "d"},
        "gap_policy": {"permitted_acceptance_ids": [], "forbidden_phrases": [], "required_fields": []},
        "acceptance": [{
            "id": "AC-333", "criterion": "c",
            "check": {"kind": "command", "command": command, "cwd": "repo_root"},
            "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": ""}
        }],
        "supplementary": []
    }))
    .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn the_verdict_path_lists_off_the_async_thread() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let slow = counting_tool(
        "archon333late",
        file.path(),
        "/bin/sleep 1; printf 'Commands:\\n    build    Build\\n'; exit 0",
    );
    let bin = Bin::new(
        Path::new("/bin/sh"),
        &[("archon333late", &slow), ("archon333late-x", PLAIN)],
    );
    warm(&bin, &["archon333late"]);
    // The host's own environment, for the originals a repair holds.
    super::HOST_PATH.with(|path| *path.borrow_mut() = Some(bin.host_path()));
    let ticks = Arc::new(AtomicUsize::new(0));
    let counter = ticks.clone();
    let ticker = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(10)).await;
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });
    let mut odd = result(
        Some(1),
        b"",
        b"archon333late: 'lint' is not a archon333late command\n",
    );
    odd.acceptance_id = "AC-333".into();
    let check = contract("archon333late lint");
    let silent = super::super::super::silent::silent_failure_off_thread;
    let why = silent(&check, &odd, &bin.context()).await;
    let during = ticks.load(Ordering::SeqCst);
    assert!(why.is_some_and(|why| why.contains(NOWHERE)));
    let held = super::super::super::baseline::originals(&check, vec![odd]).await;
    let after = ticks.load(Ordering::SeqCst) - during;
    ticker.abort();
    assert!(during >= 20, "the runtime ran while it listed: {during}");
    assert!(after >= 20, "and while the originals were listed: {after}");
    assert_eq!(held["AC-333"], super::super::super::Original::Defect);
    assert_eq!(calls(file.path()), 2, "each site lists it once");
}

#[test]
fn a_listing_that_never_stops_printing_is_stopped() {
    let bin = Bin::with_host(
        Path::new("/bin/sh"),
        &[(
            "archon333yes",
            "#!/bin/sh\n[ \"$1\" = --list ] && exec /usr/bin/yes\nexit 2\n",
        )],
        &[("archon333yes-sub", PLAIN)],
    );
    warm(&bin, &["archon333yes"]);
    let started = std::time::Instant::now();
    let warned = bin.warned("archon333yes sub");
    assert!(
        started.elapsed() < Duration::from_secs(9),
        "{:?}",
        started.elapsed()
    );
    assert!(
        warned.len() == 1 && warned[0].contains("output exceeded 1048576 bytes"),
        "{warned:?}"
    );
}

// ---- R4: a listing's whole process group dies.

fn gone(pid_file: &Path) -> bool {
    let pid: libc::pid_t = std::fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    (0..150).any(|_| {
        // SAFETY: signal 0 only asks whether the process exists.
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        std::thread::sleep(Duration::from_millis(20));
        !alive
    })
}

#[test]
fn every_process_a_listing_starts_dies_however_it_ends() {
    let pids = tempfile::tempdir().unwrap();
    let pid = |name: &str| pids.path().join(name);
    let cases = [
        // A wrapper that waits on a background child, and stalls.
        (
            "archon333wrap",
            format!(
                "/bin/sleep 30 & echo $! > '{}'; wait",
                pid("wrap").display()
            ),
            "wrap",
        ),
        // A wrapper whose foreground child does not exec, and stalls.
        (
            "archon333fore",
            format!(
                "/bin/sh -c 'echo $$ > {}; exec /bin/sleep 30'",
                pid("fore").display()
            ),
            "fore",
        ),
        // A tool that answers, leaving a child behind.
        (
            "archon333left",
            format!(
                "/bin/sleep 30 & echo $! > '{}'; printf 'Commands:\\n    build    Build\\n'; exit 0",
                pid("left").display()
            ),
            "left",
        ),
        // One that never stops printing, from a child.
        (
            "archon333loud",
            format!("/usr/bin/yes & echo $! > '{}'; wait", pid("loud").display()),
            "loud",
        ),
    ];
    for (name, listing, file) in &cases {
        let calls_file = tempfile::NamedTempFile::new().unwrap();
        let script = counting_tool(name, calls_file.path(), listing);
        let plugin = format!("{name}-sub");
        let bin = Bin::with_host(
            Path::new("/bin/sh"),
            &[(name, &script)],
            &[(&plugin, PLAIN)],
        );
        warm(&bin, &[name]);
        let mut at = bin.context();
        at.list_stall = STALL;
        let warned = unresolved_on_path(&[&format!("{name} sub")], &at);
        assert_eq!(warned[0].len(), 1, "{name}: {warned:?}");
        assert!(gone(&pid(file)), "{name}: its child outlived the listing");
    }
}

// ---- R5: a tool is known by what it is; a stall is never remembered.

#[test]
fn a_changed_tool_is_asked_again_and_a_stall_is_never_remembered() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let lists = |commands: &str| {
        counting_tool(
            "archon333id",
            file.path(),
            &format!("printf 'Commands:\\n{commands}'; exit 0"),
        )
    };
    let bin = Bin::with_host(
        Path::new("/bin/sh"),
        &[("archon333id", &lists("    build    Build\\n"))],
        &[("archon333id-lint", PLAIN)],
    );
    let tool = bin.dir.path().join("archon333id");
    warm(&bin, &["archon333id"]);
    assert_eq!(bin.warned("archon333id lint").len(), 1);
    assert_eq!(bin.warned("archon333id lint").len(), 1);
    assert_eq!(calls(file.path()), 1, "an unchanged tool is asked once");
    std::fs::write(&tool, lists("    build    Build\\n    lint    Lint\\n")).unwrap();
    warm(&bin, &["archon333id"]);
    assert!(
        bin.warned("archon333id lint").is_empty(),
        "the new tool lists it"
    );
    assert_eq!(calls(file.path()), 2);
    // A link moved to another program is that program.
    let other = bin.dir.path().join("archon333id-v2");
    std::fs::write(&other, lists("    build    Build\\n")).unwrap();
    std::fs::set_permissions(&other, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    std::fs::remove_file(&tool).unwrap();
    std::os::unix::fs::symlink(&other, &tool).unwrap();
    warm(&bin, &["archon333id"]);
    assert_eq!(
        bin.warned("archon333id lint").len(),
        1,
        "the program it now runs"
    );
    // A stall is asked again the next time.
    let stalls = tempfile::NamedTempFile::new().unwrap();
    let hangs = counting_tool("archon333again", stalls.path(), "exec /bin/sleep 30");
    let bin = Bin::with_host(
        Path::new("/bin/sh"),
        &[("archon333again", &hangs)],
        &[("archon333again-sub", PLAIN)],
    );
    warm(&bin, &["archon333again"]);
    let mut at = bin.context();
    at.list_stall = STALL;
    for _ in 0..2 {
        unresolved_on_path(&["archon333again sub"], &at);
    }
    assert_eq!(calls(stalls.path()), 2, "a stall is not remembered");
}

// ---- R6: the committed configuration of the check's tree.

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@example.invalid", "-c", "user.name=t"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A repository whose commit holds `files`; its tree.
fn committed(files: &[(&str, &str)]) -> (tempfile::TempDir, SiteTree) {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q"]);
    for (path, text) in files {
        let file = repo.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    git(repo.path(), &["add", "-A"]);
    git(
        repo.path(),
        &["commit", "-q", "--allow-empty", "-m", "base"],
    );
    let commit = git(repo.path(), &["rev-parse", "HEAD"]);
    let repository = repo.path().to_path_buf();
    (repo, SiteTree { repository, commit })
}

const ALIAS: &str = "[alias]\narchon333tree = \"version\"\n";

#[test]
fn a_listing_reads_the_committed_configuration_of_the_checks_tree() {
    let bin = Bin::new(&or_skip!(host_cargo(), "cargo"), &[SIBLING]);
    let (_repo, tree) = committed(&[(".cargo/config.toml", ALIAS), ("Cargo.toml", "")]);
    let at = bin.context().on(Some(tree));
    assert!(
        unresolved_on_path(&["cargo archon333tree"], &at)[0].is_empty(),
        "its alias"
    );
    let odd = result(Some(101), b"", b"error: no such command: `archon333tree`\n");
    assert_eq!(no_verdict("cargo archon333tree", &odd, &at), None);
    // Without the tree, nothing defines it.
    assert_eq!(bin.warned("cargo archon333tree").len(), 1);
    // An alias only in the working tree is not in the commit the site runs.
    let (repo, tree) = committed(&[("Cargo.toml", "")]);
    std::fs::create_dir_all(repo.path().join(".cargo")).unwrap();
    std::fs::write(repo.path().join(".cargo/config.toml"), ALIAS).unwrap();
    let at = bin.context().on(Some(tree));
    assert_eq!(
        unresolved_on_path(&["cargo archon333tree"], &at)[0].len(),
        1
    );
    // Nor is one in a directory below the root.
    let (_repo, tree) = committed(&[("sub/.cargo/config.toml", ALIAS)]);
    let at = bin.context().on(Some(tree));
    assert_eq!(
        unresolved_on_path(&["cargo archon333tree"], &at)[0].len(),
        1
    );
}

#[test]
fn a_listing_follows_the_toolchain_the_tree_pins() {
    let (proxy, home) = or_skip!(rustup_proxy(), "rustup");
    let bin = Bin::with_host(&proxy, &[SIBLING], &[NEXTEST]);
    let pin = "[toolchain]\nchannel = \"archon-333-pinned\"\n";
    let (_repo, tree) = committed(&[("rust-toolchain.toml", pin)]);
    let at = bin
        .context_with(&[("RUSTUP_HOME", home.as_str())])
        .on(Some(tree));
    let warned = unresolved_on_path(&["cargo nextest run"], &at);
    assert!(
        warned[0].len() == 1 && warned[0][0].contains("archon-333-pinned"),
        "{warned:?}"
    );
}

#[test]
fn only_the_roots_small_files_and_hidden_directories_are_copied() {
    let big = "x".repeat((1 << 20) + 1);
    let (_repo, tree) = committed(&[
        ("rust-toolchain.toml", "t"),
        (".cargo/config.toml", "c"),
        (".config/deep/x.toml", "d"),
        ("src/lib.rs", "s"),
        ("data.bin", &big),
    ]);
    let dir = tempfile::tempdir().unwrap();
    super::tree::materialize(&tree, dir.path()).unwrap();
    let has = |path: &str| dir.path().join(path).is_file();
    assert!(has("rust-toolchain.toml") && has(".cargo/config.toml") && has(".config/deep/x.toml"));
    assert!(!has("src/lib.rs") && !has("data.bin"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".cargo/config.toml")).unwrap(),
        "c"
    );
    let missing = SiteTree {
        repository: tree.repository.clone(),
        commit: "0".repeat(40),
    };
    assert!(super::tree::materialize(&missing, dir.path()).is_err());
}
