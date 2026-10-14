//! Generic PRD Markdown context sent with each obligation question.

/// Include the full PRD and point each obligation at its source section.
pub(super) fn prd_acceptance_context(markdown: &str, obligation_ids: &[String]) -> String {
    let mut context = String::from("Obligation source sections:\n");
    for id in obligation_ids {
        let sections =
            crate::obligation_ids::obligation_source_sections(markdown, std::slice::from_ref(id));
        let pointers = sections
            .iter()
            .map(|section| section_heading(section))
            .collect::<Vec<_>>();
        let pointer = if pointers.is_empty() {
            "(source section not located)".to_string()
        } else {
            pointers.join("; ")
        };
        context.push_str(&format!("- {id}: {pointer}\n"));
    }
    context.push_str("\nComplete PRD text:\n");
    context.push_str(markdown);
    context
}

fn section_heading(section: &str) -> String {
    section
        .lines()
        .find(|line| line.trim_start().starts_with('#'))
        .map(str::trim)
        .map(str::to_string)
        .unwrap_or_else(|| "(PRD preamble)".to_string())
}
