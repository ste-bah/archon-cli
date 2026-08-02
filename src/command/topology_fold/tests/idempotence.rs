//! What a second fold must not do.
//!
//! The `ingested` marker, the upsert keys behind it, the pending-graph sweep
//! that honours the marker, and the store isolation that keeps a replay from
//! contending with anything else.

use super::*;

#[test]
fn re_folding_the_same_trace_does_not_double_count() {
    let _guard = store_lock();
    let fixture = fixture();
    write_sample_trace(&fixture.paths, "g1");

    let first = fold_graph(
        &fixture.paths,
        "g1",
        "s1",
        "fix the parser crash",
        &fixture.topology_db,
        Some(&fixture.learning_db),
        "workspace-1",
    )
    .unwrap();
    assert_eq!(first.learning_rows_written, 1);

    let second = fold_graph(
        &fixture.paths,
        "g1",
        "s1",
        "fix the parser crash",
        &fixture.topology_db,
        Some(&fixture.learning_db),
        "workspace-1",
    )
    .unwrap();

    assert!(second.already_ingested);
    assert_eq!(
        second.learning_rows_written, 0,
        "a repeat fold must produce no learning row"
    );
    assert_eq!(second.nodes_written, 0);
    assert_eq!(count_learning_rows(&fixture.learning_db), 1);
    assert_eq!(
        count_topology_rows(&fixture.topology_db, "topology_node"),
        3
    );
}

#[test]
fn a_replayed_fold_after_a_lost_marker_upserts_rather_than_duplicating() {
    // Simulates a crash between the store writes and the marker: the marker is
    // written last precisely so this case replays. It must be harmless.
    let _guard = store_lock();
    let fixture = fixture();
    write_sample_trace(&fixture.paths, "g1");

    for _ in 0..3 {
        fold_graph(
            &fixture.paths,
            "g1",
            "s1",
            "fix the parser crash",
            &fixture.topology_db,
            Some(&fixture.learning_db),
            "workspace-1",
        )
        .unwrap();
        std::fs::remove_file(fixture.paths.ingested_marker("g1")).unwrap();
    }

    assert_eq!(
        count_topology_rows(&fixture.topology_db, "topology_graph"),
        1
    );
    assert_eq!(
        count_topology_rows(&fixture.topology_db, "topology_node"),
        3
    );
    assert_eq!(
        count_topology_rows(&fixture.topology_db, "topology_outcome"),
        1
    );
    assert_eq!(
        count_learning_rows(&fixture.learning_db),
        1,
        "the learning row id is derived from the graph id, so it upserts"
    );
}

#[test]
fn folding_all_pending_skips_already_ingested_graphs() {
    let _guard = store_lock();
    let fixture = fixture();
    write_sample_trace(&fixture.paths, "g1");
    write_sample_trace(&fixture.paths, "g2");
    fixture.paths.mark_ingested("g2", "earlier").unwrap();

    let outcomes = fold_pending_blocking(
        &fixture.paths,
        "s1",
        "fix the parser crash",
        &fixture.topology_db,
        Some(&fixture.learning_db),
        "workspace-1",
    );

    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].graph_id, "g1");
    assert_eq!(count_learning_rows(&fixture.learning_db), 1);
}

#[test]
fn folding_all_pending_tolerates_a_missing_topology_root() {
    let _guard = store_lock();
    let fixture = fixture();
    let paths = TopologyPaths::at_root(fixture.root.join("never-created"));

    let outcomes = fold_pending_blocking(
        &paths,
        "s1",
        "goal",
        &fixture.topology_db,
        Some(&fixture.learning_db),
        "workspace-1",
    );
    assert!(outcomes.is_empty());
}

#[test]
fn the_topology_store_is_a_separate_database_file() {
    // The write lock key is per canonicalized path, so isolation is exactly the
    // question of whether these two resolve to different files.
    let root = std::path::Path::new("/project");
    assert_eq!(
        topology_db_path(root),
        root.join(".archon").join("topology.db")
    );
    assert_ne!(
        archon_cozo::write_lock_path_for_db(topology_db_path(root)),
        archon_cozo::write_lock_path_for_db(root.join(".archon").join("learning-state.db")),
    );
}

#[test]
fn schema_creation_is_idempotent() {
    let _guard = store_lock();
    let fixture = fixture();
    for _ in 0..3 {
        ensure_topology_schema(&fixture.topology_db).unwrap();
    }
}
