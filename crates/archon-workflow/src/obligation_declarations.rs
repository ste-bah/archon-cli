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

/// Tables declare their role and ID column once, in the header. Ordinary data
/// cells can neither supply an ID nor change the role of subsequent rows.
pub(super) fn table_rows(prd: &str) -> Vec<Row> {
    let prepared = prepare_markdown(prd);
    let mut exclusion = HeadingExclusion::default();
    let mut header = None;
    let mut rows = Vec::new();
    for (index, line) in prepared.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            exclusion.update(trimmed);
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
        let (id_column, role, text_column) = *header.get_or_insert_with(|| {
            let id_column = cells.iter().position(|cell| {
                matches!(
                    cell.to_ascii_lowercase().as_str(),
                    "id" | "key" | "requirement id" | "criterion id" | "obligation id"
                )
            });
            let lower = trimmed.to_ascii_lowercase();
            let role = if heading_excludes_obligations(trimmed) {
                None
            } else if ["criterion", "criteria", "acceptance"]
                .iter()
                .any(|word| lower.contains(word))
            {
                Some(Role::Acceptance)
            } else if lower.contains("requirement") {
                Some(Role::Requirement)
            } else if header_states_obligations(trimmed) {
                Some(Role::Obligation)
            } else {
                Some(Role::Constraint)
            };
            let text_column = cells.iter().position(|cell| {
                let lower = cell.to_ascii_lowercase();
                [
                    "requirement",
                    "criterion",
                    "criteria",
                    "acceptance",
                    "goal",
                    "obligation",
                    "must",
                ]
                .iter()
                .any(|word| lower.contains(word))
                    && Some(cell) != id_column.and_then(|i| cells.get(i))
            });
            (id_column, role, text_column)
        });
        if exclusion.active() {
            continue;
        }
        let (Some(column), Some(role)) = (id_column, role) else {
            continue;
        };
        let Some(id) = cells.get(column).and_then(|cell| authored_id(cell)) else {
            continue;
        };
        // The header itself is metadata, never a declaration.
        if [
            "id",
            "key",
            "requirement id",
            "criterion id",
            "obligation id",
        ]
        .contains(&id.to_ascii_lowercase().as_str())
        {
            continue;
        }
        let text = if role == Role::Constraint {
            cells
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != column)
                .map(|(_, cell)| *cell)
                .collect::<Vec<_>>()
                .join(" | ")
        } else {
            text_column
                .and_then(|i| cells.get(i))
                .map(|cell| (*cell).to_string())
                .unwrap_or_default()
        };
        rows.push(Row {
            id: id.into(),
            text,
            role,
            line: index + 1,
        });
    }
    rows
}

/// Requirement sections may contain arbitrary authored IDs. The legacy adapter
/// preserves unsectioned requirement bullets in documents predating sections.
/// Other definition bullets remain constraints, as before format generalisation.
pub(super) fn list_rows(prd: &str) -> Vec<Row> {
    let prepared = prepare_markdown(prd);
    let lines: Vec<_> = prepared.lines().collect();
    let mut exclusion = HeadingExclusion::default();
    let mut requirement_level = None;
    let mut rows = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            exclusion.update(trimmed);
            let level = trimmed.chars().take_while(|ch| *ch == '#').count();
            if requirement_level.is_none_or(|open| level <= open) {
                let lower = trimmed.to_ascii_lowercase();
                requirement_level =
                    (lower.contains("requirement") && !lower.contains("document")).then_some(level);
            }
            continue;
        }
        if exclusion.active() || !is_list_item(trimmed) {
            continue;
        }
        let body = continuations::list_item_body(trimmed);
        let Some((id, first)) = constraints::definition(body) else {
            continue;
        };
        let role = if requirement_level.is_some() || legacy_requirement(id) {
            Role::Requirement
        } else {
            Role::Constraint
        };
        let mut text = vec![first.to_string()];
        continuations::append(&lines, index + 1, line.len() - trimmed.len(), &mut text);
        rows.push(Row {
            id: id.into(),
            text: continuations::join(&text),
            role,
            line: index + 1,
        });
    }
    rows
}

fn legacy_requirement(id: &str) -> bool {
    id.starts_with("REQ-")
}
