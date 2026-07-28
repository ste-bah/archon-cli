use regex::Regex;
use std::sync::LazyLock;

use super::tables::{
    ACADEMIC_MARKERS, AGENT_EXPECTED_SECTIONS, AGENT_MIN_LENGTHS, CRITICAL_AGENTS,
    EVIDENCE_LANGUAGE, METHODOLOGY_PATTERNS, STATISTICAL_PATTERNS, WRITING_AGENTS,
};
use super::{QualityAssessment, QualityBreakdown, QualityContext, QualityTier};

#[path = "score/abstract.rs"]
mod abstract_quality;
mod file_length;

/// Phase weight multipliers.
pub(super) fn phase_weight(phase: u8) -> f64 {
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

pub(super) fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

pub(super) fn abstract_body(text: &str) -> String {
    let mut in_abstract = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("## abstract") {
            in_abstract = true;
            continue;
        }
        if in_abstract
            && (trimmed.starts_with("## ")
                || trimmed.starts_with("*keywords*:")
                || trimmed.starts_with("Keywords:"))
        {
            break;
        }
        if in_abstract {
            lines.push(line);
        }
    }

    let body = lines.join("\n").trim().to_string();
    if body.is_empty() {
        text.to_string()
    } else {
        body
    }
}

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
        if context.agent_key == "abstract-writer" {
            return self.assess_abstract_quality(text, context);
        }
        if context.agent_key == "file-length-manager" {
            return self.assess_file_length_manager_quality(text, context);
        }

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
    pub(super) fn count_words(text: &str) -> usize {
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

    pub(super) fn score_structural_quality(&self, text: &str) -> f64 {
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

    pub(super) fn score_format_quality(&self, text: &str) -> f64 {
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
