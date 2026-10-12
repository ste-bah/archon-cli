//! Schema-5 records (#434d): written by the logic every schema-4 record was
//! written by, so version 1 too; promoted under it, merged with schema-4
//! records of the same question, and never read under any other version or
//! any other identity.

use super::super::super::legacy::SCHEMA_FIVE;
use super::*;

/// A verified schema-5 record under `critic_logic` for `asked`, as schema 5
/// wrote it.
fn write_schema_five(
    store: &VerdictStore,
    asked: &Asked,
    critic_logic: &str,
    at: &str,
    verdict: FidelityVerdict,
) {
    let identity = serde_json::json!({
        "critic_logic": critic_logic,
        "model": "model",
        "provider": "provider",
        "request": "representative-request",
    });
    let obligation = &asked.obligation;
    let material = serde_json::json!([SCHEMA_FIVE, obligation, "skeleton", identity]);
    let key = hex::encode(Sha256::digest(material.to_string().as_bytes()));
    let record = serde_json::json!({
        "schema": SCHEMA_FIVE,
        "key": key,
        "obligation": obligation,
        "skeleton": "skeleton",
        "identity": identity,
        "entries": vec![entry(at, &asked.texts, verdict)],
    });
    std::fs::create_dir_all(store.dir()).unwrap();
    std::fs::write(
        store.dir().join(format!("{key}.json")),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
}

/// #434d: a schema-5 record under the logic it was written by (the digest
/// 22e024436 built) is version 1, reused and promoted; under any other
/// digest it is not a candidate.
#[test]
fn a_schema_five_record_of_the_predecessor_logic_is_promoted() {
    let temp = tempfile::tempdir().unwrap();
    let store = legacy_store(temp.path());
    let asked = asked();
    write_schema_five(
        &store,
        &asked,
        "another-digest",
        "2026-10-12T09:00:00+00:00",
        refuted(),
    );
    assert!(
        store.legacy_records().is_empty(),
        "another digest is no candidate"
    );
    assert_eq!(migrate(&store, &asked), None);

    write_schema_five(
        &store,
        &asked,
        LEGACY_CRITIC_LOGIC,
        "2026-10-12T10:00:00+00:00",
        supported(),
    );
    assert_eq!(store.legacy_records().len(), 1);
    assert_eq!(
        migrate(&store, &asked),
        Some(Standing::Verdict(supported()))
    );
    assert_eq!(
        store.load(asked.question()),
        Some(Standing::Verdict(supported())),
        "promoted to a schema-6 record"
    );
    assert_eq!(records(&store), 3);
    let later = VerdictStore::new(
        store.dir().into(),
        identity().with_critic_logic_version(PREDECESSOR_VERSION + 1),
    );
    assert_eq!(
        later.load(asked.question()),
        None,
        "another version re-asks"
    );
}

/// A schema-5 and a schema-4 record that disagree over the same texts merge
/// into one contradiction; a schema-5 record whose identity differs in more
/// than its logic is no answer.
#[test]
fn schema_four_and_five_records_merge_and_must_ask_the_same_question() {
    let temp = tempfile::tempdir().unwrap();
    let store = legacy_store(temp.path());
    let asked = asked();
    write_one(
        &store,
        &asked,
        "binary-X",
        "2026-10-12T10:00:00+00:00",
        refuted(),
    );
    write_schema_five(
        &store,
        &asked,
        LEGACY_CRITIC_LOGIC,
        "2026-10-12T11:00:00+00:00",
        supported(),
    );
    let contradiction = Standing::Contradiction {
        refuted: refuted(),
        supported: supported(),
    };
    assert_eq!(migrate(&store, &asked), Some(contradiction));

    let temp = tempfile::tempdir().unwrap();
    let store = legacy_store(temp.path());
    write_schema_five(
        &store,
        &asked,
        LEGACY_CRITIC_LOGIC,
        "2026-10-12T11:00:00+00:00",
        supported(),
    );
    let mut other_model = identity().with_critic_logic_version(PREDECESSOR_VERSION);
    other_model.model = "different-model".into();
    let other_model = VerdictStore::new(store.dir().into(), other_model);
    assert_eq!(migrate(&other_model, &asked), None);
    assert_eq!(
        migrate(&store.for_request(Some("other".into())), &asked),
        None
    );
    assert_eq!(records(&store), 1, "nothing promoted");
}
