use super::phase_weight;
use super::{
    PhDQualityCalculator, QualityAssessment, QualityBreakdown, QualityContext, QualityTier,
};

impl PhDQualityCalculator {
    pub(super) fn assess_file_length_manager_quality(
        &self,
        text: &str,
        context: &QualityContext,
    ) -> QualityAssessment {
        let lower = text.replace("**", "").replace('`', "").to_lowercase();
        let status_pass = |label: &str| {
            lower
                .lines()
                .any(|line| line.contains(label) && line.contains("pass"))
        };
        let status_fail = |label: &str| {
            lower
                .lines()
                .any(|line| line.contains(label) && line.contains("fail"))
        };

        let required_passes = [
            status_pass("length cap status"),
            status_pass("chapter source coverage"),
            status_pass("final assembly readiness"),
        ];
        let any_failed_status = [
            status_fail("length cap status"),
            status_fail("chapter source coverage"),
            status_fail("final assembly readiness"),
        ]
        .into_iter()
        .any(|failed| failed);

        let pass_count = required_passes
            .into_iter()
            .filter(|present| *present)
            .count();
        let has_required_table = [
            "abstract-writer",
            "introduction-writer",
            "literature-review-writer",
            "methodology-writer",
            "results-writer",
            "discussion-writer",
            "conclusion-writer",
            "citation-reconciler",
        ]
        .into_iter()
        .all(|marker| lower.contains(marker));
        let has_aggregate = lower.contains("aggregate") && lower.contains("word");
        let has_blocking_issues = lower.contains("blocking issues");
        let has_final_instruction =
            lower.contains("instruction to chapter-synthesizer") || lower.contains("proceed");
        let has_path_evidence = lower.matches("outputs/markdown").count() >= 7;

        let content_depth = match Self::count_words(text) {
            0..100 => 0.04,
            100..250 => 0.08,
            250..500 => 0.14,
            _ => 0.18,
        };
        let structural_quality = self.score_structural_quality(text).clamp(0.12, 0.20);
        let research_rigor = (pass_count as f64 * 0.04
            + if has_required_table { 0.05 } else { 0.0 }
            + if has_aggregate { 0.03 } else { 0.0 }
            + if has_path_evidence { 0.03 } else { 0.0 })
        .min(0.25);
        let completeness = (pass_count as f64 * 0.04
            + if has_required_table { 0.03 } else { 0.0 }
            + if has_blocking_issues { 0.02 } else { 0.0 }
            + if has_final_instruction { 0.03 } else { 0.0 })
        .min(0.20);
        let format_quality = self.score_format_quality(text).clamp(0.06, 0.10);

        let raw_total =
            content_depth + structural_quality + research_rigor + completeness + format_quality;
        let mut score = (raw_total * phase_weight(context.phase)).min(0.95);
        if any_failed_status || pass_count < 3 {
            score = score.min(0.35);
        }

        let tier = match score {
            s if s >= 0.85 => QualityTier::Excellent,
            s if s >= 0.70 => QualityTier::Good,
            s if s >= 0.50 => QualityTier::Adequate,
            _ => QualityTier::Poor,
        };
        let summary = format!(
            "{} file-length gate quality ({:.2}): passes={}/3, content_depth={:.3}, structural={:.3}, rigor={:.3}, completeness={:.3}, format={:.3}",
            tier,
            score,
            pass_count,
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
}
