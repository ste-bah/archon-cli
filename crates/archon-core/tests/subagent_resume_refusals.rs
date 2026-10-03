//! #241, end to end: a resume restores the exact confinement an agent was
//! spawned with, or it refuses. It never runs the agent less confined.
//!
//! Its own test binary, because the executor puts transcripts under the
//! home directory and worktrees under the data directory, and this binary
//! points both at a temporary directory through the environment.

#[path = "support/boundary_harness.rs"]
mod harness;
use harness::*;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use archon_core::agent::AgentConfig;
use archon_core::agents::transcript::{AgentTranscriptStore, plan_resume};
use archon_tools::isolation::{AutoIsolation, IsolationTier};
use archon_tools::tool::ToolContext;

const BOUNDARY: &str = "workspace-boundary";

/// The temporary home every test in this binary shares. Set once, before
/// any test reads the environment.
fn home() -> &'static Path {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = std::fs::canonicalize(temp.path()).expect("real temp");
        std::mem::forget(temp);
        // SAFETY: set inside the `OnceLock` initialiser, which every test
        // calls before it reads the environment, so no other thread reads
        // it while it is written.
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("XDG_DATA_HOME", home.join("data"));
            std::env::set_var("ARCHON_DATA_DIR", home.join("data").join("archon"));
        }
        home
    })
}

fn new_session() -> (String, AgentTranscriptStore) {
    home();
    let session = format!("resume-refusals-{}", uuid::Uuid::new_v4());
    let store = AgentTranscriptStore::new(&session).expect("a home directory");
    (session, store)
}

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "{args:?}: {out:?}");
}

/// A checkout with one commit.
fn checkout(root: &Path) -> PathBuf {
    let repo = dir(root, "repo");
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "t@example.invalid"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("lib.txt"), "base\n").expect("seed");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "base"]);
    repo
}

/// The stored record, as the JSON the spawn wrote.
fn record(store: &AgentTranscriptStore, agent: &str) -> serde_json::Value {
    let raw = std::fs::read_to_string(store.metadata_path(agent)).expect("metadata");
    let meta: serde_json::Value = serde_json::from_str(&raw).expect("metadata json");
    meta["confinement"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_boundary_agent_on_a_worktree_rung_resumes_in_its_checkout() {
    let (_t, root) = real_temp();
    let repo = checkout(&root);
    let project = dir(&root, "project");
    let escaped = repo.join("escaped.txt");
    let (session, store) = new_session();
    let agent = format!("boundary-on-worktree-{}", uuid::Uuid::new_v4());
    // The policy isolates every writer, so the bounded agent runs on the
    // worktree rung without naming it, as an overlap would put it there.
    let always = AgentConfig {
        subagent_auto_isolation: AutoIsolation::Always,
        ..AgentConfig::default()
    };
    Host::with_config(&project, &session, vec![STOP], always)
        .spawn(
            &agent,
            request(&repo, Some(BOUNDARY), Vec::new()),
            parent(&project, &[]),
        )
        .await
        .expect("the spawn runs");
    let spawned = record(&store, &agent);
    assert_eq!(spawned["isolation"], BOUNDARY, "{spawned}");
    assert_eq!(spawned["tier"], "worktree", "{spawned}");

    // The policy that put it there no longer applies (the overlap is gone).
    let host = Host::new(&project, &session, vec![write(&escaped, "x\n"), STOP]);
    let plan = plan_resume(&store, &agent, "continue")
        .expect("a transcript")
        .expect("a recorded spawn resumes");
    let outcome = host.resume(&agent, plan, parent(&project, &[])).await;

    assert!(
        !escaped.exists(),
        "the resumed agent ran in the source checkout, not on its worktree rung: {outcome:?}"
    );
    assert!(outcome.is_ok(), "{outcome:?}");
    let refused = host.outcome(0);
    assert!(refused.is_error, "{refused:?}");
    // Refused because it is confined to its own checkout, which lives under
    // the data directory's worktrees.
    assert!(refused.text.contains("worktrees"), "{refused:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_resume_refuses_when_the_tier_cap_would_lower_its_rung() {
    let (_t, root) = real_temp();
    let repo = checkout(&root);
    let project = dir(&root, "project");
    let escaped = repo.join("escaped.txt");
    let (session, store) = new_session();
    let agent = format!("worktree-agent-{}", uuid::Uuid::new_v4());
    Host::new(&project, &session, vec![STOP])
        .spawn(
            &agent,
            request(&repo, Some("worktree"), Vec::new()),
            parent(&project, &[]),
        )
        .await
        .expect("the spawn runs");
    assert_eq!(record(&store, &agent)["tier"], "worktree");

    // The cap was lowered after the spawn: the rung is no longer granted.
    let capped = AgentConfig {
        subagent_isolation_max_tier: IsolationTier::Shared,
        ..AgentConfig::default()
    };
    let host = Host::with_config(&project, &session, vec![write(&escaped, "x\n")], capped);
    let plan = plan_resume(&store, &agent, "continue")
        .expect("a transcript")
        .expect("the record is complete");
    let outcome = host.resume(&agent, plan, parent(&project, &[])).await;

    assert!(
        !escaped.exists(),
        "the resumed agent ran on a lower rung, in the source checkout: {outcome:?}"
    );
    let refusal = outcome
        .expect_err("a resume on a lower rung must be refused")
        .to_string();
    assert!(refusal.contains(&agent), "{refusal}");
    assert!(refusal.contains("worktree"), "{refusal}");
    assert!(refusal.contains("isolation_max_tier"), "{refusal}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_confined_agent_cannot_rewrite_its_own_resume_record() {
    let (_t, root) = real_temp();
    let project = dir(&root, "project");
    let workspace = dir(&root, "workspace");
    let (session, store) = new_session();
    let records = store
        .metadata_path("x")
        .parent()
        .expect("records dir")
        .to_path_buf();
    std::fs::create_dir_all(&records).expect("records dir");
    let unbounded = serde_json::json!({
        "agent_type": "general-purpose",
        "confinement": {
            "isolation": null, "tier": "none", "cwd": "/", "read_roots": [],
            "write_roots": [], "allowed_tools": [], "model": null,
            "max_turns": 16, "timeout_secs": 60
        }
    })
    .to_string();

    // Once with a write root that holds the records, once in the home
    // directory itself, which holds them too.
    for (name, cwd, write_roots) in [
        (
            "root",
            workspace.clone(),
            vec![records.display().to_string()],
        ),
        ("home", home().to_path_buf(), Vec::new()),
    ] {
        let agent = format!("tamper-{name}-{}", uuid::Uuid::new_v4());
        let meta = store.metadata_path(&agent);
        let host = Host::new(
            &project,
            &session,
            vec![read(&meta), write(&meta, &unbounded), STOP],
        );
        let mut spawn = request(&cwd, Some(BOUNDARY), Vec::new());
        spawn.write_roots = write_roots;
        host.spawn(&agent, spawn, parent(&project, &[]))
            .await
            .expect("the spawn runs");

        let attempt = host.outcome(1);
        assert!(
            attempt.is_error,
            "{name}: the agent rewrote its own record: {attempt:?}"
        );
        // Read back from outside the agent: the record is the spawn's.
        let kept = record(&store, &agent);
        assert_eq!(kept["isolation"], BOUNDARY, "{name}: {kept}");
        assert_eq!(kept["cwd"], cwd.display().to_string(), "{name}: {kept}");
        let plan = plan_resume(&store, &agent, "continue")
            .expect("a transcript")
            .expect("the spawn's record resumes");
        assert_eq!(plan.request.isolation.as_deref(), Some(BOUNDARY), "{name}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_child_with_inherited_confinement_is_refused_not_resumed_weaker() {
    let (_t, root) = real_temp();
    let repo = checkout(&root);
    let project = dir(&root, "project");
    let workspace = dir(&root, "workspace");
    let (session, store) = new_session();
    let cases = [
        (
            "workflow",
            ToolContext {
                run_store: Some(archon_tools::workflow_read_guard::RunStoreScope::default()),
                ..parent(&project, &[])
            },
            "workflow",
        ),
        (
            "sealed",
            ToolContext {
                sealed_repositories: vec![repo.clone()],
                ..parent(&project, &[])
            },
            "sealed",
        ),
        (
            "nested",
            ToolContext {
                subagent_id: Some("lead-subagent".into()),
                ..parent(&project, &[])
            },
            "subagent 'lead-subagent'",
        ),
    ];
    for (name, spawn_parent, why) in cases {
        let agent = format!("inherited-{name}-{}", uuid::Uuid::new_v4());
        Host::new(&project, &session, vec![STOP])
            .spawn(&agent, request(&workspace, None, Vec::new()), spawn_parent)
            .await
            .expect("the spawn runs");

        let refusal = plan_resume(&store, &agent, "continue")
            .expect("a transcript")
            .expect_err("a child that inherited confinement must not resume without it");
        assert!(refusal.contains(&format!("'{agent}'")), "{name}: {refusal}");
        assert!(refusal.contains(why), "{name}: {refusal}");
        assert!(refusal.contains("spawn a new agent"), "{name}: {refusal}");
    }
}
