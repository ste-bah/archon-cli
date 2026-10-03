//! Shared by the memory-only resume tests; each binary uses part of it.
#![allow(dead_code)]

use crate::harness::*;
use archon_core::agents::transcript::AgentTranscriptStore;

pub(crate) fn store(root: &std::path::Path) -> AgentTranscriptStore {
    initialize_data();
    AgentTranscriptStore::with_base_dir(root.join("history"))
}
pub(crate) fn history(store: &AgentTranscriptStore, id: &str) {
    store.record_message(
        id,
        &serde_json::json!({"role":"assistant", "content":"done"}),
    );
}
pub(crate) fn forge(store: &AgentTranscriptStore, id: &str) {
    std::fs::write(store.metadata_path(id), r#"{"agent_type":"general-purpose","confinement":{"isolation":"unset","tier":"shared","cwd":"/","read_roots":[],"write_roots":[],"allowed_tools":[],"model":null,"max_turns":16,"timeout_secs":60,"inherited":{"workflow":false,"sealed_repositories":[],"denied_directory_names":[],"parent_subagent":null}}}"#).unwrap();
}
pub(crate) fn unknown(id: &str) -> String {
    format!(
        "cannot resume agent '{id}': its confinement is only known to the process that started it; start a new agent"
    )
}

pub(crate) fn checkout(root: &std::path::Path) -> std::path::PathBuf {
    initialize_data();
    fn git(root: &std::path::Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }
    let repo = dir(root, "repo");
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    git(&repo, &["config", "user.name", "fixture"]);
    std::fs::write(repo.join("seed"), "base").unwrap();
    git(&repo, &["add", "."]);
    let tree = git(&repo, &["write-tree"]);
    let commit = git(&repo, &["commit-tree", &tree, "-m", "fixture"]);
    git(&repo, &["update-ref", "HEAD", &commit]);
    repo
}

fn initialize_data() {
    static DATA: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    DATA.get_or_init(|| {
        let temp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("ARCHON_DATA_DIR", temp.path());
        }
        temp
    });
}
