//! Schema-4 records through the whole audit (#434c): reused without a
//! critic call under the critic logic they were written by, asked again
//! under any other.

use archon_workflow::fidelity_audit::{ClaimedObligation, ClaimingTask, SkeletonSummary};

use super::*;
use crate::command::topology_lint::fidelity_store::{LEGACY_CRITIC_LOGIC, store_dir};

fn convert_to_schema_four(path: &Path) {
    use sha2::Digest;

    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let identity = &record["identity"];
    let legacy_identity = serde_json::json!({
        "binary": "binary-X",
        "model": identity["model"],
        "provider": identity["provider"],
        "request": identity["request"],
    });
    let material =
        serde_json::json!([4, record["obligation"], record["skeleton"], legacy_identity,]);
    let key = hex::encode(sha2::Sha256::digest(material.to_string().as_bytes()));
    record["schema"] = 4.into();
    record["key"] = key.clone().into();
    record["identity"] = legacy_identity;
    let legacy_path = path.with_file_name(format!("{key}.json"));
    std::fs::write(&legacy_path, serde_json::to_vec(&record).unwrap()).unwrap();
    std::fs::remove_file(path).unwrap();
}

/// The corpus's three clusters, as the audit builds them.
fn clusters() -> Vec<Cluster> {
    [(1, [1, 2]), (2, [3, 4]), (3, [5, 6])]
        .into_iter()
        .map(|(n, claims)| {
            let obligations = claims
                .iter()
                .map(|claim| ClaimedObligation {
                    id: format!("{PREFIX}00{claim}"),
                    text: format!("the catalogue lists gadget {claim}"),
                })
                .collect();
            let tasks = vec![ClaimingTask {
                task_id: format!("TASK-QX-00{n}"),
                text: task(n, claims),
            }];
            (obligations, tasks)
        })
        .collect()
}

async fn resolve_under(cwd: &Path, critic: &Arc<Critic>, critic_logic: &str) -> Resolved {
    let identity = StoreIdentity::of(critic.as_ref()).with_critic_logic(critic_logic);
    let store = VerdictStore::new(store_dir(cwd), identity);
    resolve(
        critic.as_ref(),
        &store,
        &clusters(),
        &SkeletonSummary::absent(),
        &FreezeResume::none(),
    )
    .await
    .expect("resolved")
}

/// H1: the same schema-4 records are reused under the schema-4 critic logic
/// and asked again under any other.
#[tokio::test]
async fn schema_four_records_migrate_only_under_the_schema_four_critic_logic() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path();
    let first = resolve_under(cwd, &Critic::new(), LEGACY_CRITIC_LOGIC).await;
    assert_eq!((first.asked, first.cached), (6, 0));
    for path in records(cwd) {
        convert_to_schema_four(&path);
    }

    let critic = Critic::new();
    let changed = resolve_under(cwd, &critic, "critic-logic-after-schema-four").await;
    assert_eq!(critic.calls(), 3, "other critic logic asks every batch");
    assert_eq!((changed.asked, changed.cached), (6, 0));

    let critic = Critic::new();
    let migrated = resolve_under(cwd, &critic, LEGACY_CRITIC_LOGIC).await;
    assert_eq!(critic.calls(), 0, "validated legacy entries are promoted");
    assert_eq!((migrated.asked, migrated.cached), (0, 6));
    assert_eq!(
        records(cwd).len(),
        18,
        "legacy, other-logic and promoted records"
    );
}

/// The real gate path with this build's digest: it migrates exactly when
/// this build's critic logic is the schema-4 logic (true when #434c landed).
#[tokio::test]
async fn this_build_migrates_schema_four_records_only_under_their_critic_logic() {
    let temp = corpus();
    let cwd = temp.path();
    complete(cwd, Critic::new(), &FreezeResume::none()).await;
    let current = records(cwd);
    assert_eq!(current.len(), 6);
    for path in current {
        convert_to_schema_four(&path);
    }

    let critic = Critic::new();
    complete(cwd, critic.clone(), &FreezeResume::none()).await;
    let migrates = env!("ARCHON_FIDELITY_LOGIC_DIGEST") == LEGACY_CRITIC_LOGIC;
    assert_eq!(critic.calls(), if migrates { 0 } else { 3 });
    assert_eq!(
        records(cwd).len(),
        12,
        "legacy and schema-5 records coexist"
    );
}
