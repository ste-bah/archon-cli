use archon_context::cache::{
    classify_blocks, optimize_block_order, to_api_blocks, BlockType, CacheBlock, CacheStats,
    SectionInput,
};

// ---------------------------------------------------------------------------
// Classification tests
// ---------------------------------------------------------------------------

#[test]
fn classify_static_blocks() {
    let input = SectionInput {
        identity: Some("I am Archon".into()),
        personality: Some("Friendly".into()),
        project_instructions: Some("See CLAUDE.md".into()),
        user_prompt: Some("Write code".into()),
        ..Default::default()
    };

    let blocks = classify_blocks(&input);
    for block in &blocks {
        assert_eq!(block.block_type, BlockType::Static);
    }
    assert_eq!(blocks.len(), 4);
}

#[test]
fn classify_dynamic_blocks() {
    let input = SectionInput {
        memories: Some("User prefers Rust".into()),
        rules: Some("Rule 1".into()),
        inner_voice: Some("<inner_voice>state</inner_voice>".into()),
        ..Default::default()
    };

    let blocks = classify_blocks(&input);
    for block in &blocks {
        assert_eq!(block.block_type, BlockType::Dynamic);
    }
    assert_eq!(blocks.len(), 3);
}

// ---------------------------------------------------------------------------
// Ordering tests
// ---------------------------------------------------------------------------

#[test]
fn optimize_order_static_first() {
    let blocks = vec![
        CacheBlock {
            content: "dynamic-rules".into(),
            block_type: BlockType::Dynamic,
            cache_control: None,
            content_hash: 1,
        },
        CacheBlock {
            content: "static-identity".into(),
            block_type: BlockType::Static,
            cache_control: None,
            content_hash: 2,
        },
        CacheBlock {
            content: "dynamic-memory".into(),
            block_type: BlockType::Dynamic,
            cache_control: None,
            content_hash: 3,
        },
        CacheBlock {
            content: "static-personality".into(),
            block_type: BlockType::Static,
            cache_control: None,
            content_hash: 4,
        },
    ];

    let ordered = optimize_block_order(blocks, true);
    // First two should be static, last two dynamic
    assert_eq!(ordered[0].block_type, BlockType::Static);
    assert_eq!(ordered[1].block_type, BlockType::Static);
    assert_eq!(ordered[2].block_type, BlockType::Dynamic);
    assert_eq!(ordered[3].block_type, BlockType::Dynamic);
}

#[test]
fn optimize_cache_control_on_last_static() {
    let blocks = vec![
        CacheBlock {
            content: "identity".into(),
            block_type: BlockType::Static,
            cache_control: None,
            content_hash: 1,
        },
        CacheBlock {
            content: "personality".into(),
            block_type: BlockType::Static,
            cache_control: None,
            content_hash: 2,
        },
        CacheBlock {
            content: "rules".into(),
            block_type: BlockType::Dynamic,
            cache_control: None,
            content_hash: 3,
        },
    ];

    let ordered = optimize_block_order(blocks, true);
    // Last static is index 1 (personality)
    assert!(ordered[1].cache_control.is_some());
    let cc = ordered[1].cache_control.as_ref().expect("cache_control");
    assert_eq!(cc["type"], "ephemeral");
}

#[test]
fn optimize_no_cache_control_on_dynamic() {
    let blocks = vec![
        CacheBlock {
            content: "static".into(),
            block_type: BlockType::Static,
            cache_control: None,
            content_hash: 1,
        },
        CacheBlock {
            content: "dynamic".into(),
            block_type: BlockType::Dynamic,
            cache_control: None,
            content_hash: 2,
        },
    ];

    let ordered = optimize_block_order(blocks, true);
    for block in &ordered {
        if block.block_type == BlockType::Dynamic {
            assert!(block.cache_control.is_none());
        }
    }
}

#[test]
fn optimize_disabled_no_cache_control() {
    let blocks = vec![
        CacheBlock {
            content: "identity".into(),
            block_type: BlockType::Static,
            cache_control: None,
            content_hash: 1,
        },
        CacheBlock {
            content: "rules".into(),
            block_type: BlockType::Dynamic,
            cache_control: None,
            content_hash: 2,
        },
    ];

    let ordered = optimize_block_order(blocks, false);
    for block in &ordered {
        assert!(block.cache_control.is_none());
    }
}

// ---------------------------------------------------------------------------
// CacheStats tests
// ---------------------------------------------------------------------------

#[test]
fn cache_stats_hit_rate() {
    let mut stats = CacheStats::default();
    stats.update(200, 800, 1000);
    let rate = stats.hit_rate();
    assert!((rate - 80.0).abs() < 0.01);
}

#[test]
fn cache_stats_zero_division() {
    let stats = CacheStats::default();
    assert_eq!(stats.hit_rate(), 0.0);
}

#[test]
fn cache_stats_savings() {
    let mut stats = CacheStats::default();
    // 800 cache read tokens save 90% of their input cost
    stats.update(100, 800, 1000);
    let savings = stats.estimated_savings();
    // 800 * 0.9 = 720 tokens worth of savings
    assert!((savings - 720.0).abs() < 0.01);
}

#[test]
fn cache_stats_format() {
    let mut stats = CacheStats::default();
    stats.update(100, 800, 1000);
    let formatted = stats.format_for_cost();
    assert!(formatted.contains("80.0%"));
    assert!(formatted.contains("Cache"));
}

// ---------------------------------------------------------------------------
// Content hash tests
// ---------------------------------------------------------------------------

#[test]
fn content_hash_changes() {
    let input_a = SectionInput {
        identity: Some("Hello".into()),
        ..Default::default()
    };
    let input_b = SectionInput {
        identity: Some("World".into()),
        ..Default::default()
    };

    let blocks_a = classify_blocks(&input_a);
    let blocks_b = classify_blocks(&input_b);
    assert_ne!(blocks_a[0].content_hash, blocks_b[0].content_hash);
}

#[test]
fn content_hash_same() {
    let input_a = SectionInput {
        identity: Some("Hello".into()),
        ..Default::default()
    };
    let input_b = SectionInput {
        identity: Some("Hello".into()),
        ..Default::default()
    };

    let blocks_a = classify_blocks(&input_a);
    let blocks_b = classify_blocks(&input_b);
    assert_eq!(blocks_a[0].content_hash, blocks_b[0].content_hash);
}

// ---------------------------------------------------------------------------
// API blocks format test
// ---------------------------------------------------------------------------

#[test]
fn to_api_blocks_format() {
    let blocks = vec![
        CacheBlock {
            content: "Hello world".into(),
            block_type: BlockType::Static,
            cache_control: Some(serde_json::json!({"type": "ephemeral"})),
            content_hash: 1,
        },
        CacheBlock {
            content: "Dynamic content".into(),
            block_type: BlockType::Dynamic,
            cache_control: None,
            content_hash: 2,
        },
    ];

    let api_blocks = to_api_blocks(&blocks);
    assert_eq!(api_blocks.len(), 2);

    // First block should have cache_control
    assert_eq!(api_blocks[0]["type"], "text");
    assert_eq!(api_blocks[0]["text"], "Hello world");
    assert!(api_blocks[0]["cache_control"].is_object());
    assert_eq!(api_blocks[0]["cache_control"]["type"], "ephemeral");

    // Second block should not have cache_control
    assert_eq!(api_blocks[1]["type"], "text");
    assert_eq!(api_blocks[1]["text"], "Dynamic content");
    assert!(api_blocks[1].get("cache_control").is_none());
}

// ---------------------------------------------------------------------------
// Empty input test
// ---------------------------------------------------------------------------

#[test]
fn empty_input_produces_no_blocks() {
    let input = SectionInput::default();
    let blocks = classify_blocks(&input);
    assert!(blocks.is_empty());
}
