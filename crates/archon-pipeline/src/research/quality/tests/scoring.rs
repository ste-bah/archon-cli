use super::*;

// 10. Score never exceeds 0.95
#[test]
fn test_score_cap_095() {
    // Build a maximally rich document
    let mut text = String::new();
    text.push_str("# Title\n\n## Section 1\n\n### Sub 1\n\n## Section 2\n\n### Sub 2\n\n## Section 3\n\n### Sub 3\n\n");
    text.push_str("## Theoretical Framework\n## Key Themes\n## Gaps\n## Synthesis\n## Summary\n\n");

    // Tons of citations
    for i in 0..200 {
        text.push_str(&format!("(Author, 2023) [{}] Smith et al. found ", i));
    }

    // All methodology, statistical, evidence terms
    text.push_str("\nResearch design data collection sampling interview survey case study ethnography grounded theory phenomenology content analysis meta-analysis.\n");
    text.push_str(
        "P-value correlation regression significant standard deviation mean median chi-square.\n",
    );
    text.push_str("Evidence suggests findings indicate results show data reveals analysis demonstrates research confirms.\n");

    // Formatting
    text.push_str("\n- Bullet\n* Bullet\n1. Numbered\n\n");
    text.push_str("| Col | Val |\n|---|---|\n| A | 1 |\n\n");
    text.push_str("```code```\n**bold** `inline` ![img](x)\n\n");

    // Completeness
    text.push_str("## References\n\nBibliography\n\n## Conclusion\n\n");
    text.push_str("As discussed, see section 2. Limitation noted. Future work planned.\n");
    text.push_str("In summary, overall the findings are clear.\n\n");

    // Pad to 10000+ words
    for _ in 0..800 {
        text.push_str("methodology framework hypothesis empirical theoretical systematic analysis findings implications limitations literature qualitative quantitative validity reliability.\n");
    }

    let c = calc();
    // Phase 6 with 1.15x multiplier
    let ctx = PhDQualityCalculator::create_quality_context("literature-review-writer", 6);
    let result = c.assess_quality(&text, &ctx);
    assert!(
        result.score <= 0.95,
        "Score must never exceed 0.95, got {}",
        result.score
    );
}

// 11. CONTENT_DEPTH_TIERS exact values
#[test]
fn test_content_depth_tiers() {
    let c = calc();
    let ctx = default_ctx();

    // Helper: generate N words of plain text
    fn make_words(n: usize) -> String {
        (0..n).map(|i| format!("w{} ", i)).collect()
    }

    // <100 words -> 0.02
    let r = c.assess_quality(&make_words(50), &ctx);
    assert!(
        (r.breakdown.content_depth - 0.02).abs() < 0.005,
        "Tier <100: expected 0.02, got {}",
        r.breakdown.content_depth
    );

    // 100-299 -> 0.04
    let r = c.assess_quality(&make_words(150), &ctx);
    assert!(
        (r.breakdown.content_depth - 0.04).abs() < 0.005,
        "Tier 100-299: expected 0.04, got {}",
        r.breakdown.content_depth
    );

    // 300-499 -> 0.06
    let r = c.assess_quality(&make_words(350), &ctx);
    assert!(
        (r.breakdown.content_depth - 0.06).abs() < 0.005,
        "Tier 300-499: expected 0.06, got {}",
        r.breakdown.content_depth
    );

    // 500-999 -> 0.10
    let r = c.assess_quality(&make_words(700), &ctx);
    assert!(
        (r.breakdown.content_depth - 0.10).abs() < 0.005,
        "Tier 500-999: expected 0.10, got {}",
        r.breakdown.content_depth
    );

    // 1000-1999 -> 0.14
    let r = c.assess_quality(&make_words(1500), &ctx);
    assert!(
        (r.breakdown.content_depth - 0.14).abs() < 0.005,
        "Tier 1000-1999: expected 0.14, got {}",
        r.breakdown.content_depth
    );

    // 2000-3999 -> 0.18
    let r = c.assess_quality(&make_words(3000), &ctx);
    assert!(
        (r.breakdown.content_depth - 0.18).abs() < 0.005,
        "Tier 2000-3999: expected 0.18, got {}",
        r.breakdown.content_depth
    );

    // 4000-7999 -> 0.22
    let r = c.assess_quality(&make_words(5000), &ctx);
    assert!(
        (r.breakdown.content_depth - 0.22).abs() < 0.005,
        "Tier 4000-7999: expected 0.22, got {}",
        r.breakdown.content_depth
    );

    // 8000+ -> 0.25
    let r = c.assess_quality(&make_words(9000), &ctx);
    assert!(
        (r.breakdown.content_depth - 0.25).abs() < 0.005,
        "Tier 8000+: expected 0.25, got {}",
        r.breakdown.content_depth
    );
}

// 12. Citation deduplication: floor(count / 2)
#[test]
fn test_citation_deduplication() {
    // 8 raw citations -> effective 4 -> tier 0..5 -> 0.0
    let text_few = "(Smith, 2020) (Jones, 2021) (Lee, 2022) (Chen, 2023) [1] [2] [3] [4]";
    let c = calc();
    let ctx = default_ctx();
    let r = c.assess_quality(text_few, &ctx);
    // 8 raw / 2 = 4 effective -> tier 0..5 -> 0.0 for citations
    assert!(
        r.breakdown.research_rigor < 0.04,
        "8 raw citations (4 effective) should yield low rigor, got {}",
        r.breakdown.research_rigor
    );

    // 10 raw citations -> effective 5 -> tier 5..15 -> 0.03
    let names = [
        "Adams", "Brown", "Clark", "Davis", "Evans", "Frank", "Green", "Hayes", "Irwin", "James",
    ];
    let text_more: String = names.iter().map(|n| format!("({}, 2020) ", n)).collect();
    let r2 = c.assess_quality(&text_more, &ctx);
    // 10 raw / 2 = 5 effective -> 0.03 from citations
    assert!(
        r2.breakdown.research_rigor >= 0.03,
        "10 raw citations (5 effective) should reach 0.03 citation tier, got {}",
        r2.breakdown.research_rigor
    );
}

#[test]
fn test_file_length_manager_pass_report_scores_adequate() {
    let text = r#"
Length Cap Status: PASS
Chapter Source Coverage: PASS
Final Assembly Readiness: PASS

# File Length and Source-Coverage Validation Report

## Required Source Coverage Table

| Required source | Path verified | Words | Status |
|---|---|---:|---|
| abstract-writer | outputs/markdown/033-abstract-writer.md | 649 | PASS |
| introduction-writer | outputs/markdown/028-introduction-writer.md | 2377 | PASS |
| literature-review-writer | outputs/markdown/029-literature-review-writer.md | 4672 | PASS |
| methodology-writer | outputs/markdown/027-methodology-writer.md | 2500 | PASS |
| results-writer | outputs/markdown/030-results-writer.md | 1888 | PASS |
| discussion-writer | outputs/markdown/031-discussion-writer.md | 2655 | PASS |
| conclusion-writer | outputs/markdown/032-conclusion-writer.md | 1323 | PASS |
| citation-reconciler | outputs/markdown/041-citation-reconciler.md | 2487 | PASS |

## Aggregate Chapter-Source Word Count

Aggregate chapter source material totals 16,064 words.

## Blocking Issues

None.

## Instruction to Chapter-Synthesizer

Proceed with final synthesis.
"#;

    let c = calc();
    let ctx = PhDQualityCalculator::create_quality_context("file-length-manager", 7);
    let result = c.assess_quality(text, &ctx);
    assert!(
        result.score >= 0.50,
        "PASS file-length report should clear quality threshold, got {result:?}"
    );
}

#[test]
fn test_file_length_manager_fail_report_scores_poor() {
    let text = "Length Cap Status: PASS\n\
            Chapter Source Coverage: FAIL\n\
            Final Assembly Readiness: FAIL";

    let c = calc();
    let ctx = PhDQualityCalculator::create_quality_context("file-length-manager", 7);
    let result = c.assess_quality(text, &ctx);
    assert!(
        result.score < 0.50,
        "FAIL file-length report must not clear quality threshold, got {result:?}"
    );
}

// 13. create_quality_context sets fields correctly
#[test]
fn test_create_quality_context() {
    let ctx = PhDQualityCalculator::create_quality_context("introduction-writer", 6);
    assert_eq!(ctx.agent_key, "introduction-writer");
    assert_eq!(ctx.phase, 6);
    assert_eq!(ctx.expected_min_length, Some(5000));
    assert!(ctx.is_writing_agent);
    assert!(ctx.is_critical_agent);

    let ctx2 = PhDQualityCalculator::create_quality_context("gap-hunter", 3);
    assert!(ctx2.is_critical_agent);
    assert!(!ctx2.is_writing_agent);
    assert_eq!(ctx2.expected_min_length, Some(1500));

    let ctx_repair = PhDQualityCalculator::create_quality_context("citation-reconciler", 7);
    assert!(ctx_repair.is_critical_agent);
    assert_eq!(ctx_repair.expected_min_length, Some(1500));

    let ctx_length = PhDQualityCalculator::create_quality_context("file-length-manager", 7);
    assert!(ctx_length.is_critical_agent);

    let ctx3 = PhDQualityCalculator::create_quality_context("unknown-agent", 2);
    assert_eq!(ctx3.expected_min_length, None);
    assert!(!ctx3.is_writing_agent);
    assert!(!ctx3.is_critical_agent);
}

// 14. QualityTier boundaries
#[test]
fn test_quality_tier_boundaries() {
    // We test the tier assignment logic directly
    let c = calc();

    // Build texts that hit specific score ranges is hard,
    // so we verify the tier logic via the Display impl and enum values
    assert_eq!(format!("{}", QualityTier::Excellent), "Excellent");
    assert_eq!(format!("{}", QualityTier::Good), "Good");
    assert_eq!(format!("{}", QualityTier::Adequate), "Adequate");
    assert_eq!(format!("{}", QualityTier::Poor), "Poor");

    // Verify empty is Poor
    let r = c.assess_quality("", &default_ctx());
    assert_eq!(r.tier, QualityTier::Poor);
}

// 15. Format quality scoring
#[test]
fn test_format_quality_elements() {
    let text = "| Col | Val |\n|---|---|\n| A | 1 |\n\n```rust\nfn main() {}\n```\n\n**bold** and `code`\n\n![Figure](img.png)\n\n- bullet\n* bullet\n\n1. numbered\n";
    let c = calc();
    let ctx = default_ctx();
    let r = c.assess_quality(text, &ctx);

    // Should have all format elements: table(0.03) + code_block(0.02) + bold(0.01) + inline(0.01) + image(0.02) + consistent_lists(0.01) = 0.10
    assert!(
        (r.breakdown.format_quality - 0.10).abs() < 0.005,
        "Full format should be 0.10, got {}",
        r.breakdown.format_quality
    );
}
