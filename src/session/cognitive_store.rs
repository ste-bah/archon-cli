use std::path::{Path, PathBuf};

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
