//! Source-section lookup uses the inventory's declaration locations.

use std::collections::BTreeSet;

/// The smallest complete section containing each requested declaration.
/// References and repeated prose cannot displace the declaration's context.
pub fn obligation_source_sections(prd: &str, ids: &[String]) -> Vec<String> {
    let locations = super::obligation_source_lines(prd);
    let prepared = super::markdown::prepare(prd, false);
    let lines: Vec<&str> = prepared.lines().collect();
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
    let mut ranges = Vec::new();
    let first = headings.first().map_or(lines.len(), |(index, _)| *index);
    if first > 0 {
        ranges.push((0, first));
    }
    for &(start, level) in &headings {
        let end = headings
            .iter()
            .find(|(index, next)| *index > start && *next <= level)
            .map_or(lines.len(), |(index, _)| *index);
        ranges.push((start, end));
    }
    let mut sections = BTreeSet::new();
    for id in ids {
        let Some(line) = locations.get(id).map(|line| line - 1) else {
            continue;
        };
        if let Some(&(start, end)) = ranges
            .iter()
            .filter(|(start, end)| *start <= line && line < *end)
            .min_by_key(|(start, end)| end - start)
        {
            sections.insert(lines[start..end].join("\n"));
        }
    }
    sections.into_iter().collect()
}
