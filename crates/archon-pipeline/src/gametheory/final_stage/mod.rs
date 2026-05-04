//! Final-stage report synthesis for game-theory pipeline.
//!
//! Takes specialist outputs, maps them to report sections via the deterministic
//! [`super::sections::sections_for_specialist`] mapping, assembles section
//! content, and combines sections into a final report with provenance footnotes.

pub mod combiner;
pub mod mapper;
pub mod scanner;
pub mod style_applier;
pub mod writer;

use std::collections::HashMap;

use super::quality::QualityCheck;

/// Result of running the final-stage assembly pipeline.
#[derive(Debug, Clone)]
pub struct FinalStageResult {
    /// The assembled report as a single markdown string.
    pub report: String,
    /// Number of sections in the report.
    pub section_count: usize,
    /// Total word count.
    pub word_count: usize,
    /// Non-fatal warnings collected during assembly.
    pub warnings: Vec<String>,
}

/// Run the full final-stage pipeline: scan → map → write → combine → style.
///
/// `specialist_outputs` maps agent_key → raw output text.
/// `quality_results` maps agent_key → quality check results.
pub fn assemble_report(
    specialist_outputs: &HashMap<String, String>,
    quality_results: &HashMap<String, Vec<QualityCheck>>,
    style_profile_id: Option<&str>,
) -> FinalStageResult {
    let warnings = Vec::new();

    // 1. Scan
    let outputs = scanner::scan_outputs(specialist_outputs, quality_results);

    // 2. Map
    let assignments = mapper::map_to_sections(&outputs);

    // 3. Write sections
    let provider = writer::SectionWriterProvider;
    let sections = provider.synthesize_sections(&assignments);

    // 4. Combine with provenance
    let report = combiner::combine_sections(&sections);

    // 5. Style (pass-through)
    let styled = style_applier::apply_style(&report, style_profile_id);

    let word_count = styled.split_whitespace().count();
    let section_count = sections.len();

    FinalStageResult {
        report: styled,
        section_count,
        word_count,
        warnings,
    }
}
