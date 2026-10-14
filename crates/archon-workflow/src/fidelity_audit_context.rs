//! Generic PRD Markdown context sent with each obligation question.

/// Include obligation source sections and separately headed policy sections.
pub(super) fn prd_acceptance_context(markdown: &str, obligation_ids: &[String]) -> String {
    let mut selected = crate::obligation_ids::obligation_source_sections(markdown, obligation_ids);
    let policy = policy_sections(markdown);
    for section in &policy {
        if !selected.contains(section) {
            selected.push(section.clone());
        }
    }
    let mut context = selected.join("\n\n");
    if policy.is_empty() {
        if !context.is_empty() {
            context.push_str("\n\n");
        }
        context.push_str("No separately headed done definition, acceptance criteria, or residual/gap policy section was found; only sections containing extracted obligations are supplied.");
    }
    context
}

fn policy_sections(markdown: &str) -> Vec<String> {
    let mut selected = Vec::new();
    let mut current = Vec::new();
    let mut include = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with('#') {
            if include && !current.is_empty() {
                selected.push(current.join("\n"));
            }
            current.clear();
            let heading = line.trim_start_matches('#').trim().to_ascii_lowercase();
            include = heading.contains("done")
                || heading.contains("criteria")
                || heading.contains("residual")
                || heading.contains("gap");
        }
        if include {
            current.push(line);
        }
    }
    if include && !current.is_empty() {
        selected.push(current.join("\n"));
    }
    selected
}
