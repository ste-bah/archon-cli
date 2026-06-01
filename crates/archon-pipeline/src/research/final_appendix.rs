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
        .unwrap_or_else(|| "No explicit primary source path detected in the request.".to_string());
    format!(
        "## Appendix A: Primary Source Register\n\n\
         | Field | Description |\n\
         |---|---|\n\
         | Primary source | Current research task primary source set |\n\
         | Source path | `{source_path}` |\n\
         | Role in study | Primary evidence identified by the request and validated by the pipeline. |\n\
         | Evidence boundary | Treated according to source type: manuals and documents establish documented behaviour, forum posts establish practitioner testimony, vendor pages establish vendor claims, and translated sources require translation caution. |\n\
         | Citation form | Use the master reference list produced by the citation reconciler. |"
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
     Primary documents are controlling only for their documented scope. Public \
     forum posts and community comments are practitioner testimony, not universal \
     consensus. Vendor sources are used as vendor-positioning or product-behaviour \
     evidence only. Negative search findings mean no high-confidence evidence was \
     recovered, not that evidence cannot exist. All bibliography entries are \
     consolidated into the single References section before these appendices."
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
