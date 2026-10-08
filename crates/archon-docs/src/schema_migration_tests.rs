//! The image-vector dimension migration reports store errors; only an absent
//! relation counts as "nothing to migrate".
use super::*;

fn test_db() -> DbInstance {
    DbInstance::new("mem", "", Default::default()).unwrap()
}

fn fail_at(context: &'static str) -> impl FnMut(&str) -> Option<anyhow::Error> {
    move |actual| (actual == context).then(|| anyhow::anyhow!("disk I/O error (code 10)"))
}

fn stale_dim(db: &DbInstance) -> Option<usize> {
    let rows = db
        .run_script(
            "::columns vec_page_images",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .ok()?;
    format!("{:?}", rows.rows).contains("768").then_some(768)
}

fn migration_fails_at(context: &'static str) {
    let db = test_db();
    ensure_vec_page_images(&db, 768).unwrap();
    let error =
        archon_cozo::with_guarded_failure(fail_at(context), || ensure_vec_page_images(&db, 512))
            .expect_err("a store error during the migration must not be ignored");
    assert!(format!("{error:#}").contains("disk I/O error"), "{error:#}");
}

#[test]
fn a_failed_dimension_read_is_an_error_not_an_absent_relation() {
    migration_fails_at("existing vec page images dim");
}

#[test]
fn a_failed_index_drop_stops_the_migration() {
    migration_fails_at("vec_page_images index drop");
}

#[test]
fn a_failed_relation_removal_stops_the_migration() {
    migration_fails_at("vec_page_images dim migration");
    let db = test_db();
    ensure_vec_page_images(&db, 768).unwrap();
    let _ = archon_cozo::with_guarded_failure(fail_at("vec_page_images dim migration"), || {
        ensure_vec_page_images(&db, 512)
    });
    assert_eq!(
        stale_dim(&db),
        Some(768),
        "nothing reports a half migration as done"
    );
}

#[test]
fn an_absent_relation_is_created_at_the_requested_dimension() {
    let db = test_db();
    ensure_vec_page_images(&db, 512).unwrap();
    ensure_vec_page_images(&db, 512).unwrap();
    assert_eq!(stale_dim(&db), None);
}

#[test]
fn a_relation_without_its_index_still_migrates() {
    let db = test_db();
    ensure_vec_page_images(&db, 768).unwrap();
    db.run_script(
        "::hnsw drop vec_page_images:page_image_embedding_idx",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    ensure_vec_page_images(&db, 512).unwrap();
    assert_eq!(stale_dim(&db), None, "the stale relation must be recreated");
}
