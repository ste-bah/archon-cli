use super::*;
use crate::v2::acceptance_regression::{CheckObserver, Observations, SearchBudget, Verdict};
use std::collections::BTreeMap;

struct Takeover {
    store: WorkflowStore,
    run: String,
    case: u8,
}
#[async_trait::async_trait]
impl CheckObserver for Takeover {
    async fn observe(&self, _: &str, ids: &[String]) -> Option<BTreeMap<String, Verdict>> {
        tokio::task::yield_now().await;
        let control = LifecycleController::new(self.store.clone());
        control
            .apply(
                &self.run,
                if self.case == 2 {
                    LifecycleAction::Cancel
                } else {
                    LifecycleAction::Pause
                },
            )
            .unwrap();
        if self.case == 1 {
            control.apply(&self.run, LifecycleAction::Resume).unwrap();
        }
        Some(
            ids.iter()
                .map(|id| (id.clone(), Verdict::Held(true)))
                .collect(),
        )
    }
}
async fn cache_takeover(case: u8) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let (run_id, generation) = running(&store);
    let root = store.run_dir(&run_id);
    let path = root
        .join(crate::v2::acceptance_stage::ACCEPTANCE_RECORDS_DIR)
        .join("observations/abcdef.json");
    if case == 1 {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{\"other\":true}").unwrap();
    }
    let before = std::fs::read(&path).ok();
    let observer = Takeover {
        store: store.clone(),
        run: run_id.clone(),
        case,
    };
    let writer = StageWriter {
        store,
        run_id,
        owner: PauseOwner::Generation(generation),
    };
    let found = scope(writer, async {
        let mut observations = Observations::new(&root, &observer, SearchBudget::default());
        observations
            .at("abcdef", &[("check".into(), "check".into())])
            .await
    })
    .await;
    assert_eq!(
        std::fs::read(path).ok(),
        before,
        "obsolete observer changed the cache"
    );
    assert!(found.is_none(), "obsolete observation returned a verdict");
}
#[tokio::test]
async fn r3_paused_observer_cannot_create_cache() {
    cache_takeover(0).await;
}
#[tokio::test]
async fn r3_resumed_observer_cannot_overwrite_cache() {
    cache_takeover(1).await;
}
#[tokio::test]
async fn r3_cancelled_observer_cannot_create_cache() {
    cache_takeover(2).await;
}
