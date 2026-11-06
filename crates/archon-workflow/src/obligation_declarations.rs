//! Declaration structure determines class; identifier spelling determines identity.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    Requirement,
    Acceptance,
    Obligation,
    Constraint,
}

pub(super) struct Row {
    pub id: String,
    pub text: String,
    pub role: Role,
    pub line: usize,
}

// Compact lowercase labels in nested lists describe a parent (for example versions).
fn nested_label(id: &str) -> bool {
    id.chars().next().is_some_and(char::is_lowercase) && id.chars().all(char::is_alphanumeric)
}

/// Lowercase identities must belong to a dominant, repeated authored family.
struct Grammar {
    prefixes: BTreeSet<String>,
}
impl Grammar {
    fn new(source: &str) -> Self {
        let mut families = BTreeMap::<String, BTreeSet<String>>::new();
        for line in source.lines() {
            let trimmed = line.trim_start();
            let candidates: Vec<_> = if trimmed.starts_with('|') {
                trimmed.trim_matches('|').split('|').collect()
            } else if is_list_item(trimmed) {
                vec![
                    continuations::list_item_body(trimmed)
                        .split_whitespace()
                        .next()
                        .unwrap_or(""),
                ]
            } else {
                Vec::new()
            };
            for candidate in candidates {
                let token = candidate
                    .trim()
                    .trim_matches(['*', '_', '`'])
                    .trim_end_matches([':', '—', '–']);
                if let Some(id) = shaped_id(token) {
                    if line.len() > trimmed.len() && nested_label(id) {
                        continue;
                    }
                    families
                        .entry(Self::prefix(id).into())
                        .or_default()
                        .insert(id.into());
                }
            }
        }
        let dominant = families
            .values()
            .map(BTreeSet::len)
            .max()
            .unwrap_or(0)
            .max(2);
        Self {
            prefixes: families
                .into_iter()
                .filter_map(|(prefix, ids)| (ids.len() == dominant).then_some(prefix))
                .collect(),
        }
    }
    fn prefix(id: &str) -> &str {
        id.trim_end_matches(char::is_numeric)
            .trim_end_matches(['-', '_', '.', ':', '/', '~', '+'])
    }
    fn id<'a>(&self, value: &'a str) -> Option<&'a str> {
        let id = shaped_id(value)?;
        (authored_id(id).is_some() || self.prefixes.contains(Self::prefix(id))).then_some(id)
    }
}

#[derive(Default)]
struct Section(Option<(usize, Role)>);
impl Section {
    fn update(&mut self, heading: &str) {
        let level = heading.chars().take_while(|ch| *ch == '#').count();
        if self.0.is_some_and(|(open, _)| level > open) {
            return;
        }
        // A document title never turns all its subsections into requirements.
        self.0 = (level >= 2)
            .then(|| heading_role(heading))
            .flatten()
            .map(|role| (level, role));
    }
    fn role(&self) -> Option<Role> {
        self.0.map(|(_, role)| role)
    }
}

fn heading_role(heading: &str) -> Option<Role> {
    if heading_excludes_obligations(heading) {
        return None;
    }
    let lower = heading.to_lowercase();
    let words: Vec<_> = lower
        .split(|ch: char| !ch.is_alphabetic())
        .filter(|s| !s.is_empty())
        .collect();
    if words.contains(&"requirement")
        || words.contains(&"requirements")
        || words.contains(&"features")
        || words
            .windows(2)
            .any(|pair| pair == ["functional", "spec"] || pair == ["user", "stories"])
    {
        Some(Role::Requirement)
    } else if words.contains(&"acceptance") {
        Some(Role::Acceptance)
    } else if words.contains(&"goals") || words.contains(&"obligations") {
        Some(Role::Obligation)
    } else {
        None
    }
}

pub(super) fn table_rows(prd: &str) -> Vec<Row> {
    tables(prd).0
}

fn tables(prd: &str) -> (Vec<Row>, Vec<String>) {
    let prepared = prepare_markdown(prd);
    let grammar = Grammar::new(&prepared);
    let mut exclusion = HeadingExclusion::default();
    let mut section = Section::default();
    let mut header: Option<Vec<&str>> = None;
    let mut rows = Vec::new();
    let mut malformed = Vec::new();
    for (index, line) in prepared.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            exclusion.update(trimmed);
            section.update(trimmed);
        }
        if !trimmed.starts_with('|') {
            header = None;
            continue;
        }
        let cells: Vec<_> = trimmed
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        let Some(headers) = &header else {
            header = Some(cells);
            continue;
        };
        if exclusion.active()
            || heading_excludes_obligations(&headers.join(" | "))
            || cells
                .iter()
                .all(|cell| cell.chars().all(|ch| "-: ".contains(ch)))
        {
            continue;
        }
        let requirement = text_column(headers, Role::Requirement);
        let acceptance = text_column(headers, Role::Acceptance);
        let role = if requirement.is_some() && acceptance.is_some() {
            section.role().unwrap_or(Role::Requirement)
        } else if acceptance.is_some() {
            Role::Acceptance
        } else if requirement.is_some() {
            Role::Requirement
        } else if header_states_obligations(
            &headers
                .iter()
                .filter(|cell| !cell.to_ascii_lowercase().ends_with(" id"))
                .copied()
                .collect::<Vec<_>>()
                .join(" | "),
        ) {
            Role::Obligation
        } else {
            section.role().unwrap_or(Role::Constraint)
        };
        let named = headers.iter().position(|cell| {
            matches!(
                cell.to_ascii_lowercase().as_str(),
                "id" | "req id"
                    | "requirement id"
                    | "#"
                    | "identifier"
                    | "key"
                    | "criterion id"
                    | "obligation id"
            )
        });
        let column = named
            .filter(|i| cells.get(*i).and_then(|cell| grammar.id(cell)).is_some())
            .or_else(|| {
                (role != Role::Constraint
                    && cells.first().and_then(|cell| grammar.id(cell)).is_some())
                .then_some(0)
            });
        let id = column
            .and_then(|column| cells.get(column))
            .and_then(|cell| grammar.id(cell));
        let Some(id) = id else {
            if role == Role::Requirement
                || column
                    .and_then(|i| cells.get(i))
                    .is_some_and(|cell| shaped_id(cell).is_some())
            {
                malformed.push(format!("table row {}: {}", index + 1, trimmed));
            }
            continue;
        };
        let text_column =
            text_column(headers, role).or_else(|| (0..cells.len()).find(|i| Some(*i) != column));
        let text = if role == Role::Constraint {
            cells
                .iter()
                .enumerate()
                .filter(|(i, _)| Some(*i) != column)
                .map(|(_, cell)| *cell)
                .collect::<Vec<_>>()
                .join(" | ")
        } else {
            text_column
                .and_then(|i| cells.get(i))
                .unwrap_or(&"")
                .to_string()
        };
        if text.is_empty() {
            malformed.push(id.to_string());
        }
        rows.push(Row {
            id: id.into(),
            text,
            role,
            line: index + 1,
        });
    }
    (rows, malformed)
}

fn text_column(headers: &[&str], role: Role) -> Option<usize> {
    headers.iter().position(|cell| {
        let lower = cell.to_ascii_lowercase();
        if lower.ends_with(" id") {
            return false;
        }
        match role {
            Role::Requirement => lower.contains("requirement"),
            Role::Acceptance => ["criterion", "criteria", "acceptance"]
                .iter()
                .any(|word| lower.contains(word)),
            Role::Obligation => ["goal", "obligation", "must"]
                .iter()
                .any(|word| lower.contains(word)),
            Role::Constraint => false,
        }
    })
}

pub(super) fn list_rows(prd: &str) -> Vec<Row> {
    let prepared = prepare_markdown(prd);
    let grammar = Grammar::new(&prepared);
    let lines: Vec<_> = prepared.lines().collect();
    let mut exclusion = HeadingExclusion::default();
    let mut section = Section::default();
    let mut rows = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            exclusion.update(trimmed);
            section.update(trimmed);
            continue;
        }
        if exclusion.active() || !is_list_item(trimmed) {
            continue;
        }
        let body = continuations::list_item_body(trimmed);
        let Some((id, first)) = constraints::definition(body) else {
            continue;
        };
        if grammar.id(id).is_none()
            || (line.len() > trimmed.len() && nested_label(id))
            || (line.len() > trimmed.len()
                && section.role() != Some(Role::Requirement)
                && constraints::explicit_definition(body).is_none())
        {
            continue;
        }
        let role = if section.role() == Some(Role::Requirement) || id.starts_with("REQ-") {
            Role::Requirement
        } else {
            Role::Constraint
        };
        if role == Role::Constraint && constraints::explicit_definition(body).is_none() {
            continue;
        }
        let mut text = vec![first.to_string()];
        continuations::append_with_ids(
            &lines,
            index + 1,
            line.len() - trimmed.len(),
            &mut text,
            |body| {
                let declaration = if role == Role::Requirement {
                    constraints::definition(body)
                } else {
                    constraints::explicit_definition(body)
                };
                declaration.is_some_and(|(id, _)| grammar.id(id).is_some() && !nested_label(id))
            },
        );
        rows.push(Row {
            id: id.into(),
            text: continuations::join(&text),
            role,
            line: index + 1,
        });
    }
    rows
}

pub(super) fn malformed_ids(prd: &str) -> Vec<String> {
    let prepared = prepare_markdown(prd);
    let grammar = Grammar::new(&prepared);
    let mut findings = tables(prd).1;
    let mut exclusion = HeadingExclusion::default();
    let mut section = Section::default();
    for line in prepared.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            exclusion.update(trimmed);
            section.update(trimmed);
        }
        if exclusion.active() || !is_list_item(trimmed) {
            continue;
        }
        let body = continuations::list_item_body(trimmed);
        let token = body
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches(['*', '_', '`'])
            .trim_end_matches([':', '—', '–']);
        let raw = body
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches(['*', '_', '`']);
        let definition_context = section.role() == Some(Role::Requirement)
            || token.starts_with("REQ-")
            || raw.ends_with([':', '—', '–'])
            || body[body.split_whitespace().next().unwrap_or("").len()..]
                .trim_start()
                .starts_with([':', '—', '–', '-']);
        if !(line.len() > trimmed.len() && nested_label(token))
            && definition_context
            && grammar.id(token).is_some()
            && constraints::definition(body).is_none()
        {
            findings.push(token.to_string());
        }
    }
    findings.sort();
    findings.dedup();
    findings
}
