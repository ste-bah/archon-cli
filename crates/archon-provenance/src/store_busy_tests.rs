use super::*;
fn paused<T>(context: &'static str, run: impl FnOnce() -> Result<T>) {
    let error = archon_cozo::with_guarded_failure(
        move |c| {
            (c == context).then(|| {
                archon_cozo::StoreBusy {
                    context: c.into(),
                    attempts: 1,
                    detail: "acquisition paused".into(),
                }
                .into()
            })
        },
        run,
    )
    .err()
    .expect("pause must propagate");
    assert!(
        std::error::Error::source(&error)
            .and_then(|s| s.downcast_ref::<archon_cozo::StoreBusy>())
            .is_some(),
        "pause must stay typed: {error}"
    );
}
#[test]
fn schema_acquisition_pause_remains_typed() {
    let db = DbInstance::new("mem", "", "").unwrap();
    paused("provenance schema: create relation", || ensure_schema(&db));
}
#[test]
fn record_write_pause_remains_typed() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let record = ProvenanceRecord {
        record_id: "record".into(),
        artifact_id: "artifact".into(),
        artifact_type: "text".into(),
        operation: "ingest".into(),
        input_hashes: vec![],
        output_hash: "hash".into(),
        parent_record_ids: vec![],
        tool_name: None,
        agent_name: None,
        model: None,
        parameters_json: serde_json::json!({}),
        timestamp: "now".into(),
        chain_hash: "chain".into(),
    };
    paused("provenance store: insert prov_records row", || {
        insert_record(&db, &record)
    });
}
#[test]
fn edge_write_pause_remains_typed() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let edge = ProvenanceEdge::new("a", "b", ProvenanceEdgeType::DerivedFrom);
    paused("provenance store: insert prov_edges row", || {
        insert_edge(&db, &edge)
    });
}
