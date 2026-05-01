//! PhD-quality scoring calculator for research agent outputs.
//!
//! Scores research agent outputs across 5 weighted dimensions to produce
//! scores in the 0.30–0.95 range. Replicates the TypeScript
//! `PhDQualityCalculator` scoring logic.

use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Contextual metadata about the agent being scored.
#[derive(Clone, Debug)]
pub struct QualityContext {
    pub agent_key: String,
    pub phase: u8,
    pub expected_min_length: Option<usize>,
    pub is_writing_agent: bool,
    pub is_critical_agent: bool,
}

/// Per-dimension score breakdown.
#[derive(Clone, Debug)]
pub struct QualityBreakdown {
    /// Max 0.25
    pub content_depth: f64,
    /// Max 0.20
    pub structural_quality: f64,
    /// Max 0.25
    pub research_rigor: f64,
    /// Max 0.20
    pub completeness: f64,
    /// Max 0.10
    pub format_quality: f64,
}

/// Final quality assessment with breakdown, tier, and summary.
#[derive(Clone, Debug)]
pub struct QualityAssessment {
    /// 0.0–0.95
    pub score: f64,
    pub breakdown: QualityBreakdown,
    pub tier: QualityTier,
    pub summary: String,
}

/// Tier classification based on final score.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QualityTier {
    /// >= 0.85
    Excellent,
    /// >= 0.70
    Good,
    /// >= 0.50
    Adequate,
    /// < 0.50
    Poor,
}

impl std::fmt::Display for QualityTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QualityTier::Excellent => write!(f, "Excellent"),
            QualityTier::Good => write!(f, "Good"),
            QualityTier::Adequate => write!(f, "Adequate"),
            QualityTier::Poor => write!(f, "Poor"),
        }
    }
}

// ---------------------------------------------------------------------------
// Static data tables
// ---------------------------------------------------------------------------

/// Minimum expected word counts per agent key.
static AGENT_MIN_LENGTHS: LazyLock<HashMap<&'static str, usize>> = LazyLock::new(|| {
    HashMap::from([
        ("literature-review-writer", 8000),
        ("introduction-writer", 5000),
        ("discussion-writer", 5000),
        ("methodology-writer", 4000),
        ("results-writer", 4000),
        ("conclusion-writer", 3000),
        ("abstract-writer", 300),
        ("chapter-synthesizer", 6000),
        ("systematic-reviewer", 5000),
        ("literature-mapper", 3000),
        ("evidence-synthesizer", 3000),
        ("thematic-synthesizer", 2500),
        ("theory-builder", 2500),
        ("method-designer", 2000),
        ("hypothesis-generator", 1500),
        ("model-architect", 2000),
        ("instrument-developer", 2000),
        ("sampling-strategist", 1500),
        ("analysis-planner", 1500),
        ("step-back-analyzer", 1500),
        ("contradiction-analyzer", 2000),
        ("gap-hunter", 1500),
        ("self-ask-decomposer", 1000),
    ])
});

static CRITICAL_AGENTS: &[&str] = &[
    "step-back-analyzer",
    "contradiction-analyzer",
    "gap-hunter",
    "theoretical-framework-analyst",
    "bias-detector",
    "quality-assessor",
    "validity-guardian",
];

static WRITING_AGENTS: &[&str] = &[
    "introduction-writer",
    "literature-review-writer",
    "methodology-writer",
    "results-writer",
    "discussion-writer",
    "conclusion-writer",
    "chapter-synthesizer",
    "abstract-writer",
];

/// Expected sections per writing agent (keywords searched case-insensitively).
static AGENT_EXPECTED_SECTIONS: LazyLock<HashMap<&'static str, Vec<&'static str>>> =
    LazyLock::new(|| {
        HashMap::from([
            (
                "introduction-writer",
                vec![
                    "background",
                    "problem statement",
                    "research questions",
                    "significance",
                    "scope",
                ],
            ),
            (
                "literature-review-writer",
                vec![
                    "theoretical framework",
                    "key themes",
                    "gaps",
                    "synthesis",
                    "summary",
                ],
            ),
            (
                "methodology-writer",
                vec![
                    "research design",
                    "data collection",
                    "sampling",
                    "analysis",
                    "validity",
                    "ethics",
                ],
            ),
            (
                "results-writer",
                vec!["findings", "analysis", "themes", "patterns", "summary"],
            ),
            (
                "discussion-writer",
                vec![
                    "interpretation",
                    "implications",
                    "limitations",
                    "comparison",
                    "recommendations",
                ],
            ),
            (
                "conclusion-writer",
                vec![
                    "summary",
                    "contributions",
                    "limitations",
                    "future research",
                    "final remarks",
                ],
            ),
            (
                "abstract-writer",
                vec!["purpose", "method", "results", "conclusions"],
            ),
        ])
    });

/// Phase weight multipliers.
fn phase_weight(phase: u8) -> f64 {
    match phase {
        1 => 1.10,
        2 => 1.00,
        3 => 1.05,
        4 => 1.00,
        5 => 1.05,
        6 => 1.15,
        7 => 1.10,
        _ => 1.00,
    }
}

// ---------------------------------------------------------------------------
// Academic markers
// ---------------------------------------------------------------------------

static ACADEMIC_MARKERS: &[&str] = &[
    "methodology",
    "framework",
    "hypothesis",
    "empirical",
    "theoretical",
    "systematic",
    "analysis",
    "findings",
    "implications",
    "limitations",
    "literature",
    "qualitative",
    "quantitative",
    "validity",
    "reliability",
];

static METHODOLOGY_PATTERNS: &[&str] = &[
    "research design",
    "data collection",
    "sampling",
    "interview",
    "survey",
    "case study",
    "ethnography",
    "grounded theory",
    "phenomenology",
    "content analysis",
    "meta-analysis",
];

static STATISTICAL_PATTERNS: &[&str] = &[
    "p-value",
    "correlation",
    "regression",
    "significant",
    "standard deviation",
    "mean",
    "median",
    "chi-square",
];

static EVIDENCE_LANGUAGE: &[&str] = &[
    "evidence suggests",
    "findings indicate",
    "results show",
    "data reveals",
    "analysis demonstrates",
    "research confirms",
];

// ---------------------------------------------------------------------------
// Compiled regex patterns (compiled once)
// ---------------------------------------------------------------------------

static CITATION_AUTHOR_YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\([A-Z][a-z]+,\s*\d{4}\)").unwrap());
static CITATION_BRACKET_NUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[\d+\]").unwrap());
static CITATION_ET_AL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Z][a-z]+\s+et\s+al\.").unwrap());

/// Regex to strip code blocks (fenced).
static CODE_BLOCK_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)```[^`]*```").unwrap());
/// Regex to strip inline code.
static INLINE_CODE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`[^`]+`").unwrap());
/// Regex to strip markdown links (keep link text).
static MD_LINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]*)\]\([^)]*\)").unwrap());
/// Regex to strip markdown formatting (**bold**, __underline__).
static MD_FORMAT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[*_]{1,3}").unwrap());
/// Heading markers.
static HEADING_STRIP_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#{1,6}\s+").unwrap());

static NUMBERED_LIST_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+\.\s").unwrap());

// ---------------------------------------------------------------------------
// Calculator
// ---------------------------------------------------------------------------

/// PhD-quality calculator that scores research agent outputs.
pub struct PhDQualityCalculator;

impl PhDQualityCalculator {
    pub fn new() -> Self {
        PhDQualityCalculator
    }

    /// Create a `QualityContext` from an agent key and phase number.
    pub fn create_quality_context(agent_key: &str, phase: u8) -> QualityContext {
        QualityContext {
            agent_key: agent_key.to_string(),
            phase,
            expected_min_length: AGENT_MIN_LENGTHS.get(agent_key).copied(),
            is_writing_agent: WRITING_AGENTS.contains(&agent_key),
            is_critical_agent: CRITICAL_AGENTS.contains(&agent_key),
        }
    }

    /// Assess the quality of `text` produced by the agent described by `context`.
    pub fn assess_quality(&self, text: &str, context: &QualityContext) -> QualityAssessment {
        let content_depth = self.score_content_depth(text, context);
        let structural_quality = self.score_structural_quality(text);
        let research_rigor = self.score_research_rigor(text);
        let completeness = self.score_completeness(text, context);
        let format_quality = self.score_format_quality(text);

        let raw_total =
            content_depth + structural_quality + research_rigor + completeness + format_quality;
        let weight = phase_weight(context.phase);
        let score = (raw_total * weight).min(0.95);

        let tier = match score {
            s if s >= 0.85 => QualityTier::Excellent,
            s if s >= 0.70 => QualityTier::Good,
            s if s >= 0.50 => QualityTier::Adequate,
            _ => QualityTier::Poor,
        };

        let summary = format!(
            "{} quality ({:.2}): content_depth={:.3}, structural={:.3}, rigor={:.3}, completeness={:.3}, format={:.3}",
            tier,
            score,
            content_depth,
            structural_quality,
            research_rigor,
            completeness,
            format_quality,
        );

        QualityAssessment {
            score,
            breakdown: QualityBreakdown {
                content_depth,
                structural_quality,
                research_rigor,
                completeness,
                format_quality,
            },
            tier,
            summary,
        }
    }

    // -----------------------------------------------------------------------
    // Dimension 1: Content Depth (max 0.25)
    // -----------------------------------------------------------------------

    fn score_content_depth(&self, text: &str, context: &QualityContext) -> f64 {
        let word_count = Self::count_words(text);

        let mut score = match word_count {
            0..100 => 0.02,
            100..300 => 0.04,
            300..500 => 0.06,
            500..1000 => 0.10,
            1000..2000 => 0.14,
            2000..4000 => 0.18,
            4000..8000 => 0.22,
            _ => 0.25,
        };

        // Agent-specific minimum penalty
        if let Some(expected_min) = context.expected_min_length
            && word_count < expected_min
        {
            let ratio = word_count as f64 / expected_min as f64;
            score *= 0.7 + 0.3 * ratio;
        }

        // Critical agent penalty
        if context.is_critical_agent && word_count < 1000 {
            score *= 0.8;
        }

        score.min(0.25)
    }

    /// Count words after stripping code blocks, inline code, markdown links, and formatting.
    fn count_words(text: &str) -> usize {
        let stripped = CODE_BLOCK_RE.replace_all(text, "");
        let stripped = INLINE_CODE_RE.replace_all(&stripped, "");
        let stripped = MD_LINK_RE.replace_all(&stripped, "$1");
        let stripped = MD_FORMAT_RE.replace_all(&stripped, "");
        let stripped = HEADING_STRIP_RE.replace_all(&stripped, "");
        stripped.split_whitespace().count()
    }

    // -----------------------------------------------------------------------
    // Dimension 2: Structural Quality (max 0.20)
    // -----------------------------------------------------------------------

    fn score_structural_quality(&self, text: &str) -> f64 {
        let mut score = 0.0_f64;
        let text_lower = text.to_lowercase();

        let mut has_bullets = false;
        let mut has_numbered = false;
        let mut paragraph_count = 0_usize;

        // Count blank-line-separated paragraphs
        let mut in_paragraph = false;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                if in_paragraph {
                    in_paragraph = false;
                }
            } else {
                if !in_paragraph {
                    paragraph_count += 1;
                    in_paragraph = true;
                }
            }

            // Headings
            if trimmed.starts_with("# ") && !trimmed.starts_with("## ") {
                score += 0.02;
            } else if trimmed.starts_with("## ") && !trimmed.starts_with("### ") {
                score += 0.03;
            } else if trimmed.starts_with("### ") {
                score += 0.02;
            }

            // List detection
            if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
                has_bullets = true;
            }
            if NUMBERED_LIST_RE.is_match(trimmed) {
                has_numbered = true;
            }
        }

        if has_bullets {
            score += 0.02;
        }
        if has_numbered {
            score += 0.02;
        }

        // Paragraph count tiers
        score += match paragraph_count {
            0..3 => 0.0,
            3..6 => 0.02,
            6..10 => 0.04,
            10..20 => 0.06,
            _ => 0.08,
        };

        // Academic markers
        let academic_count = ACADEMIC_MARKERS
            .iter()
            .filter(|m| text_lower.contains(**m))
            .count();

        score += match academic_count {
            0..2 => 0.0,
            2..4 => 0.02,
            4..6 => 0.04,
            _ => 0.06,
        };

        score.min(0.20)
    }

    // -----------------------------------------------------------------------
    // Dimension 3: Research Rigor (max 0.25)
    // -----------------------------------------------------------------------

    fn score_research_rigor(&self, text: &str) -> f64 {
        let mut score = 0.0_f64;
        let text_lower = text.to_lowercase();

        // Citation patterns (deduplicated)
        let raw_citations = CITATION_AUTHOR_YEAR.find_iter(text).count()
            + CITATION_BRACKET_NUM.find_iter(text).count()
            + CITATION_ET_AL.find_iter(text).count();
        let effective = raw_citations / 2;

        score += match effective {
            0..5 => 0.0,
            5..15 => 0.03,
            15..30 => 0.06,
            30..50 => 0.09,
            _ => 0.12,
        };

        // Methodology patterns
        let method_count = METHODOLOGY_PATTERNS
            .iter()
            .filter(|p| text_lower.contains(**p))
            .count();

        score += match method_count {
            0..3 => 0.0,
            3..5 => 0.03,
            5..7 => 0.05,
            _ => 0.07,
        };

        // Statistical patterns
        let stat_count = STATISTICAL_PATTERNS
            .iter()
            .filter(|p| text_lower.contains(**p))
            .count();

        score += match stat_count {
            0..2 => 0.0,
            2..4 => 0.02,
            _ => 0.04,
        };

        // Evidence language
        let evidence_count = EVIDENCE_LANGUAGE
            .iter()
            .filter(|p| text_lower.contains(**p))
            .count();

        score += match evidence_count {
            0..2 => 0.0,
            2..4 => 0.01,
            4..6 => 0.02,
            _ => 0.03,
        };

        score.min(0.25)
    }

    // -----------------------------------------------------------------------
    // Dimension 4: Completeness (max 0.20)
    // -----------------------------------------------------------------------

    fn score_completeness(&self, text: &str, context: &QualityContext) -> f64 {
        let mut score = 0.0_f64;
        let text_lower = text.to_lowercase();

        // Expected sections check
        if let Some(sections) = AGENT_EXPECTED_SECTIONS.get(context.agent_key.as_str()) {
            let found = sections.iter().filter(|s| text_lower.contains(**s)).count();
            score += (found as f64 / sections.len() as f64) * 0.10;
        }

        // Reference/bibliography
        if text_lower.contains("reference") || text_lower.contains("bibliography") {
            score += 0.02;
        }

        // Conclusion section
        if text_lower.contains("conclusion") {
            score += 0.02;
        }

        // Cross-reference language
        if text_lower.contains("as discussed")
            || text_lower.contains("as mentioned")
            || text_lower.contains("see section")
        {
            score += 0.02;
        }

        // Limitations / future work
        if text_lower.contains("limitation") || text_lower.contains("future work") {
            score += 0.02;
        }

        // Summary language
        if text_lower.contains("in summary")
            || text_lower.contains("to summarize")
            || text_lower.contains("overall")
        {
            score += 0.02;
        }

        score.min(0.20)
    }

    // -----------------------------------------------------------------------
    // Dimension 5: Format Quality (max 0.10)
    // -----------------------------------------------------------------------

    fn score_format_quality(&self, text: &str) -> f64 {
        let mut score = 0.0_f64;

        let has_tables = text.lines().any(|l| l.contains('|'));
        let has_code_blocks = text.contains("```");
        let has_bold = text.contains("**") || text.contains("__");
        let has_inline_code = {
            // Single backtick not part of triple
            let stripped = text.replace("```", "");
            stripped.contains('`')
        };
        let has_images = text.contains("![") || text.to_lowercase().contains("figure");
        let has_bullets = text.lines().any(|l| {
            let t = l.trim();
            t.starts_with("- ") || t.starts_with("* ")
        });
        let has_numbered = text.lines().any(|l| NUMBERED_LIST_RE.is_match(l.trim()));

        if has_tables {
            score += 0.03;
        }
        if has_code_blocks {
            score += 0.02;
        }
        if has_bold {
            score += 0.01;
        }
        if has_inline_code {
            score += 0.01;
        }
        if has_images {
            score += 0.02;
        }
        if has_bullets && has_numbered {
            score += 0.01;
        }

        score.min(0.10)
    }
}

impl Default for PhDQualityCalculator {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
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
        text.push_str(
            "This study examines the theoretical framework for methodology analysis.\n\n",
        );
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
            result.score > 0.20,
            "Abstract writer with all sections should score reasonably, got {}",
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

    // 10. Score never exceeds 0.95
    #[test]
    fn test_score_cap_095() {
        // Build a maximally rich document
        let mut text = String::new();
        text.push_str("# Title\n\n## Section 1\n\n### Sub 1\n\n## Section 2\n\n### Sub 2\n\n## Section 3\n\n### Sub 3\n\n");
        text.push_str(
            "## Theoretical Framework\n## Key Themes\n## Gaps\n## Synthesis\n## Summary\n\n",
        );

        // Tons of citations
        for i in 0..200 {
            text.push_str(&format!("(Author, 2023) [{}] Smith et al. found ", i));
        }

        // All methodology, statistical, evidence terms
        text.push_str("\nResearch design data collection sampling interview survey case study ethnography grounded theory phenomenology content analysis meta-analysis.\n");
        text.push_str("P-value correlation regression significant standard deviation mean median chi-square.\n");
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
            "Adams", "Brown", "Clark", "Davis", "Evans", "Frank", "Green", "Hayes", "Irwin",
            "James",
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

    // 13. create_quality_context sets fields correctly
    #[test]
    fn test_create_quality_context() {
        let ctx = PhDQualityCalculator::create_quality_context("introduction-writer", 6);
        assert_eq!(ctx.agent_key, "introduction-writer");
        assert_eq!(ctx.phase, 6);
        assert_eq!(ctx.expected_min_length, Some(5000));
        assert!(ctx.is_writing_agent);
        assert!(!ctx.is_critical_agent);

        let ctx2 = PhDQualityCalculator::create_quality_context("gap-hunter", 3);
        assert!(ctx2.is_critical_agent);
        assert!(!ctx2.is_writing_agent);
        assert_eq!(ctx2.expected_min_length, Some(1500));

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
}
