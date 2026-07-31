use std::path::{Path, PathBuf};

pub(super) fn wire_runtime(
    agent: &mut archon_core::agent::Agent,
    config: &archon_core::config::ArchonConfig,
    working_dir: &Path,
    store: archon_cognitive::PersistentCognitiveStore,
) {
    let ledger_dir = store.root().to_path_buf();
    agent.set_cognitive_store(store);
    let policy = match runtime_policy(working_dir) {
        Ok(Some(policy)) => policy,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(%error, "cognitive executive policy unavailable; advisory disabled");
            return;
        }
    };
    if !config.learning.cognitive.enabled {
        return;
    }
    agent.set_cognitive_executive(config.learning.cognitive.clone(), policy, ledger_dir);
    wire_world_model(agent, config, working_dir);
}

/// Inject the world-model prediction backend behind the executive advisory.
///
/// Fail-open by design: if the world model is disabled, has no active model, or
/// its store cannot be read, the agent keeps the heuristic scorer and the turn
/// is unaffected.
fn wire_world_model(
    agent: &mut archon_core::agent::Agent,
    config: &archon_core::config::ArchonConfig,
    working_dir: &Path,
) {
    let Some((advisor, state)) = crate::command::world_model::runtime_prediction_context(config)
    else {
        return;
    };
    let session_id = working_dir.display().to_string();
    let backend: archon_cognitive::SharedPredictionBackend = std::sync::Arc::new(
        super::world_model_backend::WorldModelPredictionBackend::new(advisor, session_id),
    );
    tracing::debug!(
        active_model_id = ?state.active_model_id,
        shadow_only = state.shadow_only,
        "world-model prediction backend wired into cognitive advisory"
    );
    agent.set_cognitive_world_model(backend, state);
}

fn runtime_policy(
    working_dir: &Path,
) -> Result<Option<archon_cognitive::CognitivePolicy>, archon_policy::PolicyError> {
    let policy = archon_policy::load_effective_policy(working_dir)?.cognitive;
    Ok(policy.enabled.then_some(policy))
}

pub(super) async fn open(
    working_dir: &Path,
) -> anyhow::Result<Option<archon_cognitive::PersistentCognitiveStore>> {
    let root = working_dir.join(".archon").join("cognitive");
    open_with(root, archon_cognitive::PersistentCognitiveStore::open).await
}

async fn open_with<F>(
    root: PathBuf,
    opener: F,
) -> anyhow::Result<Option<archon_cognitive::PersistentCognitiveStore>>
where
    F: FnOnce(
            PathBuf,
        ) -> Result<
            archon_cognitive::PersistentCognitiveStore,
            archon_cognitive::CognitiveError,
        > + Send
        + 'static,
{
    let display_root = root.clone();
    let result =
        archon_observability::spawn_blocking_named("open-cognitive-store", move || opener(root))
            .await;
    match result {
        Ok(Ok(store)) => {
            tracing::info!(
                path = %store.db_path().display(),
                "cognitive executive store wired"
            );
            Ok(Some(store))
        }
        Ok(Err(error)) => {
            tracing::warn!(
                %error,
                path = %display_root.display(),
                "cognitive executive store unavailable; continuing without persistence"
            );
            Ok(None)
        }
        Err(error) => Err(anyhow::anyhow!(
            "cognitive executive store task failed for {}: {error}",
            display_root.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn disabled_cognitive_policy_does_not_enable_runtime() {
        let temp = tempfile::tempdir().unwrap();

        assert!(runtime_policy(temp.path()).unwrap().is_none());
    }

    #[test]
    fn malformed_cognitive_policy_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(".archon");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("policy.toml"), "[policy.cognitive\nenabled = true").unwrap();

        assert!(runtime_policy(temp.path()).is_err());
    }

    #[tokio::test]
    async fn panicking_store_open_is_a_startup_error() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cognitive");

        let result = open_with(root, |_| panic!("store opener panicked")).await;

        assert!(
            result.is_err(),
            "store panic was downgraded to optional absence"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blocked_store_open_does_not_stall_runtime_task() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cognitive");
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();

        let opening = tokio::spawn(open_with(root, move |root| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            archon_cognitive::PersistentCognitiveStore::open(root)
        }));
        archon_observability::spawn_blocking_named("wait-for-cognitive-open", move || {
            started_rx.recv().unwrap()
        })
        .await
        .unwrap();

        let heartbeat = tokio::spawn(async { "runtime-responsive" });
        assert_eq!(
            tokio::time::timeout(Duration::from_millis(100), heartbeat)
                .await
                .unwrap()
                .unwrap(),
            "runtime-responsive"
        );

        release_tx.send(()).unwrap();
        assert!(opening.await.unwrap().unwrap().is_some());
    }
}
