use super::*;

#[test]
fn atomic_delta_rejects_nan_inf_and_empty_provenance() {
    let graph = make_graph();
    let id = graph
        .store_memory("rule", "", MemoryType::Rule, 50.0, &[], "test", "")
        .expect("store rule");

    for delta in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(graph.apply_importance_delta(&id, delta, "valid").is_err());
    }
    assert!(graph.apply_importance_delta(&id, 10.0, "").is_err());
    assert_eq!(graph.get_memory(&id).expect("read rule").importance, 50.0);
}
