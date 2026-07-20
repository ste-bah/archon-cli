use super::*;

fn calc() -> PhDQualityCalculator {
    PhDQualityCalculator::new()
}

fn default_ctx() -> QualityContext {
    QualityContext {
        agent_key: "test-agent".to_string(),
        phase: 2,
        expected_min_length: None,
        is_writing_agent: false,
        is_critical_agent: false,
    }
}

// 1. Empty string -> score near 0.0
#[test]
fn test_empty_string() {
    let c = calc();
    let ctx = default_ctx();
    let result = c.assess_quality("", &ctx);
    assert!(
        result.score < 0.05,
        "Empty string should score near 0, got {}",
        result.score
    );
    assert_eq!(result.tier, QualityTier::Poor);
}

// 2. 100-word plain text -> low score
#[test]
fn test_100_word_plain() {
    let words: String = (0..100).map(|i| format!("word{} ", i)).collect();
    let c = calc();
    let ctx = default_ctx();
    let result = c.assess_quality(&words, &ctx);
    assert!(
        result.score < 0.15,
        "100-word plain text should be low, got {}",
        result.score
    );
    assert_eq!(result.tier, QualityTier::Poor);
}

// 3. 500-word academic text with citations -> moderate (~0.30-0.50)
#[test]
fn test_500_word_academic_with_citations() {
    let mut text = String::new();
    text.push_str("# Introduction\n\n");
    text.push_str("## Background\n\n");
    text.push_str("This research examines the theoretical framework and methodology.\n\n");
    text.push_str("The analysis of findings reveals significant implications.\n");
    text.push_str("The literature supports a systematic approach with empirical evidence.\n\n");
    // Add citations
    for i in 0..20 {
        text.push_str(&format!(
            "(Smith, 2020) evidence suggests analysis [{}] ",
            i
        ));
    }
    text.push('\n');
    // Pad to ~500 words
    for _ in 0..40 {
        text.push_str("The methodology framework analysis findings implications limitations literature qualitative quantitative validity reliability.\n");
    }

    let c = calc();
    let ctx = default_ctx();
    let result = c.assess_quality(&text, &ctx);
    assert!(
        result.score >= 0.25 && result.score <= 0.60,
        "500-word academic text should be moderate, got {}",
        result.score
    );
}

// 4. 2000-word structured academic paper -> good (~0.60-0.75)
#[test]
fn test_2000_word_structured_paper() {
    let mut text = String::new();
    text.push_str("# Research Paper\n\n");
    text.push_str("## Introduction\n\n");
    text.push_str("### Background\n\n");
    text.push_str("This study uses a systematic methodology with a theoretical framework.\n");
    text.push_str("The hypothesis is tested using empirical analysis of findings.\n\n");
    text.push_str("## Literature Review\n\n");
    text.push_str("### Key Themes\n\n");
    text.push_str("The literature reveals qualitative and quantitative approaches.\n");
    text.push_str("Validity and reliability are central concerns.\n\n");
    text.push_str("## Methodology\n\n");
    text.push_str("### Research Design\n\n");
    text.push_str("Data collection through sampling and interview methods.\n");
    text.push_str("Survey instruments with case study and content analysis.\n\n");
    text.push_str("## Results\n\n");
    text.push_str("### Findings\n\n");
    text.push_str("Evidence suggests significant correlation (p-value < 0.05).\n");
    text.push_str("Regression analysis shows standard deviation patterns.\n");
    text.push_str("Findings indicate results show clear patterns.\n\n");

    // Citations
    for i in 0..40 {
        text.push_str(&format!(
            "(Author, 2021) [{}] Research confirms analysis demonstrates. ",
            i
        ));
    }
    text.push('\n');

    // Lists
    text.push_str("\n- First finding\n- Second finding\n- Third finding\n\n");
    text.push_str("1. Step one\n2. Step two\n3. Step three\n\n");

    // Tables
    text.push_str("| Variable | Value |\n|---|---|\n| X | 1.0 |\n\n");

    // More content
    text.push_str("## Discussion\n\n");
    text.push_str("As discussed in the methodology, the implications are significant.\n");
    text.push_str("The limitation of this approach is acknowledged.\n");
    text.push_str("In summary, the conclusion supports further research.\n\n");

    // Reference section
    text.push_str("## References\n\nBibliography entries here.\n\n");

    // Pad to ~2000 words
    for _ in 0..140 {
        text.push_str("The systematic theoretical empirical methodology framework analysis findings implications limitations literature qualitative quantitative.\n");
    }

    let c = calc();
    let ctx = default_ctx();
    let result = c.assess_quality(&text, &ctx);
    assert!(
        result.score >= 0.50 && result.score <= 0.85,
        "2000-word structured paper should score well, got {}",
        result.score
    );
}

// 5. 8000-word literature review with all sections -> excellent for Phase 6
#[test]
fn test_8000_word_lit_review_phase6() {
    let mut text = String::new();
    text.push_str("# Literature Review\n\n");
    text.push_str("## Theoretical Framework\n\n");
    text.push_str("### Key Themes\n\n");
    text.push_str("The theoretical framework provides a systematic approach.\n");
    text.push_str("Methodology and empirical analysis support the hypothesis.\n\n");
    text.push_str("## Gaps in Literature\n\n");
    text.push_str("Research gaps are identified through qualitative analysis.\n\n");
    text.push_str("## Synthesis\n\n");
    text.push_str("Evidence suggests findings indicate significant patterns.\n");
    text.push_str("Results show data reveals clear implications.\n");
    text.push_str("Analysis demonstrates research confirms the framework.\n\n");
    text.push_str("## Summary\n\n");
    text.push_str("In summary, the literature supports the research questions.\n");
    text.push_str("As discussed previously, limitations exist.\n\n");

    // Methodology terms
    text.push_str("Research design involves data collection and sampling.\n");
    text.push_str("Interview and survey methods with case study approach.\n");
    text.push_str("Content analysis and meta-analysis techniques.\n\n");

    // Statistical terms
    text.push_str("Correlation analysis with p-value significance.\n");
    text.push_str("Regression shows standard deviation and mean values.\n");
    text.push_str("Median and chi-square tests applied.\n\n");

    // Citations (many)
    for i in 0..80 {
        text.push_str(&format!("(Author, 2022) [{}] Smith et al. found that ", i));
    }
    text.push('\n');

    // Formatting
    text.push_str("\n- Finding one\n- Finding two\n* Finding three\n\n");
    text.push_str("1. First approach\n2. Second approach\n\n");
    text.push_str("| Theme | Count |\n|---|---|\n| A | 10 |\n\n");
    text.push_str("```\ncode example\n```\n\n");
    text.push_str("**Bold text** and `inline code`\n\n");
    text.push_str("![Figure 1](image.png)\n\n");

    // Conclusion, references, limitations
    text.push_str("## Conclusion\n\n");
    text.push_str("Future work should address these limitations.\n\n");
    text.push_str("## References\n\nBibliography.\n\n");

    // Pad to ~8000 words
    for _ in 0..600 {
        text.push_str("The systematic theoretical empirical methodology framework analysis findings implications limitations literature qualitative quantitative validity reliability.\n");
    }

    let c = calc();
    let ctx = PhDQualityCalculator::create_quality_context("literature-review-writer", 6);
    let result = c.assess_quality(&text, &ctx);
    assert!(
        result.score >= 0.75,
        "8000-word lit review at phase 6 should be excellent, got {}",
        result.score
    );
}

// 6. abstract-writer output (300+ words)
#[test]
fn test_abstract_writer_output() {
    let mut text = String::new();
    text.push_str("## Purpose\n\n");
    text.push_str("This study examines the theoretical framework for methodology analysis.\n\n");
    text.push_str("## Method\n\n");
    text.push_str("The research design uses systematic data collection and sampling.\n\n");
    text.push_str("## Results\n\n");
    text.push_str("Findings indicate significant correlation with p-value analysis.\n\n");
    text.push_str("## Conclusions\n\n");
    text.push_str("In summary, the evidence suggests implications for future research.\n");
    text.push_str("Limitations include scope and validity considerations.\n\n");
    // Pad to 300+ words
    for _ in 0..25 {
        text.push_str("The methodology framework systematic analysis findings implications limitations qualitative quantitative validity reliability.\n");
    }

    let c = calc();
    let ctx = PhDQualityCalculator::create_quality_context("abstract-writer", 6);
    assert!(ctx.is_writing_agent);
    assert_eq!(ctx.expected_min_length, Some(300));
    let result = c.assess_quality(&text, &ctx);
    // Should get decent score with all expected sections present
    assert!(
        result.score >= 0.50,
        "Abstract writer with all sections should score reasonably, got {}",
        result.score
    );
}

#[test]
fn apa_style_abstract_passes_abstract_gate() {
    let text = r#"# Abstract: Governing Proprietary Match Scoring

## Abstract

Financial-crime screening platforms require secure, explainable, configurable,
and operationally manageable match-scoring and alert-disposition capabilities.
This paper examined how a GKB-style architecture can support proprietary match
scoring and disposition algorithms for sanctions, anti-money laundering,
politically exposed person, and watchlist screening use cases. The study used
the GKB high-level design document as the primary architectural source and
synthesized relevant research on entity resolution, probabilistic record
linkage, explainable decision systems, model governance, secure software
architecture, and competitor capabilities. The analysis found that the proposed
architecture is best interpreted as a governed decision platform comprising an
Alert Disposition Hub, Configuration Manager, Match Scoring Adapter, model
execution service, messaging topics, asynchronous processing options, and
model-version metadata. No empirical benchmark results were available;
therefore, superiority over competitors cannot yet be claimed. Findings suggest
that proprietary advantage should come from governed evidence fusion,
explainability, configuration safety, secure deployment, and continuous
validation rather than opaque scoring alone. Future work should benchmark
accuracy, false-positive reduction, latency, scalability, analyst trust,
security resilience, and update governance.

*Keywords*: financial-crime screening, entity resolution, match scoring,
disposition algorithms, model governance
"#;

    let c = calc();
    let ctx = PhDQualityCalculator::create_quality_context("abstract-writer", 6);
    let result = c.assess_quality(text, &ctx);
    assert!(
        result.score >= 0.50,
        "concise APA-style abstract should pass, got {} ({})",
        result.score,
        result.summary
    );
}

#[test]
fn abstract_status_stub_fails_abstract_gate() {
    let text = "Completed abstract-writer output.\n\nArtifacts created:\n- abstract.md\n- executive-summary.md\n\nMemory stored.";
    let c = calc();
    let ctx = PhDQualityCalculator::create_quality_context("abstract-writer", 6);
    let result = c.assess_quality(text, &ctx);
    assert!(
        result.score < 0.50,
        "status stub must not pass abstract gate, got {}",
        result.score
    );
}

// 7. Critical agent with <1000 words -> penalty
#[test]
fn test_critical_agent_penalty() {
    let words: String = (0..500).map(|i| format!("word{} ", i)).collect();
    let c = calc();

    let ctx_normal = QualityContext {
        agent_key: "some-agent".to_string(),
        phase: 3,
        expected_min_length: None,
        is_writing_agent: false,
        is_critical_agent: false,
    };

    let ctx_critical = PhDQualityCalculator::create_quality_context("step-back-analyzer", 1);
    assert!(ctx_critical.is_critical_agent);

    let normal_result = c.assess_quality(&words, &ctx_normal);
    let critical_result = c.assess_quality(&words, &ctx_critical);

    // Critical agent with <1000 words gets penalized on content depth
    assert!(
        critical_result.breakdown.content_depth < normal_result.breakdown.content_depth
            || critical_result.breakdown.content_depth <= 0.10 * 0.8,
        "Critical agent penalty should reduce content depth"
    );
}

// 8. Phase 6 gets 1.15x multiplier
#[test]
fn test_phase6_multiplier() {
    let text = "# Heading\n\n## Sub\n\nSome methodology framework analysis findings.\n";
    let c = calc();

    let ctx_p2 = QualityContext {
        agent_key: "test".to_string(),
        phase: 2,
        expected_min_length: None,
        is_writing_agent: false,
        is_critical_agent: false,
    };
    let ctx_p6 = QualityContext {
        agent_key: "test".to_string(),
        phase: 6,
        expected_min_length: None,
        is_writing_agent: false,
        is_critical_agent: false,
    };

    let r2 = c.assess_quality(text, &ctx_p2);
    let r6 = c.assess_quality(text, &ctx_p6);

    // Phase 2 weight = 1.00, Phase 6 weight = 1.15
    // r6.score should be ~1.15x r2.score (unless capped)
    let expected_ratio = 1.15;
    let actual_ratio = r6.score / r2.score;
    assert!(
        (actual_ratio - expected_ratio).abs() < 0.01,
        "Phase 6 should multiply by 1.15, ratio was {}",
        actual_ratio
    );
}

// 9. Phase 1 gets 1.10x multiplier
#[test]
fn test_phase1_multiplier() {
    let text = "# Heading\n\n## Sub\n\nSome methodology framework analysis findings.\n";
    let c = calc();

    let ctx_p2 = QualityContext {
        agent_key: "test".to_string(),
        phase: 2,
        expected_min_length: None,
        is_writing_agent: false,
        is_critical_agent: false,
    };
    let ctx_p1 = QualityContext {
        agent_key: "test".to_string(),
        phase: 1,
        expected_min_length: None,
        is_writing_agent: false,
        is_critical_agent: false,
    };

    let r2 = c.assess_quality(text, &ctx_p2);
    let r1 = c.assess_quality(text, &ctx_p1);

    let expected_ratio = 1.10;
    let actual_ratio = r1.score / r2.score;
    assert!(
        (actual_ratio - expected_ratio).abs() < 0.01,
        "Phase 1 should multiply by 1.10, ratio was {}",
        actual_ratio
    );
}

mod scoring;
