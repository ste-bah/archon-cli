//! The migration of schema-4 and schema-5 records (#434c, #434d): only under
//! the critic logic version they answer to, only for the very same question,
//! every record for it merged so a flip between deploys is a recorded
//! contradiction, and never a gate failure because of the scan.

use super::super::legacy::{LEGACY_CRITIC_LOGIC, LEGACY_STORE_SCHEMA, PREDECESSOR_VERSION};
use super::*;

/// The store of a build whose critic logic version is the earlier schemas'.
fn legacy_store(dir: &Path) -> VerdictStore {
    VerdictStore::new(
        dir.join("verdicts"),
        identity().with_critic_logic_version(PREDECESSOR_VERSION),
    )
}

fn entry(at: &str, texts: &TaskTexts, verdict: FidelityVerdict) -> Entry {
    Entry {
        audited_at: at.into(),
        tasks: texts.clone(),
        verdict,
    }
}

/// A verified schema-4 record of `binary` for `obligation`, as schema 4
/// wrote it.
fn write_legacy(
    store: &VerdictStore,
    obligation: &ClaimedObligation,
    skeleton: &str,
    binary: &str,
    entries: Vec<Entry>,
) {
    let identity = serde_json::json!({
        "binary": binary,
        "model": "model",
        "provider": "provider",
        "request": "representative-request",
    });
    let material = serde_json::json!([LEGACY_STORE_SCHEMA, obligation, skeleton, identity]);
    let key = hex::encode(Sha256::digest(material.to_string().as_bytes()));
    let record = serde_json::json!({
        "schema": LEGACY_STORE_SCHEMA,
        "key": key,
        "obligation": obligation,
        "skeleton": skeleton,
        "identity": identity,
        "entries": entries,
    });
    std::fs::create_dir_all(store.dir()).unwrap();
    std::fs::write(
        store.dir().join(format!("{key}.json")),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
}

fn asked() -> Asked {
    Asked::new(
        obligation_with("The registry holds every entry."),
        tasks(OBLIGES, LOOPHOLE),
    )
}

/// One schema-4 record of `binary` over `asked`'s exact question.
fn write_one(
    store: &VerdictStore,
    asked: &Asked,
    binary: &str,
    at: &str,
    verdict: FidelityVerdict,
) {
    let entries = vec![entry(at, &asked.texts, verdict)];
    write_legacy(store, &asked.obligation, "skeleton", binary, entries);
}

fn migrate(store: &VerdictStore, asked: &Asked) -> Option<Standing> {
    let legacy = store.legacy_records();
    store.load_or_migrate(asked.question(), &legacy)
}

#[test]
fn a_legacy_record_is_reused_and_promoted_under_the_critic_logic_key() {
    let temp = tempfile::tempdir().unwrap();
    let store = legacy_store(temp.path());
    let asked = asked();
    write_one(
        &store,
        &asked,
        "binary-X",
        "2026-10-12T10:00:00+00:00",
        supported(),
    );

    assert_eq!(store.legacy_records().len(), 1);
    assert_eq!(
        migrate(&store, &asked),
        Some(Standing::Verdict(supported()))
    );
    assert_eq!(
        records(&store),
        2,
        "legacy source and promoted schema-6 record"
    );
    assert_eq!(
        store.load(asked.question()),
        Some(Standing::Verdict(supported())),
        "the promoted record stands on its own"
    );
}

/// H1: once the critic logic version is another, a stored verdict may mean
/// something else; the scan is skipped and the obligation is asked.
#[test]
fn another_critic_logic_version_never_reads_legacy_records() {
    let temp = tempfile::tempdir().unwrap();
    let store = legacy_store(temp.path());
    let asked = asked();
    write_one(
        &store,
        &asked,
        "binary-X",
        "2026-10-12T10:00:00+00:00",
        supported(),
    );

    let changed = VerdictStore::new(
        store.dir().into(),
        identity().with_critic_logic_version(PREDECESSOR_VERSION + 1),
    );
    assert!(changed.legacy_records().is_empty(), "the scan is skipped");
    let all = legacy_store(temp.path()).legacy_records();
    assert_eq!(all.len(), 1);
    assert_eq!(changed.load_or_migrate(asked.question(), &all), None);
    assert_eq!(records(&store), 1, "nothing promoted");
}

/// M1: two deploys answered the same question over the same texts and
/// disagreed. Both are kept as a contradiction, whatever the directory order.
#[test]
fn conflicting_legacy_records_promote_one_contradiction() {
    let temp = tempfile::tempdir().unwrap();
    let store = legacy_store(temp.path());
    let asked = asked();
    write_one(
        &store,
        &asked,
        "binary-X",
        "2026-10-12T10:00:00+00:00",
        supported(),
    );
    write_one(
        &store,
        &asked,
        "binary-Y",
        "2026-10-12T11:00:00+00:00",
        refuted(),
    );

    let contradiction = Standing::Contradiction {
        refuted: refuted(),
        supported: supported(),
    };
    assert_eq!(migrate(&store, &asked), Some(contradiction.clone()));
    assert_eq!(
        store.load(asked.question()),
        Some(contradiction),
        "the contradiction is recorded, not resolved"
    );
}

/// M1: of several verdicts of one meaning, the latest by time stands, as in
/// a schema-6 record, never the first one the directory lists.
#[test]
fn the_latest_legacy_verdict_of_a_meaning_stands() {
    let temp = tempfile::tempdir().unwrap();
    let asked = asked();
    let mut earlier = supported();
    earlier.reason = "an earlier deploy's reason".into();
    for (first, second) in [("binary-A", "binary-B"), ("binary-B", "binary-A")] {
        let store = legacy_store(&temp.path().join(first));
        write_one(
            &store,
            &asked,
            first,
            "2026-10-12T12:00:00+00:00",
            supported(),
        );
        write_one(
            &store,
            &asked,
            second,
            "2026-10-12T09:00:00Z",
            earlier.clone(),
        );
        assert_eq!(
            migrate(&store, &asked),
            Some(Standing::Verdict(supported()))
        );
    }
}

#[test]
fn legacy_request_and_model_mismatches_reask() {
    let temp = tempfile::tempdir().unwrap();
    let base = legacy_store(temp.path());
    let asked = asked();
    write_one(
        &base,
        &asked,
        "binary-X",
        "2026-10-12T10:00:00+00:00",
        supported(),
    );

    let changed_request = base.for_request(Some("different-request".into()));
    assert_eq!(migrate(&changed_request, &asked), None);
    let mut changed_model = identity().with_critic_logic_version(PREDECESSOR_VERSION);
    changed_model.model = "different-model".into();
    let changed_model = VerdictStore::new(base.dir().into(), changed_model);
    assert_eq!(migrate(&changed_model, &asked), None);
    assert_eq!(migrate(&base.for_request(None), &asked), None);
}

/// A task edited since the legacy verdict was given re-asks it.
#[test]
fn a_changed_task_text_under_a_legacy_record_reasks() {
    let temp = tempfile::tempdir().unwrap();
    let store = legacy_store(temp.path());
    let before = asked();
    write_one(
        &store,
        &before,
        "binary-X",
        "2026-10-12T10:00:00+00:00",
        supported(),
    );

    let edited = Asked::new(
        before.obligation.clone(),
        tasks(OBLIGES, "Tests may use a temporary registry, or none."),
    );
    assert_eq!(migrate(&store, &edited), None);
    assert_eq!(records(&store), 1, "nothing promoted");
}

/// Another obligation, or the same one under another skeleton, is another
/// question.
#[test]
fn another_obligation_or_skeleton_under_a_legacy_record_reasks() {
    let temp = tempfile::tempdir().unwrap();
    let store = legacy_store(temp.path());
    let asked = asked();
    write_one(
        &store,
        &asked,
        "binary-X",
        "2026-10-12T10:00:00+00:00",
        supported(),
    );

    let other = Asked::new(
        obligation_with("The registry holds no entry."),
        tasks(OBLIGES, LOOPHOLE),
    );
    assert_eq!(migrate(&store, &other), None);
    let legacy = store.legacy_records();
    let reframed = Question {
        skeleton: "re-frozen skeleton",
        ..asked.question()
    };
    assert_eq!(store.load_or_migrate(reframed, &legacy), None);
    assert_eq!(records(&store), 1, "nothing promoted");
}

#[test]
fn invalid_torn_and_foreign_legacy_records_are_not_reused() {
    let temp = tempfile::tempdir().unwrap();
    let store = legacy_store(temp.path());
    let asked = asked();
    let mut no_longer_holds = supported();
    no_longer_holds.supporting_text = "text no longer present".into();
    write_one(
        &store,
        &asked,
        "binary-X",
        "2026-10-12T10:00:00+00:00",
        no_longer_holds,
    );
    std::fs::write(store.dir().join("torn.json"), b"{\"schema\":4").unwrap();

    assert_eq!(
        store.legacy_records().len(),
        1,
        "the torn file is no record"
    );
    assert_eq!(
        migrate(&store, &asked),
        None,
        "a failed check spoils the merge"
    );

    let temp = tempfile::tempdir().unwrap();
    let store = legacy_store(temp.path());
    let mut foreign = supported();
    foreign.obligation_id = "OBL-OTHER".into();
    write_one(
        &store,
        &asked,
        "binary-X",
        "2026-10-12T10:00:00+00:00",
        foreign,
    );
    write_one(
        &store,
        &asked,
        "binary-Y",
        "2026-10-12T11:00:00+00:00",
        supported(),
    );
    assert_eq!(
        migrate(&store, &asked),
        None,
        "a foreign entry spoils the merge"
    );
}

/// M3: past the scan limit there are no candidates at all, so a partial
/// scan cannot hide one side of a conflict; the gate asks, never fails.
#[test]
fn a_store_over_the_scan_limit_has_no_legacy_candidates() {
    let temp = tempfile::tempdir().unwrap();
    let store = legacy_store(temp.path());
    let asked = asked();
    write_one(
        &store,
        &asked,
        "binary-X",
        "2026-10-12T10:00:00+00:00",
        supported(),
    );
    for n in 0..2 {
        std::fs::write(store.dir().join(format!("other-{n}.json")), b"{}").unwrap();
    }
    assert_eq!(store.legacy_records_within(3).len(), 1);
    let over = store.legacy_records_within(2);
    assert!(over.is_empty());
    assert_eq!(store.load_or_migrate(asked.question(), &over), None);
}

/// M3: a record file that cannot be read stops the scan with no candidates.
#[cfg(unix)]
#[test]
fn an_unreadable_record_file_leaves_no_legacy_candidates() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let store = legacy_store(temp.path());
    let asked = asked();
    write_one(
        &store,
        &asked,
        "binary-X",
        "2026-10-12T10:00:00+00:00",
        supported(),
    );
    let locked = store.dir().join("unreadable.json");
    std::fs::write(&locked, b"{}").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let readable_anyway = std::fs::read(&locked).is_ok();
    let found = store.legacy_records();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o600)).unwrap();
    if readable_anyway {
        // Running as root: permissions cannot make it unreadable.
        assert_eq!(found.len(), 1);
    } else {
        assert!(found.is_empty());
    }
}

/// A schema-6 record supersedes every earlier one: when it no longer
/// stands, the obligation is asked; the legacy verdict is neither returned
/// nor promoted over it.
#[test]
fn a_current_record_that_no_longer_stands_is_asked_not_migrated() {
    let temp = tempfile::tempdir().unwrap();
    let store = legacy_store(temp.path());
    let asked = asked();
    let mut stale = supported();
    stale.supporting_text = "text no longer present".into();
    let entries = vec![entry("2026-10-12T12:00:00+00:00", &asked.texts, stale)];
    std::fs::create_dir_all(store.dir()).unwrap();
    store
        .write(&store.record(asked.question(), store.key(asked.question()), entries))
        .unwrap();
    write_one(
        &store,
        &asked,
        "binary-X",
        "2026-10-12T10:00:00+00:00",
        supported(),
    );

    assert_eq!(store.load(asked.question()), None);
    assert_eq!(migrate(&store, &asked), None);
    assert_eq!(records(&store), 2, "the schema-6 record is left as it was");
}

#[path = "fidelity_store_schema_five_tests.rs"]
mod schema_five;
