//! Output scanner — reads in-memory specialist outputs and quality check results.

use std::collections::HashMap;

use super::super::quality::QualityCheck;

/// A scanned specialist output with quality metadata.
#[derive(Debug, Clone)]
pub struct SpecialistOutput {
    /// The agent key that produced this output.
    pub agent_key: String,
    /// The raw output text.
    pub content: String,
    /// Quality checks run against this output.
    pub quality_checks: Vec<QualityCheck>,
}

/// Scan in-memory specialist outputs into structured [`SpecialistOutput`] records.
///
/// Each entry in `outputs` becomes a `SpecialistOutput` with its corresponding
/// quality checks (or empty if none were run).
pub fn scan_outputs(
    outputs: &HashMap<String, String>,
    quality_results: &HashMap<String, Vec<QualityCheck>>,
) -> Vec<SpecialistOutput> {
    let mut scanned: Vec<SpecialistOutput> = outputs
        .iter()
        .map(|(key, content)| {
            let quality_checks = quality_results.get(key).cloned().unwrap_or_default();
            SpecialistOutput {
                agent_key: key.clone(),
                content: content.clone(),
                quality_checks,
            }
        })
        .collect();

    scanned.sort_by(|a, b| a.agent_key.cmp(&b.agent_key));
    scanned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_outputs_preserves_content() {
        let mut outputs = HashMap::new();
        outputs.insert("gt-nash".to_string(), "Equilibrium found.".to_string());
        outputs.insert("gt-payoff".to_string(), "Payoff matrix built.".to_string());

        let quality = HashMap::new();
        let scanned = scan_outputs(&outputs, &quality);

        assert_eq!(scanned.len(), 2);
        assert!(scanned.iter().any(|s| s.agent_key == "gt-nash" && s.content == "Equilibrium found."));
        assert!(scanned.iter().any(|s| s.agent_key == "gt-payoff" && s.content == "Payoff matrix built."));
    }

    #[test]
    fn test_scan_outputs_empty_input() {
        let scanned = scan_outputs(&HashMap::new(), &HashMap::new());
        assert!(scanned.is_empty());
    }
}
