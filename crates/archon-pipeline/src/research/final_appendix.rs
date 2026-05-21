//! Deterministic appendices for final research-paper export.

use crate::runner::PipelineSession;

use super::chapters::ChapterStructure;

pub fn appendices(session: &PipelineSession, structure: &ChapterStructure) -> String {
    [
        primary_source_appendix(session),
        chapter_map_appendix(structure),
        evidence_controls_appendix(),
    ]
    .join("\n\n")
}

fn primary_source_appendix(session: &PipelineSession) -> String {
    let source_path = extract_source_path(&session.task)
        .unwrap_or_else(|| "GKB-HLD - Match Scoring-200526-100339.pdf".to_string());
    format!(
        "## Appendix A: Primary Architecture Source Register\n\n\
         | Field | Description |\n\
         |---|---|\n\
         | Primary source | GKB Match Scoring High-Level Design PDF |\n\
         | Source path | `{source_path}` |\n\
         | Role in study | Primary architecture evidence for the GKB match scoring baseline, component boundaries, invocation patterns, configuration lifecycle, deployment assumptions, and audit/versioning concerns. |\n\
         | Evidence boundary | Treated as design evidence, not as production telemetry, benchmark evidence, security certification, or proof of regulatory approval. |\n\
         | Citation form | GSS / GKB Architecture Team. (2020). *HLD - Match Scoring* [Internal high-level design document]. Global Screening / GKB. |"
    )
}

fn chapter_map_appendix(structure: &ChapterStructure) -> String {
    let mut out = String::from(
        "## Appendix B: Locked Chapter Architecture\n\n\
         | Chapter | Title | Target words | Principal sections |\n\
         |---:|---|---:|---|\n",
    );
    for chapter in &structure.chapters {
        let sections = if chapter.sections.is_empty() {
            "Defined by dissertation architect".to_string()
        } else {
            chapter.sections.join("; ")
        };
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            chapter.number,
            escape_table_cell(&chapter.title),
            chapter.target_words,
            escape_table_cell(&sections)
        ));
    }
    out.trim_end().to_string()
}

fn evidence_controls_appendix() -> String {
    "## Appendix C: Evidence, Citation, and Claim Controls\n\n\
     This appendix defines the evidence controls applied during final assembly. \
     The HLD is the controlling source for internal GKB architecture claims. \
     Academic, standards, regulatory, architecture, and vendor sources support \
     broader claims about matching algorithms, software architecture, governance, \
     model risk, security, scalability, and market positioning. Vendor sources are \
     used as vendor-positioning evidence only. The paper does not claim empirical \
     GKB superiority, quantified false-positive reduction, production scalability, \
     or model validation without benchmark data, production telemetry, or formal \
     validation evidence. All bibliography entries are consolidated into the single \
     References section before these appendices."
        .to_string()
}

fn extract_source_path(task: &str) -> Option<String> {
    let start = task.find("/Volumes/")?;
    let tail = &task[start..];
    let end = tail.find(".pdf")? + ".pdf".len();
    Some(tail[..end].trim_matches('"').to_string())
}

fn escape_table_cell(input: &str) -> String {
    input.replace('|', "\\|").replace('\n', " ")
}
