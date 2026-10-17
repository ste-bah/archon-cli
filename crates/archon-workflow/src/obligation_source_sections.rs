//! Source-section lookup for extracted PRD obligations.

use std::collections::BTreeSet;

use super::obligation_texts;

/// The complete Markdown section containing each requested extracted
/// obligation, including synthetic numbered done items.
pub fn obligation_source_sections(prd: &str, ids: &[String]) -> Vec<String> {
    let texts = obligation_texts(prd);
    let mut sections = BTreeSet::new();
    let lines: Vec<&str> = prd.lines().collect();
    let headings: Vec<(usize, usize)> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim_start();
            trimmed
                .starts_with('#')
                .then(|| (index, trimmed.chars().take_while(|ch| *ch == '#').count()))
        })
        .collect();
    let mut candidates = Vec::new();
    match headings.first() {
        Some((first, _)) if *first > 0 => candidates.push(lines[..*first].join("\n")),
        None => candidates.push(lines.join("\n")),
        _ => {}
    }
    for (start, level) in &headings {
        let end = headings
            .iter()
            .find(|(index, next_level)| index > start && next_level <= level)
            .map_or(lines.len(), |(index, _)| *index);
        candidates.push(lines[*start..end].join("\n"));
    }
    for id in ids {
        let Some(text) = texts.get(id) else { continue };
        let normalized_text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if let Some(section) = candidates
            .iter()
            .filter(|section| {
                section
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .contains(&normalized_text)
            })
            .min_by_key(|section| section.len())
        {
            sections.insert(section.clone());
        }
    }
    sections.into_iter().collect()
}
