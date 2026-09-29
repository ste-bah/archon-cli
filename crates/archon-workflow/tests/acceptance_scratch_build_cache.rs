//! Batch J2: native observations sharing a build cache build warm across
//! observations and commits, rebuild exactly what changed, and go cold
//! whenever a build could have seen other bytes than a file's time says.
//!
//! Each observation here is a real scratch (`ScratchRoots::prepare`) at a
//! commit of a two-crate Cargo workspace, running `cargo build -v` with the
//! scratch's own environment, as a check does. `Fresh dep` in Cargo's
//! verbose output means the unchanged crate was reused; `Compiling app`
//! that the changed one was rebuilt.
#![cfg(unix)]

use archon_workflow::acceptance_scratch::{ScratchPolicy, ScratchRoots};
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// The directory holding the `cargo` this test runs with, if any.
fn cargo_dir() -> Option<PathBuf> {
    let out = Command::new("sh")
        .args(["-c", "command -v cargo"])
        .output()
        .ok()?;
    let path = PathBuf::from(String::from_utf8(out.stdout).ok()?.trim());
    Some(path.canonicalize().ok()?.parent()?.to_path_buf())
}

struct Fixture {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    policy: ScratchPolicy,
    commits: Vec<String>,
}

fn commit(repo: &Path, file: &str, body: &str) -> String {
    std::fs::create_dir_all(repo.join(file).parent().unwrap()).unwrap();
    std::fs::write(repo.join(file), body).unwrap();
    git(repo, &["add", "."]);
    git(repo, &["commit", "-qm", file]);
    git(repo, &["rev-parse", "HEAD"])
}

fn fixture(cache: bool) -> Option<Fixture> {
    let cargo = cargo_dir()?;
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    let project = temp.path().join("project");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    git(&repo, &["config", "user.name", "fixture"]);
    std::fs::write(repo.join(".gitignore"), "/target\n").unwrap();
    std::fs::write(
        repo.join("Cargo.toml"),
        "[workspace]\nmembers = [\"dep\", \"app\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(repo.join("dep/src")).unwrap();
    std::fs::create_dir_all(repo.join("app/src")).unwrap();
    std::fs::write(
        repo.join("dep/Cargo.toml"),
        "[package]\nname = \"dep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(repo.join("dep/src/lib.rs"), "pub fn v() -> u32 { 1 }\n").unwrap();
    std::fs::write(
        repo.join("app/Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[dependencies]\ndep = { path = \"../dep\" }\n",
    )
    .unwrap();
    let first = commit(
        &repo,
        "app/src/main.rs",
        "fn main() { println!(\"{}\", dep::v()); }\n",
    );
    let second = commit(
        &repo,
        "app/src/main.rs",
        "fn main() { println!(\"{}\", dep::v() + 1); }\n",
    );
    let mut environment = std::collections::BTreeMap::new();
    // A rustup proxy finds its toolchains through RUSTUP_HOME, not the
    // scratch's private HOME.
    let rustup = std::env::var("RUSTUP_HOME").ok().or_else(|| {
        let home = std::env::var("HOME").ok()?;
        Path::new(&home)
            .join(".rustup")
            .is_dir()
            .then(|| format!("{home}/.rustup"))
    });
    if let Some(rustup) = rustup {
        environment.insert("RUSTUP_HOME".to_string(), rustup);
    }
    let policy = ScratchPolicy {
        repository: repo.clone(),
        project: project.clone(),
        task_root: project.join("tasks"),
        scratch_parent: temp.path().join("observations"),
        project_inputs: vec![],
        project_input_excludes: vec![],
        combined: true,
        toolchain_path: format!("{}:/usr/bin:/bin", cargo.display()),
        environment,
        environment_allowlist: vec![],
        cargo_seed: None,
        timeout_secs: 600,
        output_bytes: 1 << 20,
        scratch_bytes: 4 << 30,
        build_cache: cache.then(|| temp.path().join("observations/build-cache/wf-test")),
    };
    Some(Fixture {
        _temp: temp,
        repo,
        policy,
        commits: vec![first, second],
    })
}

/// One observation at `commit`: `cargo build -v` in the scratch project,
/// with an optional hook run first; returns Cargo's report and the roots'
/// scratch path.
fn observe(f: &Fixture, commit: &str, before: impl FnOnce(&ScratchRoots)) -> (String, PathBuf) {
    let mut roots = ScratchRoots::prepare(&f.policy, commit).unwrap();
    before(&roots);
    let out = Command::new("cargo")
        .args(["build", "-v", "--offline"])
        .current_dir(roots.project())
        .env_clear()
        .envs(roots.environment(&f.policy))
        .output()
        .unwrap();
    let report = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{report}");
    let root = roots.root().to_path_buf();
    roots.cleanup().unwrap();
    (report, root)
}

fn fresh(report: &str, krate: &str) -> bool {
    report.contains(&format!("Fresh {krate} v0.1.0"))
}

fn compiled(report: &str, krate: &str) -> bool {
    report.contains(&format!("Compiling {krate} v0.1.0"))
}

#[test]
fn observations_sharing_a_cache_build_warm_across_commits() {
    let Some(f) = fixture(true) else {
        eprintln!("no cargo on PATH; skipped");
        return;
    };
    let (one, two) = (&f.commits[0], &f.commits[1]);
    let (cold, root) = observe(&f, one, |_| {});
    assert!(compiled(&cold, "dep") && compiled(&cold, "app"), "{cold}");
    // Another commit: the unchanged crate is reused, the changed one rebuilt.
    let (warm, again) = observe(&f, two, |_| {});
    assert!(fresh(&warm, "dep") && compiled(&warm, "app"), "{warm}");
    assert_eq!(root, again, "one fixed scratch path per cache");
    // Back to the first commit: its bytes are not the last ones built.
    let (back, _) = observe(&f, one, |_| {});
    assert!(fresh(&back, "dep") && compiled(&back, "app"), "{back}");
    // The same commit twice: nothing to build.
    let (same, _) = observe(&f, one, |_| {});
    assert!(fresh(&same, "dep") && fresh(&same, "app"), "{same}");
    // The cache lives outside the scratch, which is gone after each one.
    assert!(!root.exists());
    assert!(
        git(&f.repo, &["worktree", "list", "--porcelain"])
            .lines()
            .all(|l| !l.contains("build-cache"))
    );
}

/// Without a cache every observation builds cold (the pre-J2 behaviour).
#[test]
fn observations_without_a_cache_build_cold() {
    let Some(f) = fixture(false) else {
        return;
    };
    let (_, first) = observe(&f, &f.commits[0], |_| {});
    let (cold, second) = observe(&f, &f.commits[1], |_| {});
    assert!(compiled(&cold, "dep"), "{cold}");
    assert_ne!(first, second);
}

/// A check that touches a tracked file (its build may have seen other
/// bytes than the file's time stands for) makes the next one build cold,
/// as does a slot a holder never tore down.
#[test]
fn a_touched_source_or_an_abandoned_slot_forgets_every_build() {
    let Some(f) = fixture(true) else {
        return;
    };
    let one = &f.commits[0];
    observe(&f, one, |_| {});
    // A check rewrites a dependency's source and builds it, then puts the
    // bytes back: same content, but the build saw other bytes.
    let (_, _) = observe(&f, one, |roots| {
        let lib = roots.project().join("dep/src/lib.rs");
        std::fs::write(&lib, "pub fn v() -> u32 { 2 }\n").unwrap();
        let built = Command::new("cargo")
            .args(["build", "--offline"])
            .current_dir(roots.project())
            .env_clear()
            .envs(roots.environment(&f.policy))
            .status()
            .unwrap();
        assert!(built.success());
        std::fs::write(&lib, "pub fn v() -> u32 { 1 }\n").unwrap();
    });
    let (after, _) = observe(&f, one, |_| {});
    assert!(compiled(&after, "dep"), "{after}");
    // A slot left behind by a holder that died: reclaimed, and cold.
    let cache = f.policy.build_cache.clone().unwrap();
    let generation = std::fs::read_to_string(cache.join("generation")).unwrap_or_default();
    let slot = format!("scratch-{}", generation.trim().parse::<u64>().unwrap_or(0));
    std::fs::create_dir_all(cache.join(slot).join("junk")).unwrap();
    let (reclaimed, _) = observe(&f, one, |_| {});
    assert!(compiled(&reclaimed, "dep"), "{reclaimed}");
}

/// Observations sharing a cache run one at a time.
#[test]
fn observations_sharing_a_cache_are_serialized() {
    let Some(f) = fixture(true) else {
        return;
    };
    let f = std::sync::Arc::new(f);
    let held = std::time::Duration::from_millis(1500);
    let started = std::time::Instant::now();
    let first = {
        let f = f.clone();
        std::thread::spawn(move || {
            let mut roots = ScratchRoots::prepare(&f.policy, &f.commits[0]).unwrap();
            std::thread::sleep(held);
            // The lock is released inside cleanup, after this instant.
            let releasing = std::time::Instant::now();
            roots.cleanup().unwrap();
            releasing
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(300));
    let mut second = ScratchRoots::prepare(&f.policy, &f.commits[1]).unwrap();
    let entered = std::time::Instant::now();
    second.cleanup().unwrap();
    let released = first.join().unwrap();
    assert!(
        entered >= released,
        "the second entered while the first held the cache"
    );
    assert!(entered.duration_since(started) >= held);
}
