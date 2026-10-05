//! Issue 331: a subcommand its search path cannot resolve, of which the
//! host has the program, gives no verdict, by one rule for every tool, on
//! real runs of real tools.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use archon_workflow::acceptance_scratch::CheckResult;

use super::super::{Context, may_be_host_failure, no_verdict};
use super::{LIST_STALL, unresolved_on_path};

/// The host's own `cargo`, if it has one.
pub(super) fn host_cargo() -> Option<PathBuf> {
    // A rustup proxy finds no toolchain under the fixture's empty HOME, so
    // the fixture links the toolchain's own cargo when rustup names one.
    let toolchain = std::process::Command::new("rustup")
        .args(["which", "cargo"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
        .filter(|cargo| cargo.is_file());
    if toolchain.is_some() {
        return toolchain;
    }
    let path = std::env::var("PATH").ok()?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("cargo"))
        .find(|cargo| cargo.is_file())
}

/// A search path: a fresh bin directory holding `cargo` (a link to the
/// host's) and `tools` (name, script), then the system's; and a host whose
/// own search path has a directory of programs the check's path lacks.
pub(super) struct Bin {
    pub(super) dir: tempfile::TempDir,
    pub(super) host: tempfile::TempDir,
}

impl Bin {
    pub(super) fn new(cargo: &Path, tools: &[(&str, &str)]) -> Self {
        Self::with_host(cargo, tools, &[])
    }

    /// As [`Bin::new`], and the host has `installed` (name, script).
    pub(super) fn with_host(
        cargo: &Path,
        tools: &[(&str, &str)],
        installed: &[(&str, &str)],
    ) -> Self {
        let (dir, host) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        std::os::unix::fs::symlink(cargo, dir.path().join("cargo")).unwrap();
        for (dir, tools) in [(dir.path(), tools), (host.path(), installed)] {
            for (name, script) in tools {
                let file = dir.join(name);
                std::fs::write(&file, script).unwrap();
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        Self { dir, host }
    }

    pub(super) fn path(&self) -> String {
        let bin = self.dir.path().canonicalize().unwrap();
        format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display())
    }

    /// The host's search path: its own directory, then the check's.
    pub(super) fn host_path(&self) -> String {
        let host = self.host.path().canonicalize().unwrap();
        format!("{}:{}", host.display(), self.path())
    }

    /// The site: this search path, and `environment` besides.
    pub(super) fn context_with(&self, environment: &[(&str, &str)]) -> Context {
        let mut site: std::collections::BTreeMap<String, String> = (environment.iter())
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        site.insert("PATH".into(), self.path());
        Context {
            path: Some(self.path()),
            deliverables: Vec::new(),
            environment: site,
            host_path: Some(self.host_path()),
            list_stall: LIST_STALL,
        }
    }

    pub(super) fn context(&self) -> Context {
        self.context_with(&[])
    }

    /// What the run-start warning names for `text` at this site.
    pub(super) fn warned(&self, text: &str) -> Vec<String> {
        unresolved_on_path(&[text], &self.context()).remove(0)
    }

    /// Run `command` with only this search path and an empty cargo home,
    /// as the scratch site runs a check.
    pub(super) fn run(&self, command: &str) -> CheckResult {
        self.run_with(command, &[])
    }

    /// As [`Bin::run`], with `environment` besides.
    pub(super) fn run_with(&self, command: &str, environment: &[(&str, &str)]) -> CheckResult {
        let home = tempfile::tempdir().unwrap();
        let out = std::process::Command::new("/bin/sh")
            .args(["-c", command])
            .current_dir(home.path())
            .env_clear()
            .envs(environment.iter().copied())
            .env("PATH", self.path())
            .env("HOME", home.path())
            .env("CARGO_HOME", home.path().join("cargo-home"))
            .output()
            .unwrap();
        result(out.status.code(), &out.stdout, &out.stderr)
    }
}

pub(super) fn result(code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> CheckResult {
    CheckResult {
        acceptance_id: "AC-331".into(),
        exit_code: code,
        stdout: stdout.to_vec(),
        stderr: stderr.to_vec(),
        quota_walk_count: 0,
        operational_error: None,
        classification: None,
    }
}

/// A `cargo-*` program that makes `cargo` a dispatcher on the path, as the
/// toolchain's own `cargo-clippy` does.
pub(super) const SIBLING: (&str, &str) = ("cargo-archon331sibling", "#!/bin/sh\nexit 0\n");

/// The host's own `cargo-nextest`, which the check's path does not reach.
pub(super) const NEXTEST: (&str, &str) = ("cargo-nextest", "#!/bin/sh\nexit 0\n");

macro_rules! cargo_or_skip {
    () => {
        match host_cargo() {
            Some(cargo) => cargo,
            None => {
                eprintln!("skipped: no cargo on the host's PATH");
                return;
            }
        }
    };
}

#[test]
fn cargo_nextest_without_its_program_on_the_path_gives_no_verdict() {
    let bin = Bin::with_host(&cargo_or_skip!(), &[SIBLING], &[NEXTEST]);
    let run = bin.run("cargo nextest run --workspace");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert_eq!(run.exit_code, Some(101), "{stderr}");
    assert!(stderr.contains("no such command"), "{stderr}");
    let why = no_verdict("cargo nextest run --workspace", &run, &bin.context())
        .expect("a missing subcommand is no verdict");
    assert!(
        why.contains("`cargo nextest`") && why.contains("`cargo-nextest`"),
        "{why}"
    );
    assert!(why.contains(&bin.path()), "names the search path: {why}");
    assert!(why.contains("the host has one at"), "{why}");
    assert!(may_be_host_failure(&run), "never remembered as a verdict");
    let warned = bin.warned("cargo nextest run");
    assert_eq!(warned.len(), 1, "{warned:?}");
    assert!(warned[0].contains("`cargo nextest`"), "{warned:?}");
}

#[test]
fn a_built_in_cargo_subcommand_is_unaffected() {
    let host_test = ("cargo-test", "#!/bin/sh\nexit 0\n");
    let bin = Bin::with_host(&cargo_or_skip!(), &[SIBLING], &[host_test]);
    assert!(
        bin.warned("cargo test -q && cargo +stable build")
            .is_empty()
    );
    // Its own failure is still its verdict.
    let run = bin.run("cargo test -q");
    assert_eq!(run.exit_code, Some(101));
    assert_eq!(no_verdict("cargo test -q", &run, &bin.context()), None);
    // Output that rejects it though the host lists it as built in: the
    // site lacks what the host lists (a toolchain its tree pins, which the
    // host never reads), whether or not the line names cargo (Issue 333).
    for own in [
        &b"error: no such command: `test`\n"[..],
        b"error: no such command: `test`\n\n\tView all installed commands with `cargo --list`\n",
    ] {
        let own = result(Some(101), b"", own);
        let why = no_verdict("cargo test", &own, &bin.context()).expect("no verdict");
        assert!(
            why.contains("that site lacks it") && why.contains("must not depend"),
            "{why}"
        );
    }
}

#[test]
fn an_installed_plugin_resolves() {
    let plugin = (
        "cargo-archon331plug",
        "#!/bin/sh\necho 'archon331plug: the check found the feature absent' >&2\nexit 1\n",
    );
    let bin = Bin::new(&cargo_or_skip!(), &[plugin]);
    let run = bin.run("cargo archon331plug --all");
    assert_eq!(run.exit_code, Some(1), "the plugin ran");
    assert_eq!(
        no_verdict("cargo archon331plug --all", &run, &bin.context()),
        None
    );
    let odd = result(Some(101), b"", b"error: no such command: `archon331plug`\n");
    assert_eq!(
        no_verdict("cargo archon331plug", &odd, &bin.context()),
        None
    );
    assert!(bin.warned("cargo archon331plug").is_empty());
}

/// Any tool that dispatches subcommands, under the same rule.
#[test]
fn every_dispatcher_follows_the_same_rule() {
    let tool = (
        "archon331tool",
        "#!/bin/sh\ncase \"$1\" in\n--list) printf 'Commands:\\n    build    Build it\\n' ;;\nbuild) echo 'build: the feature is absent' >&2; exit 1 ;;\n*) if [ -x \"$(dirname \"$0\")/archon331tool-$1\" ]; then exec \"$(dirname \"$0\")/archon331tool-$1\"; fi\n   echo \"archon331tool: '$1' is not a archon331tool command\" >&2; exit 2 ;;\nesac\n",
    );
    let plugin = ("archon331tool-other", "#!/bin/sh\nexit 3\n");
    let lint = ("archon331tool-lint", "#!/bin/sh\nexit 0\n");
    let cargo = host_cargo().unwrap_or_else(|| PathBuf::from("/bin/sh"));
    let bin = Bin::with_host(&cargo, &[tool, plugin], &[lint]);
    let at = bin.context();
    let lint = bin.run("archon331tool lint src");
    let why = no_verdict("archon331tool lint src", &lint, &at).expect("a missing subcommand");
    assert!(why.contains("`archon331tool lint`"), "{why}");
    for command in ["archon331tool build", "archon331tool other"] {
        let run = bin.run(command);
        assert!(matches!(run.exit_code, Some(1 | 3)), "{command}: {run:?}");
        assert_eq!(no_verdict(command, &run, &at), None, "{command}");
    }
    let warned = bin.warned("archon331tool build && archon331tool other && archon331tool lint");
    assert_eq!(warned.len(), 1, "{warned:?}");
    assert!(warned[0].contains("`archon331tool lint`"), "{warned:?}");
}

/// A check that falls back when the subcommand is missing keeps the
/// verdict its fallback gave.
#[test]
fn a_fallback_that_asserted_keeps_its_verdict() {
    let bin = Bin::with_host(&cargo_or_skip!(), &[SIBLING], &[NEXTEST]);
    let run = result(
        Some(101),
        b"test result: FAILED. 0 passed; 1 failed; 0 ignored\n",
        b"error: no such command: `nextest`\n",
    );
    let command = "cargo nextest run || cargo test";
    assert_eq!(no_verdict(command, &run, &bin.context()), None);
}

/// A program a check starts by name that the path lacks is named too.
#[test]
fn a_program_missing_from_the_path_is_named() {
    let bin = Bin::new(Path::new("/bin/sh"), &[]);
    let warned =
        bin.warned("archon-issue-331-absent --verify && /opt/archon-331/none x && ./bin/local");
    assert_eq!(
        warned,
        vec![
            "`archon-issue-331-absent` (not on the path)".to_string(),
            "`/opt/archon-331/none` (no such file)".to_string(),
        ]
    );
}

/// A tool that lists no commands, of whose `tool-word` the host has none,
/// may be the product itself: a subcommand it rejects may be the
/// deliverable not built yet, a verdict (Issue 328). One that lists its
/// commands without the word, with no `tool-word` anywhere, passes only if
/// the deliverable adds it, and so goes back to its author (Issue 333).
#[test]
fn a_tool_not_known_to_dispatch_keeps_its_verdict() {
    let rejects = "#!/bin/sh\necho \"error: unrecognized subcommand '$1'\" >&2\nexit 2\n";
    let lists = "#!/bin/sh\n[ \"$1\" = --list ] && { printf 'Commands:\\n    run    Run\\n'; exit 0; }\necho \"error: unrecognized subcommand '$1'\" >&2\nexit 2\n";
    let tools = [
        ("archon331cli", rejects),
        ("archon331cli-plugin", "#!/bin/sh\nexit 0\n"),
        ("archon331lister", lists),
    ];
    let cargo = host_cargo().unwrap_or_else(|| PathBuf::from("/bin/sh"));
    let bin = Bin::new(&cargo, &tools);
    let run = bin.run("archon331cli data status");
    assert_eq!(run.exit_code, Some(2), "{run:?}");
    assert_eq!(
        no_verdict("archon331cli data status", &run, &bin.context()),
        None
    );
    assert!(bin.warned("archon331cli data status").is_empty());
    let run = bin.run("archon331lister data");
    let why = no_verdict("archon331lister data", &run, &bin.context()).expect("its author's");
    assert!(
        why.contains("passes only if the deliverable adds it"),
        "{why}"
    );
    // The host has `archon331cli-data`: the tool lists no commands, so the
    // listing cannot make it built in, and the run gives no verdict.
    let data = ("archon331cli-data", "#!/bin/sh\nexit 0\n");
    let bin = Bin::with_host(&cargo, &tools, &[data]);
    let run = bin.run("archon331cli data status");
    let why = no_verdict("archon331cli data status", &run, &bin.context()).expect("no verdict");
    assert!(
        why.contains("is not known") && why.contains("exited 2"),
        "{why}"
    );
    let warned = bin.warned("archon331cli data status");
    assert!(
        warned.len() == 1 && warned[0].contains("is not known") && warned[0].contains("exited 2"),
        "{warned:?}"
    );
}
