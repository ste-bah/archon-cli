//! Schema-4 and schema-5 records through the whole audit (#434c, #434d):
//! reused without a critic call under the critic logic version they answer
//! to, asked again under any other.

use archon_workflow::fidelity_audit::{ClaimedObligation, ClaimingTask, SkeletonSummary};

use super::*;
use crate::command::topology_lint::fidelity_store::{
    CRITIC_LOGIC_VERSION, LEGACY_CRITIC_LOGIC, PREDECESSOR_VERSION, store_dir,
};

/// Rewrite the current record at `path` as `schema` wrote it: schema 4 with
/// a build name, schema 5 with `critic_logic`.
fn convert(path: &Path, schema: u32, critic_logic: &str) {
    use sha2::Digest;

    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let identity = &record["identity"];
    let earlier_identity = if schema == 4 {
        serde_json::json!({
            "binary": "binary-X",
            "model": identity["model"],
            "provider": identity["provider"],
            "request": identity["request"],
        })
    } else {
        serde_json::json!({
            "critic_logic": critic_logic,
            "model": identity["model"],
            "provider": identity["provider"],
            "request": identity["request"],
        })
    };
    let material = serde_json::json!([
        schema,
        record["obligation"],
        record["skeleton"],
        earlier_identity,
    ]);
    let key = hex::encode(sha2::Sha256::digest(material.to_string().as_bytes()));
    record["schema"] = schema.into();
    record["key"] = key.clone().into();
    record["identity"] = earlier_identity;
    let earlier_path = path.with_file_name(format!("{key}.json"));
    std::fs::write(&earlier_path, serde_json::to_vec(&record).unwrap()).unwrap();
    std::fs::remove_file(path).unwrap();
}

fn convert_to_schema_four(path: &Path) {
    convert(path, 4, "");
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

async fn resolve_under(cwd: &Path, critic: &Arc<Critic>, version: u32) -> Resolved {
    let identity = StoreIdentity::of(critic.as_ref()).with_critic_logic_version(version);
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

/// H1: the same schema-4 records are reused under the version they answer
/// to and asked again under any other.
#[tokio::test]
async fn schema_four_records_migrate_only_under_the_predecessor_version() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path();
    let first = resolve_under(cwd, &Critic::new(), PREDECESSOR_VERSION).await;
    assert_eq!((first.asked, first.cached), (6, 0));
    for path in records(cwd) {
        convert_to_schema_four(&path);
    }

    let critic = Critic::new();
    let changed = resolve_under(cwd, &critic, PREDECESSOR_VERSION + 1).await;
    assert_eq!(critic.calls(), 3, "another version asks every batch");
    assert_eq!((changed.asked, changed.cached), (6, 0));

    let critic = Critic::new();
    let migrated = resolve_under(cwd, &critic, PREDECESSOR_VERSION).await;
    assert_eq!(critic.calls(), 0, "validated legacy entries are promoted");
    assert_eq!((migrated.asked, migrated.cached), (0, 6));
    assert_eq!(
        records(cwd).len(),
        18,
        "legacy, other-logic and promoted records"
    );
}

/// The real gate path with this build's version: schema-4 records, and
/// schema-5 records under the digest 22e024436 wrote, migrate exactly when
/// this build's critic logic version is theirs (true when #434d landed);
/// schema-5 records under any other digest are asked again.
#[tokio::test]
async fn this_build_migrates_schema_four_and_five_records_under_their_version() {
    let reused = if CRITIC_LOGIC_VERSION == PREDECESSOR_VERSION {
        0
    } else {
        3
    };
    for (schema, critic_logic, calls) in [
        (4, "", reused),
        (5, LEGACY_CRITIC_LOGIC, reused),
        (5, "a-digest-no-build-wrote", 3),
    ] {
        let temp = corpus();
        let cwd = temp.path();
        complete(cwd, Critic::new(), &FreezeResume::none()).await;
        let current = records(cwd);
        assert_eq!(current.len(), 6);
        for path in current {
            convert(&path, schema, critic_logic);
        }

        let critic = Critic::new();
        complete(cwd, critic.clone(), &FreezeResume::none()).await;
        assert_eq!(critic.calls(), calls, "schema {schema} {critic_logic}");
        assert_eq!(
            records(cwd).len(),
            12,
            "earlier and schema-6 records coexist"
        );
    }
}

/// A store whose build digest is `built` (#434d test seam), under this
/// build's version.
async fn resolve_built_from(cwd: &Path, critic: &Arc<Critic>, built: &str) -> Resolved {
    let identity = StoreIdentity::of(critic.as_ref());
    let store = VerdictStore::built_from(store_dir(cwd), identity, built);
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

/// Every file of the store, with its bytes.
fn snapshot(cwd: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files: Vec<_> = std::fs::read_dir(store_dir(cwd))
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    files.sort();
    files
}

/// #434d: a binary built from critic sources other than the pinned ones
/// (an unreviewed edit, a dirty tree) reuses no saved verdict, migrates no
/// earlier record and writes nothing; it asks the critic, and the gate runs.
#[tokio::test]
async fn unreviewed_critic_sources_read_migrate_and_write_no_record() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path();
    let first = resolve_under(cwd, &Critic::new(), CRITIC_LOGIC_VERSION).await;
    assert_eq!((first.asked, first.cached), (6, 0));
    for path in &records(cwd)[..2] {
        convert_to_schema_four(path);
    }
    let before = snapshot(cwd);

    let critic = Critic::new();
    let unreviewed = resolve_built_from(cwd, &critic, "an-unreviewed-sources-digest").await;
    assert_eq!(critic.calls(), 3, "every batch is asked");
    assert_eq!(
        (unreviewed.asked, unreviewed.cached),
        (6, 0),
        "no record is read"
    );
    assert_eq!(snapshot(cwd), before, "nothing is written or migrated");

    let critic = Critic::new();
    let reviewed = resolve_built_from(cwd, &critic, env!("ARCHON_FIDELITY_SOURCES_DIGEST")).await;
    assert_eq!(
        critic.calls(),
        0,
        "the reviewed build reuses and migrates them"
    );
    assert_eq!((reviewed.asked, reviewed.cached), (0, 6));
}
