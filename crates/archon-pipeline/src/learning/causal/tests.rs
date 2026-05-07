use super::*;
use std::collections::HashMap;

fn make_node(id: &str) -> CausalNode {
    CausalNode {
        id: id.to_string(),
        label: id.to_string(),
        node_type: "test".to_string(),
        metadata: HashMap::new(),
        created_at: 0,
        updated_at: 0,
    }
}

fn make_link(id: &str, sources: &[&str], target: &str, strength: f64) -> CausalLink {
    CausalLink {
        id: id.to_string(),
        source_ids: sources.iter().map(|s| s.to_string()).collect(),
        target_id: target.to_string(),
        strength,
        evidence: "test evidence".to_string(),
        created_at: 0,
    }
}

// 1. Add nodes and links, verify traversal A->B->C in 2 hops
#[test]
fn test_traversal_a_b_c_in_two_hops() {
    let mut graph = CausalHypergraph::new();
    graph.add_node(make_node("A"));
    graph.add_node(make_node("B"));
    graph.add_node(make_node("C"));

    graph.add_link(make_link("l1", &["A"], "B", 0.8)).unwrap();
    graph.add_link(make_link("l2", &["B"], "C", 0.9)).unwrap();

    let reachable = CausalTraversal::traverse_forward(&graph, "A", 2);
    assert!(reachable.contains(&"B".to_string()));
    assert!(reachable.contains(&"C".to_string()));
    assert_eq!(reachable.len(), 2);
}

// 2. Cycle detection: A->B->C, adding C->A rejected
#[test]
fn test_cycle_detection_rejects_cycle() {
    let mut graph = CausalHypergraph::new();
    graph.add_node(make_node("A"));
    graph.add_node(make_node("B"));
    graph.add_node(make_node("C"));

    graph.add_link(make_link("l1", &["A"], "B", 0.8)).unwrap();
    graph.add_link(make_link("l2", &["B"], "C", 0.9)).unwrap();

    // C -> A should be rejected.
    let result = graph.add_link(make_link("l3", &["C"], "A", 0.7));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("cycle"));
}

// 3. 5-hop deep chain traversal works
#[test]
fn test_five_hop_chain_traversal() {
    let mut graph = CausalHypergraph::new();
    let ids: Vec<String> = (0..6).map(|i| format!("N{i}")).collect();
    for id in &ids {
        graph.add_node(make_node(id));
    }
    // N0 -> N1 -> N2 -> N3 -> N4 -> N5
    for i in 0..5 {
        graph
            .add_link(make_link(&format!("l{i}"), &[&ids[i]], &ids[i + 1], 0.5))
            .unwrap();
    }

    let reachable = CausalTraversal::traverse_forward(&graph, "N0", 5);
    assert_eq!(reachable.len(), 5); // N1..N5
    assert!(reachable.contains(&"N5".to_string()));
}

// 4. Multi-cause hyperedge: A+B->C stored and traversable
#[test]
fn test_multi_cause_hyperedge() {
    let mut graph = CausalHypergraph::new();
    graph.add_node(make_node("A"));
    graph.add_node(make_node("B"));
    graph.add_node(make_node("C"));

    graph
        .add_link(make_link("l1", &["A", "B"], "C", 0.9))
        .unwrap();

    // Forward from A reaches C.
    let from_a = CausalTraversal::traverse_forward(&graph, "A", 1);
    assert!(from_a.contains(&"C".to_string()));

    // Forward from B also reaches C.
    let from_b = CausalTraversal::traverse_forward(&graph, "B", 1);
    assert!(from_b.contains(&"C".to_string()));

    // Backward from C reaches both A and B.
    let causes = CausalTraversal::traverse_backward(&graph, "C", 1);
    assert!(causes.contains(&"A".to_string()));
    assert!(causes.contains(&"B".to_string()));
}

// 5. Bidirectional indices correct (forward and backward)
#[test]
fn test_bidirectional_indices() {
    let mut graph = CausalHypergraph::new();
    graph.add_node(make_node("X"));
    graph.add_node(make_node("Y"));
    graph.add_node(make_node("Z"));

    graph.add_link(make_link("l1", &["X"], "Y", 0.5)).unwrap();
    graph.add_link(make_link("l2", &["Y"], "Z", 0.6)).unwrap();

    // Forward index: X has outgoing l1, Y has outgoing l2.
    assert!(graph.forward_index["X"].contains("l1"));
    assert!(graph.forward_index["Y"].contains("l2"));

    // Backward index: Y has incoming l1, Z has incoming l2.
    assert!(graph.backward_index["Y"].contains("l1"));
    assert!(graph.backward_index["Z"].contains("l2"));

    // X has no incoming links, Z has no outgoing links.
    assert!(graph.backward_index["X"].is_empty());
    assert!(graph.forward_index["Z"].is_empty());
}

// 6. Serialization roundtrip preserves graph state
#[test]
fn test_serialization_roundtrip() {
    let mut graph = CausalHypergraph::new();
    graph.add_node(make_node("A"));
    graph.add_node(make_node("B"));
    graph.add_link(make_link("l1", &["A"], "B", 0.75)).unwrap();

    let json = graph.to_json().unwrap();
    let restored = CausalHypergraph::from_json(&json).unwrap();

    assert_eq!(restored.nodes.len(), 2);
    assert_eq!(restored.links.len(), 1);
    assert!(restored.get_node("A").is_some());
    assert!(restored.get_node("B").is_some());
    assert!(restored.get_link("l1").is_some());
    assert_eq!(restored.get_link("l1").unwrap().strength, 0.75);

    // Forward/backward indices preserved.
    assert!(restored.forward_index["A"].contains("l1"));
    assert!(restored.backward_index["B"].contains("l1"));
}

// 7. CausalMemory.infer_causation returns real results
#[test]
fn test_causal_memory_infer_causation() {
    let mut mem = CausalMemory::new();
    mem.add_causal_link(&["A"], "B", 0.8, "A causes B").unwrap();
    mem.add_causal_link(&["B"], "C", 0.9, "B causes C").unwrap();

    let result = mem.infer_causation("B");

    // B has cause A, effect C.
    assert!(result.causes.contains(&"A".to_string()));
    assert!(result.effects.contains(&"C".to_string()));
    // Chain: A -> B -> C.
    assert_eq!(result.chain.len(), 3);
    assert!(result.confidence > 0.0);
}

// 8. get_stats reports correct counts
#[test]
fn test_get_stats() {
    let mut mem = CausalMemory::new();
    mem.add_causal_link(&["A"], "B", 0.8, "ev1").unwrap();
    mem.add_causal_link(&["B"], "C", 0.7, "ev2").unwrap();
    mem.add_causal_link(&["A"], "C", 0.6, "ev3").unwrap();

    let stats = mem.get_stats();
    assert_eq!(stats.node_count, 3);
    assert_eq!(stats.link_count, 3);
    // C has max fan_in of 2 (from A and B).
    assert_eq!(stats.max_fan_in, 2);
    // A has max fan_out of 2 (to B and C).
    assert_eq!(stats.max_fan_out, 2);
}

// 9. find_causes on leaf node returns empty
#[test]
fn test_find_causes_leaf_node_empty() {
    let mut mem = CausalMemory::new();
    mem.add_causal_link(&["A"], "B", 0.8, "ev").unwrap();

    // A is a root - it has no causes.
    let causes = mem.find_causes("A", 5);
    assert!(causes.is_empty());
}

// 10. find_effects on root node returns all descendants
#[test]
fn test_find_effects_root_returns_all_descendants() {
    let mut mem = CausalMemory::new();
    mem.add_causal_link(&["R"], "A", 0.8, "ev1").unwrap();
    mem.add_causal_link(&["A"], "B", 0.7, "ev2").unwrap();
    mem.add_causal_link(&["A"], "C", 0.6, "ev3").unwrap();
    mem.add_causal_link(&["B"], "D", 0.5, "ev4").unwrap();

    let effects = mem.find_effects("R", 5);
    assert_eq!(effects.len(), 4); // A, B, C, D
    assert!(effects.contains(&"A".to_string()));
    assert!(effects.contains(&"B".to_string()));
    assert!(effects.contains(&"C".to_string()));
    assert!(effects.contains(&"D".to_string()));
}
