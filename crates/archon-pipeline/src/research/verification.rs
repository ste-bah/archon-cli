//! Citation verification, cross-reference validation, and information density scoring.
//! Implements REQ-IMPROVE-014, REQ-IMPROVE-015, REQ-IMPROVE-016.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Citation types
// ---------------------------------------------------------------------------

/// Result of verifying a single citation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CitationStatus {
    Verified,
    /// 404/DNS = gate failure.
    Failed(String),
    /// 403/timeout = EC-PIPE-014, does NOT fail gate.
    Unverifiable(String),
}

/// A citation extracted from text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    pub url_or_doi: String,
    /// E.g. "Phase 2, agent 5".
    pub location: String,
}

/// Result of running citation verification gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitationVerificationResult {
    pub total: usize,
    pub verified: usize,
    pub failed: usize,
    pub unverifiable: usize,
    pub details: Vec<(Citation, CitationStatus)>,
    /// `false` if any `Failed`.
    pub passed: bool,
}

// ---------------------------------------------------------------------------
// Citation Verification Gate
// ---------------------------------------------------------------------------

/// Citation Verification Gate -- verifies URLs/DOIs.
/// Runs after Phase 2 and after Phase 6.
pub struct CitationVerificationGate;

impl CitationVerificationGate {
    /// Extract citations from text (URLs and DOIs).
    pub fn extract_citations(text: &str, location: &str) -> Vec<Citation> {
        let mut citations = Vec::new();

        // URL pattern
        let url_re = Regex::new(r#"https?://[^\s\)\]\}"']+"#).expect("url regex");
        for m in url_re.find_iter(text) {
            citations.push(Citation {
                url_or_doi: m.as_str().to_string(),
                location: location.to_string(),
            });
        }

        // DOI pattern -- only add if not already captured inside a URL
        let doi_re = Regex::new(r#"10\.\d{4,}/[^\s\)\]\}"']+"#).expect("doi regex");
        for m in doi_re.find_iter(text) {
            let doi = m.as_str().to_string();
            if !citations.iter().any(|c| c.url_or_doi.contains(&doi)) {
                citations.push(Citation {
                    url_or_doi: doi,
                    location: location.to_string(),
                });
            }
        }

        citations
    }

    /// Verify a single citation using the supplied verifier function.
    pub fn verify_citation(
        citation: &Citation,
        verifier: &dyn Fn(&str) -> CitationStatus,
    ) -> CitationStatus {
        verifier(&citation.url_or_doi)
    }

    /// Run the gate over all citations.
    pub fn run_gate(
        citations: &[Citation],
        verifier: &dyn Fn(&str) -> CitationStatus,
    ) -> CitationVerificationResult {
        let mut details = Vec::new();
        let mut verified: usize = 0;
        let mut failed: usize = 0;
        let mut unverifiable: usize = 0;

        for cit in citations {
            let status = Self::verify_citation(cit, verifier);
            match &status {
                CitationStatus::Verified => verified += 1,
                CitationStatus::Failed(_) => failed += 1,
                CitationStatus::Unverifiable(_) => unverifiable += 1,
            }
            details.push((cit.clone(), status));
        }

        CitationVerificationResult {
            total: citations.len(),
            verified,
            failed,
            unverifiable,
            passed: failed == 0,
            details,
        }
    }
}

// ---------------------------------------------------------------------------
// Cross-reference verification
// ---------------------------------------------------------------------------

/// Cross-reference verification for chapter/section references.
pub struct CrossReferenceVerifier;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossRefResult {
    pub total_refs: usize,
    pub valid_refs: usize,
    pub dangling_refs: Vec<String>,
    pub passed: bool,
}

impl CrossReferenceVerifier {
    /// Verify all cross-references in the text against available chapters.
    pub fn verify(text: &str, available_chapters: usize) -> CrossRefResult {
        let chapter_re = Regex::new(r"(?i)chapter\s+(\d+)").expect("chapter regex");
        let section_re = Regex::new(r"(?i)section\s+(\d+)\.(\d+)").expect("section regex");

        let mut total_refs: usize = 0;
        let mut valid_refs: usize = 0;
        let mut dangling = Vec::new();

        for cap in chapter_re.captures_iter(text) {
            let num: usize = cap[1].parse().unwrap_or(0);
            total_refs += 1;
            if num >= 1 && num <= available_chapters {
                valid_refs += 1;
            } else {
                dangling.push(format!("Chapter {}", num));
            }
        }

        for cap in section_re.captures_iter(text) {
            let chap: usize = cap[1].parse().unwrap_or(0);
            total_refs += 1;
            if chap >= 1 && chap <= available_chapters {
                valid_refs += 1;
            } else {
                dangling.push(format!("Section {}.{}", &cap[1], &cap[2]));
            }
        }

        let passed = dangling.is_empty();
        CrossRefResult {
            total_refs,
            valid_refs,
            dangling_refs: dangling,
            passed,
        }
    }
}

// ---------------------------------------------------------------------------
// Information density scorer
// ---------------------------------------------------------------------------

/// Information density scorer -- penalizes repetition, rewards unique concepts.
pub struct InformationDensityScorer;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DensityScore {
    pub word_count: usize,
    /// Unique trigrams / total trigrams.
    pub unique_ngram_ratio: f64,
    /// Citations per 1000 words.
    pub citation_density: f64,
    /// 0.0-1.0.
    pub overall_score: f64,
}

impl InformationDensityScorer {
    /// Score information density of text.
    pub fn score(text: &str) -> DensityScore {
        let words: Vec<&str> = text.split_whitespace().collect();
        let word_count = words.len();

        if word_count < 3 {
            return DensityScore {
                word_count,
                unique_ngram_ratio: 1.0,
                citation_density: 0.0,
                overall_score: 0.0,
            };
        }

        // Trigram uniqueness
        let trigrams: Vec<String> = words
            .windows(3)
            .map(|w| w.join(" ").to_lowercase())
            .collect();
        let unique_trigrams: HashSet<&String> = trigrams.iter().collect();
        let unique_ngram_ratio = if trigrams.is_empty() {
            1.0
        } else {
            unique_trigrams.len() as f64 / trigrams.len() as f64
        };

        // Citation density (URLs + DOIs per 1000 words)
        let url_count = Regex::new(r"https?://")
            .expect("url count regex")
            .find_iter(text)
            .count();
        let doi_count = Regex::new(r"10\.\d{4,}/")
            .expect("doi count regex")
            .find_iter(text)
            .count();
        let citation_density = (url_count + doi_count) as f64 * 1000.0 / word_count as f64;

        // Overall: weighted combination
        let overall_score =
            (unique_ngram_ratio * 0.7 + (citation_density / 50.0).min(1.0) * 0.3).min(1.0);

        DensityScore {
            word_count,
            unique_ngram_ratio,
            citation_density,
            overall_score,
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- extract_citations ---------------------------------------------------

    #[test]
    fn extract_citations_finds_urls() {
        let text = "See https://example.com/page and http://foo.org/bar for details.";
        let cits = CitationVerificationGate::extract_citations(text, "phase 1");
        assert_eq!(cits.len(), 2);
        assert_eq!(cits[0].url_or_doi, "https://example.com/page");
        assert_eq!(cits[1].url_or_doi, "http://foo.org/bar");
        assert_eq!(cits[0].location, "phase 1");
    }

    #[test]
    fn extract_citations_finds_dois() {
        let text = "Reference: 10.1234/some-paper.2024";
        let cits = CitationVerificationGate::extract_citations(text, "ref");
        assert_eq!(cits.len(), 1);
        assert_eq!(cits[0].url_or_doi, "10.1234/some-paper.2024");
    }

    #[test]
    fn extract_citations_deduplicates_doi_inside_url() {
        let text = "Link: https://doi.org/10.1234/paper";
        let cits = CitationVerificationGate::extract_citations(text, "loc");
        // DOI is inside the URL, so only one citation
        assert_eq!(cits.len(), 1);
        assert!(cits[0].url_or_doi.starts_with("https://"));
    }

    // -- run_gate ------------------------------------------------------------

    #[test]
    fn run_gate_404_fails_gate() {
        let cits = vec![
            Citation {
                url_or_doi: "https://example.com/good".into(),
                location: "p1".into(),
            },
            Citation {
                url_or_doi: "https://example.com/notfound".into(),
                location: "p2".into(),
            },
        ];
        let verifier = |url: &str| -> CitationStatus {
            if url.contains("notfound") {
                CitationStatus::Failed("404".into())
            } else {
                CitationStatus::Verified
            }
        };
        let result = CitationVerificationGate::run_gate(&cits, &verifier);
        assert!(!result.passed);
        assert_eq!(result.failed, 1);
        assert_eq!(result.verified, 1);
    }

    #[test]
    fn run_gate_403_passes_as_unverifiable() {
        let cits = vec![Citation {
            url_or_doi: "https://example.com/protected".into(),
            location: "p1".into(),
        }];
        let verifier =
            |_url: &str| -> CitationStatus { CitationStatus::Unverifiable("403 Forbidden".into()) };
        let result = CitationVerificationGate::run_gate(&cits, &verifier);
        assert!(result.passed); // unverifiable does NOT fail gate
        assert_eq!(result.unverifiable, 1);
        assert_eq!(result.failed, 0);
    }

    #[test]
    fn run_gate_all_verified() {
        let cits = vec![
            Citation {
                url_or_doi: "https://a.com".into(),
                location: "x".into(),
            },
            Citation {
                url_or_doi: "https://b.com".into(),
                location: "x".into(),
            },
        ];
        let verifier = |_: &str| -> CitationStatus { CitationStatus::Verified };
        let result = CitationVerificationGate::run_gate(&cits, &verifier);
        assert!(result.passed);
        assert_eq!(result.total, 2);
        assert_eq!(result.verified, 2);
    }

    #[test]
    fn run_gate_empty_citations() {
        let verifier = |_: &str| -> CitationStatus { CitationStatus::Verified };
        let result = CitationVerificationGate::run_gate(&[], &verifier);
        assert!(result.passed);
        assert_eq!(result.total, 0);
    }

    // -- cross-reference -----------------------------------------------------

    #[test]
    fn crossref_dangling_chapter_fails() {
        let text = "See Chapter 9 for details.";
        let result = CrossReferenceVerifier::verify(text, 6);
        assert!(!result.passed);
        assert_eq!(result.dangling_refs.len(), 1);
        assert_eq!(result.dangling_refs[0], "Chapter 9");
    }

    #[test]
    fn crossref_valid_chapter_passes() {
        let text = "As discussed in Chapter 3, the approach works.";
        let result = CrossReferenceVerifier::verify(text, 6);
        assert!(result.passed);
        assert_eq!(result.valid_refs, 1);
        assert_eq!(result.total_refs, 1);
    }

    #[test]
    fn crossref_section_reference() {
        let text = "Section 2.3 discusses this. Section 8.1 is also relevant.";
        let result = CrossReferenceVerifier::verify(text, 5);
        assert!(!result.passed);
        assert_eq!(result.valid_refs, 1); // Section 2.3 valid
        assert_eq!(result.dangling_refs.len(), 1); // Section 8.1 dangling
    }

    #[test]
    fn crossref_no_references() {
        let text = "This text has no chapter or section references.";
        let result = CrossReferenceVerifier::verify(text, 3);
        assert!(result.passed);
        assert_eq!(result.total_refs, 0);
    }

    // -- density scorer ------------------------------------------------------

    #[test]
    fn density_repetitive_scores_lower() {
        let repetitive = "the cat sat the cat sat the cat sat the cat sat the cat sat";
        let diverse = "quantum computing leverages superposition entanglement decoherence algorithms optimization parallel processing qubits gates circuits";
        let rep_score = InformationDensityScorer::score(repetitive);
        let div_score = InformationDensityScorer::score(diverse);
        assert!(
            div_score.unique_ngram_ratio > rep_score.unique_ngram_ratio,
            "diverse text ({}) should have higher ngram ratio than repetitive ({})",
            div_score.unique_ngram_ratio,
            rep_score.unique_ngram_ratio,
        );
        assert!(div_score.overall_score > rep_score.overall_score);
    }

    #[test]
    fn density_citations_boost_score() {
        let with_cites = "Machine learning https://arxiv.org/123 advances in deep learning https://doi.org/10.1234/paper neural networks produce remarkable results in vision tasks";
        let without_cites = "Machine learning is a field that advances in deep learning and neural networks produce remarkable results in vision tasks today";
        let with_score = InformationDensityScorer::score(with_cites);
        let without_score = InformationDensityScorer::score(without_cites);
        assert!(
            with_score.citation_density > without_score.citation_density,
            "text with citations ({}) should have higher citation density than without ({})",
            with_score.citation_density,
            without_score.citation_density,
        );
    }

    #[test]
    fn density_short_text_returns_zero_score() {
        let score = InformationDensityScorer::score("hi");
        assert_eq!(score.word_count, 1);
        assert_eq!(score.overall_score, 0.0);
    }

    #[test]
    fn density_word_count_correct() {
        let text = "one two three four five";
        let score = InformationDensityScorer::score(text);
        assert_eq!(score.word_count, 5);
    }
}
