use std::collections::HashSet;

// ---------------------------------------------------------------------------
// PhDLearningIntegration - research-specific
// ---------------------------------------------------------------------------

/// Style feedback entry for a chapter.
#[derive(Debug, Clone)]
pub struct StyleFeedback {
    pub chapter: String,
    pub score: f64,
    pub issues: Vec<String>,
}

/// Research-specific learning integration for PhD pipeline.
///
/// Tracks style consistency feedback per chapter and citation quality scores.
pub struct PhDLearningIntegration {
    style_feedback: Vec<StyleFeedback>,
    citation_scores: Vec<(String, f64)>,
}

impl PhDLearningIntegration {
    pub fn new() -> Self {
        Self {
            style_feedback: Vec::new(),
            citation_scores: Vec::new(),
        }
    }

    /// Record style feedback for a chapter.
    pub fn record_style_feedback(&mut self, chapter: &str, score: f64, issues: Vec<String>) {
        self.style_feedback.push(StyleFeedback {
            chapter: chapter.to_string(),
            score,
            issues,
        });
    }

    /// Record citation quality score for an agent.
    pub fn record_citation_quality(&mut self, agent_name: &str, score: f64) {
        self.citation_scores.push((agent_name.to_string(), score));
    }

    /// Get a summary of style feedback across all chapters.
    pub fn get_style_summary(&self) -> String {
        if self.style_feedback.is_empty() {
            return "No style feedback recorded.".to_string();
        }

        let avg: f64 = self.style_feedback.iter().map(|f| f.score).sum::<f64>()
            / self.style_feedback.len() as f64;

        let all_issues: Vec<&str> = self
            .style_feedback
            .iter()
            .flat_map(|f| f.issues.iter().map(|s| s.as_str()))
            .collect();

        let unique_issues: Vec<&str> = {
            let mut seen = HashSet::new();
            all_issues.into_iter().filter(|i| seen.insert(*i)).collect()
        };

        format!(
            "Style avg={:.2}, chapters={}, issues=[{}]",
            avg,
            self.style_feedback.len(),
            unique_issues.join(", ")
        )
    }

    /// Average citation quality across all recorded scores.
    pub fn get_citation_quality_avg(&self) -> f64 {
        if self.citation_scores.is_empty() {
            return 0.0;
        }
        self.citation_scores.iter().map(|(_, s)| s).sum::<f64>() / self.citation_scores.len() as f64
    }
}

impl Default for PhDLearningIntegration {
    fn default() -> Self {
        Self::new()
    }
}
