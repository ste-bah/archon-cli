use archon_memory::MemoryGraph;
use archon_memory::garden::generate_briefing;
use archon_memory::types::MemoryType;

#[test]
fn generate_briefing_caps_large_memory_content() {
    let graph = MemoryGraph::in_memory().expect("create in-memory graph");
    let huge_content = format!("start-{}-end", "x".repeat(40_000));

    for i in 0..20 {
        graph
            .store_memory(
                &huge_content,
                &format!("large-memory-{i}"),
                MemoryType::Fact,
                1.0,
                &["briefing".into()],
                "test",
                "/test",
            )
            .expect("store large memory");
    }

    let briefing = generate_briefing(&graph, 20).expect("generate briefing");

    assert!(
        briefing.len() <= 16_000,
        "briefing should stay bounded, got {} bytes",
        briefing.len()
    );
    assert!(briefing.contains("start-"));
    assert!(!briefing.contains("-end"));
    assert!(briefing.ends_with("</memory_briefing>"));
}
