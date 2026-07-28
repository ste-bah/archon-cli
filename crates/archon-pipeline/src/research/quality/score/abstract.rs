use super::{
    PhDQualityCalculator, QualityAssessment, QualityBreakdown, QualityContext, QualityTier,
};
use super::{abstract_body, contains_any, phase_weight};

impl PhDQualityCalculator {
    pub(super) fn assess_abstract_quality(
        &self,
        text: &str,
        context: &QualityContext,
    ) -> QualityAssessment {
        let body = abstract_body(text);
        let body_words = Self::count_words(&body);
        let text_lower = text.to_lowercase();
        let body_lower = body.to_lowercase();

        let content_depth = match body_words {
            0..80 => 0.03,
            80..120 => 0.08,
            120..300 => 0.23,
            300..450 => 0.18,
            _ => 0.10,
        };

        let structural_quality = [
            text_lower.contains("## abstract"),
            text_lower.contains("*keywords*:") || text_lower.contains("keywords:"),
            !text.contains("```"),
            !text.lines().any(|line| line.contains('|')),
        ]
        .into_iter()
        .filter(|present| *present)
        .count() as f64
            * 0.05;

        let research_rigor = [
            contains_any(
                &body_lower,
                &["examined", "used", "synthesized", "analysis"],
            ),
            contains_any(&body_lower, &["found", "findings", "indicate", "support"]),
            contains_any(&body_lower, &["no empirical", "cannot yet", "future work"]),
        ]
        .into_iter()
        .filter(|present| *present)
        .count() as f64
            * 0.06;

        let completeness = [
            contains_any(
                &body_lower,
                &["purpose", "paper examined", "study examined"],
            ),
            contains_any(&body_lower, &["method", "used", "synthesized", "analysis"]),
            contains_any(&body_lower, &["results", "findings", "found", "indicate"]),
            contains_any(
                &body_lower,
                &["conclude", "suggest", "implication", "future"],
            ),
        ]
        .into_iter()
        .filter(|present| *present)
        .count() as f64
            * 0.05;

        let format_quality = if text_lower.contains("*keywords*:") {
            0.09
        } else {
            0.04
        };
        let score = ((content_depth
            + structural_quality.min(0.20)
            + research_rigor.min(0.25)
            + completeness.min(0.20)
            + format_quality)
            * phase_weight(context.phase))
        .min(0.95);

        let tier = match score {
            s if s >= 0.85 => QualityTier::Excellent,
            s if s >= 0.70 => QualityTier::Good,
            s if s >= 0.50 => QualityTier::Adequate,
            _ => QualityTier::Poor,
        };
        let summary = format!(
            "{} abstract quality ({:.2}): body_words={}, content_depth={:.3}, structural={:.3}, rigor={:.3}, completeness={:.3}, format={:.3}",
            tier,
            score,
            body_words,
            content_depth,
            structural_quality.min(0.20),
            research_rigor.min(0.25),
            completeness.min(0.20),
            format_quality,
        );

        QualityAssessment {
            score,
            breakdown: QualityBreakdown {
                content_depth,
                structural_quality: structural_quality.min(0.20),
                research_rigor: research_rigor.min(0.25),
                completeness: completeness.min(0.20),
                format_quality,
            },
            tier,
            summary,
        }
    }
}
