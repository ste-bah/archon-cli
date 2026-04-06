use archon_context::cache::CacheStats;

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
