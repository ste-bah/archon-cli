//! PRD section keys: citation identity is independent of obligation ids.
use super::*;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrdSection {
    pub title: String,
    /// Heading plus own content, including unnumbered descendant headings.
    pub text: String,
    pub requires_citation: bool,
}

/// A section number is ASCII digits separated by dots, followed by whitespace
/// or end of title, with an optional final dot ("7. Widgets", "7.2 Widgets").
/// This is the only number grammar; citation validity is membership in this map.
fn numbered_title(title: &str) -> Option<(&str, &str)> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern =
        PATTERN.get_or_init(|| Regex::new(r"^([0-9]+(?:\.[0-9]+)*)\.?($|\s+)(.*)$").unwrap());
    let captures = pattern.captures(title)?;
    Some((captures.get(1)?.as_str(), captures.get(3)?.as_str()))
}

fn horizontal_rule(line: &str) -> bool {
    let compact: String = line.chars().filter(|ch| !ch.is_whitespace()).collect();
    compact.len() >= 3
        && ['-', '*', '_']
            .iter()
            .any(|marker| compact.chars().all(|ch| ch == *marker))
}

/// All non-excluded section keys, retaining parent-only headings as valid
/// citation targets. A leaf qualifies even when empty; a parent qualifies only
/// when its own body contains more than blank lines and horizontal rules.
/// Unnumbered descendants belong to their nearest keyed ancestor. After the
/// first numbered heading, an unnumbered heading without a keyed ancestor uses
/// its exact trimmed title as its key. Non-empty
/// preamble content is section "0", titled by the document's level-1 heading;
/// the title line itself does not count as content. Duplicate keys combine
/// titles and texts so a citation carries every occurrence.
/// Fenced code is body text. Only ATX headings (the template form) are supported;
/// Setext headings are not recognized.
pub fn numbered_sections(prd: &str) -> BTreeMap<String, PrdSection> {
    extract_sections(prd).0
}

fn extract_sections(prd: &str) -> (BTreeMap<String, PrdSection>, BTreeSet<String>) {
    let lines: Vec<_> = prd.lines().collect();
    let mut headings = Vec::new();
    let mut exclusion = HeadingExclusion::default();
    let mut fence: Option<(char, usize)> = None;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let marker = trimmed.chars().next().unwrap_or(' ');
        let count = trimmed.chars().take_while(|ch| *ch == marker).count();
        if matches!(marker, '`' | '~') && count >= 3 {
            match fence {
                None => fence = Some((marker, count)),
                Some((open, width))
                    if marker == open && count >= width && trimmed[count..].trim().is_empty() =>
                {
                    fence = None
                }
                _ => {}
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        let level = trimmed.chars().take_while(|ch| *ch == '#').count();
        if !(1..=6).contains(&level) || !trimmed[level..].starts_with(char::is_whitespace) {
            continue;
        }
        exclusion.update(trimmed);
        let title = trimmed[level..].trim().trim_end_matches('#').trim_end();
        headings.push((index, level, title, exclusion.active()));
    }
    let mut sections: BTreeMap<String, PrdSection> = BTreeMap::new();
    let mut defined = BTreeSet::new();
    let mut ancestors: Vec<(usize, String)> = Vec::new();
    let mut parents = BTreeSet::new();
    let mut content = BTreeSet::new();
    let mut preamble = Vec::new();
    let mut preamble_title = "PRD preamble";
    let mut seen_number = false;
    let first = headings.first().map_or(lines.len(), |heading| heading.0);
    preamble.extend_from_slice(&lines[..first]);
    for (slot, &(start, level, heading_title, excluded)) in headings.iter().enumerate() {
        while ancestors.last().is_some_and(|(depth, _)| *depth >= level) {
            ancestors.pop();
        }
        let end = headings
            .get(slot + 1)
            .map_or(lines.len(), |heading| heading.0);
        let numbered = numbered_title(heading_title);
        let identity = numbered.or_else(|| {
            (seen_number && ancestors.is_empty()).then_some((heading_title, heading_title))
        });
        if let Some((number, title)) = identity {
            seen_number |= numbered.is_some();
            defined.insert(number.to_string());
            if let Some((_, parent)) = ancestors.last() {
                parents.insert(parent.clone());
            }
            ancestors.push((level, number.to_string()));
            if !excluded {
                let entry = sections
                    .entry(number.to_string())
                    .or_insert_with(|| PrdSection {
                        title: String::new(),
                        text: String::new(),
                        requires_citation: false,
                    });
                if !entry.title.is_empty() {
                    entry.title.push_str(" / ");
                }
                entry.title.push_str(title);
            }
        }
        if excluded {
            continue;
        }
        let body = &lines[start + 1..end];
        let has_content = body
            .iter()
            .any(|line| !line.trim().is_empty() && !horizontal_rule(line));
        if let Some((_, number)) = ancestors.last() {
            if let Some(section) = sections.get_mut(number) {
                if has_content {
                    content.insert(number.clone());
                }
                if !section.text.is_empty() {
                    section.text.push_str("\n\n");
                }
                section.text.push_str(&lines[start..end].join("\n"));
            }
        } else if !seen_number {
            if level == 1 {
                preamble_title = heading_title;
                preamble.extend_from_slice(body);
            } else {
                preamble.extend_from_slice(&lines[start..end]);
            }
        }
    }
    for (number, section) in &mut sections {
        section.text = section.text.trim_end().to_string();
        section.requires_citation = content.contains(number) || !parents.contains(number);
    }
    if preamble
        .iter()
        .any(|line| !line.trim().is_empty() && !horizontal_rule(line))
    {
        defined.insert("0".into());
        let text = format!("# {preamble_title}\n{}", preamble.join("\n").trim());
        // An explicit section 0 shares the preamble's citation, never loses it.
        if let Some(section) = sections.get_mut("0") {
            section.title = format!("{preamble_title} / {}", section.title);
            section.text = format!("{text}\n\n{}", section.text);
            section.requires_citation = true;
        } else {
            sections.insert(
                "0".into(),
                PrdSection {
                    title: preamble_title.to_string(),
                    text,
                    requires_citation: true,
                },
            );
        }
    }
    (sections, defined)
}

/// Set-only citation findings shared by frozen/draft lint and requirements trace.
/// Parent-only headings need no finding: their children carry their paragraphs.
pub fn section_citation_findings<'a>(
    prd: &str,
    tasks: impl IntoIterator<Item = (&'a str, &'a [String])>,
) -> Vec<crate::task_skeleton::TaskSetFinding> {
    use crate::task_skeleton::TaskSetFinding;
    let (sections, defined) = extract_sections(prd);
    let mut cited = BTreeSet::new();
    let mut findings = Vec::new();
    for (task, numbers) in tasks {
        for number in numbers {
            cited.insert(number.clone());
            if !sections.contains_key(number) {
                let reason = if defined.contains(number) {
                    "is not a citable PRD section because the host excludes that section or a section it is under"
                } else {
                    "is not a section number the PRD defines"
                };
                findings.push(TaskSetFinding {
                    identity: crate::defect::DeterministicDefect::new("unknown_section", task, format!("source_sections/{number}")),
                    field: "source_sections".into(),
                    message: format!("task '{task}' source_sections cites '{number}', which {reason}; remove it or correct it to a citable PRD section key"),
                });
            }
        }
    }
    for (number, section) in sections {
        if section.requires_citation && !cited.contains(&number) {
            findings.push(TaskSetFinding {
                identity: crate::defect::DeterministicDefect::new("uncited_section", &number, "source_sections"),
                field: "source_sections".into(),
                message: format!("PRD section '{number}' ({}) is cited by no task's source_sections; cite it in at least one task: every task whose work it justifies, or any one task when it justifies none (metadata, guidance or closing statements)", section.title),
            });
        }
    }
    findings
}

#[cfg(test)]
#[path = "prd_sections_tests.rs"]
mod tests;
