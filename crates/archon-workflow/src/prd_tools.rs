//! Tool declarations and links derived solely from document and configuration input.
use crate::obligation_ids::{constraint_texts, obligation_texts};
use crate::tool_declarations::raw_tool_name;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

// Template grammar, like the Acceptance Criteria heading.
pub const REQUIRED_TOOLS_HEADING: &str = "Required Tools";

/// Issue-40: a Focused Tests token is an MCP invocation only when it carries a complete
/// `mcp__<server>__<tool>` or `mcp_action:<name>` identifier, returned as `(server, name)`.
/// A bare server prefix left by a glob such as `mcp__srv__*` is prose, not an obligation.
pub fn complete_mcp_tool(token: &str) -> Option<(Option<&str>, &str)> {
    if let Some(rest) = token.strip_prefix("mcp__") {
        let (server, tool) = rest.split_once("__")?;
        return (!server.is_empty() && !tool.is_empty()).then_some((Some(server), tool));
    }
    let name = token.strip_prefix("mcp_action:")?;
    (!name.is_empty()).then_some((None, name))
}

pub fn tool_matches(declared: &str, qualified: &str) -> bool {
    if declared.starts_with("mcp__") {
        declared == qualified
    } else {
        raw_tool_name(declared) == raw_tool_name(qualified)
    }
}

/// Complete qualified identifiers, including identifiers in examples.
pub fn prd_mcp_tokens(text: &str) -> BTreeSet<String> {
    let mut tokens = BTreeSet::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if !token_char(ch) {
            continue;
        }
        let mut end = start + ch.len_utf8();
        while let Some(&(index, ch)) = chars.peek() {
            if !token_char(ch) {
                break;
            }
            end = index + ch.len_utf8();
            chars.next();
        }
        if chars.peek().is_some_and(|(_, ch)| *ch == '*') {
            continue;
        }
        let token = text[start..end].trim_end_matches(['.', '-']);
        if let Some((Some(_), tool)) = complete_mcp_tool(token)
            && !tool.ends_with('_')
        {
            tokens.insert(token.to_owned());
        }
    }
    tokens
}
fn token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrdToolDeclaration {
    pub section_line: Option<usize>,
    pub tokens: BTreeMap<String, BTreeSet<String>>,
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let line = line.trim();
    let level = line.bytes().take_while(|ch| *ch == b'#').count();
    let rest = line.get(level..)?;
    (level >= 1 && rest.starts_with(char::is_whitespace))
        .then_some((level, rest.trim().trim_end_matches('#').trim()))
}
// Fenced examples remain discoverable tokens, but never declare a section.
fn document_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut fence = None;
    text.lines().enumerate().filter(move |(_, line)| {
        let trimmed = line.trim_start();
        let marker = trimmed.chars().next().unwrap_or_default();
        let width = trimmed.chars().take_while(|ch| *ch == marker).count();
        if let Some((open, minimum)) = fence {
            if marker == open && width >= minimum && trimmed[width..].trim().is_empty() {
                fence = None;
            }
            return false;
        }
        if matches!(marker, '`' | '~') && width >= 3 {
            fence = Some((marker, width));
            return false;
        }
        true
    })
}

fn required_heading(title: &str) -> bool {
    let title = if let Some((number, rest)) = title.split_once(char::is_whitespace) {
        let number = number.strip_suffix('.').unwrap_or(number);
        if number
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|ch| ch.is_ascii_digit()))
        {
            rest.trim()
        } else {
            title
        }
    } else {
        title
    };
    title.eq_ignore_ascii_case(REQUIRED_TOOLS_HEADING)
}
fn contains_id(text: &str, id: &str) -> bool {
    text.split(|ch: char| !ch.is_alphanumeric() && ch != '-' && ch != '_')
        .any(|word| word == id)
}

pub fn prd_tool_declaration(prd: &str) -> PrdToolDeclaration {
    let texts = id_texts(prd);
    let mut tokens: BTreeMap<_, BTreeSet<String>> = prd_mcp_tokens(prd)
        .into_iter()
        .map(|token| (token, BTreeSet::new()))
        .collect();
    for (id, text) in &texts {
        for token in prd_mcp_tokens(text) {
            tokens.entry(token).or_default().insert(id.clone());
        }
    }
    let mut section_line = None;
    let mut section_level = None;
    for (index, line) in document_lines(prd) {
        if let Some((level, title)) = heading(line) {
            if section_level.is_some_and(|start| level <= start) {
                break;
            }
            if section_line.is_none() && level >= 2 && required_heading(title) {
                section_line = Some(index + 1);
                section_level = Some(level);
            }
        }
        if section_level.is_some() {
            for token in prd_mcp_tokens(line) {
                tokens
                    .entry(token)
                    .or_default()
                    .extend(texts.keys().filter(|id| contains_id(line, id)).cloned());
            }
        }
    }
    PrdToolDeclaration {
        section_line,
        tokens,
    }
}

pub fn id_texts(prd: &str) -> BTreeMap<String, String> {
    let mut texts = obligation_texts(prd);
    texts.extend(constraint_texts(prd));
    texts
}

/// Only section declarations and identifiers in an obligation's text can refuse.
pub fn actionable_tokens(prd: &str) -> BTreeSet<String> {
    let mut tokens = BTreeSet::new();
    for text in id_texts(prd).values() {
        tokens.extend(prd_mcp_tokens(text));
    }
    let mut active = None;
    for (_, line) in document_lines(prd) {
        if let Some((level, title)) = heading(line) {
            if active.is_some_and(|start| level <= start) {
                break;
            }
            if active.is_none() && level >= 2 && required_heading(title) {
                active = Some(level);
            }
        }
        if active.is_some() {
            tokens.extend(prd_mcp_tokens(line));
        }
    }
    tokens
}

/// Configured names match ASCII case-insensitively in an explicit server/tool context.
pub fn text_names_server(text: &str, server: &str) -> bool {
    if server.is_empty() {
        return false;
    }
    let folded_text = text.to_ascii_lowercase();
    let folded_server = server.to_ascii_lowercase();
    folded_text.match_indices(&folded_server).any(|(start, _)| {
        let before = &folded_text[..start];
        let after = &folded_text[start + server.len()..];
        let boundary = |ch: char| ch.is_alphanumeric() || matches!(ch, '-' | '_');
        if before.chars().next_back().is_some_and(boundary)
            || after.chars().next().is_some_and(boundary)
        {
            return false;
        }
        let context = |word: &str| matches!(word, "server" | "tool" | "tools" | "mcp");
        (before.ends_with('`') && after.starts_with('`'))
            || before
                .trim_end_matches(|ch: char| !ch.is_alphanumeric())
                .rsplit(|ch: char| !ch.is_alphanumeric())
                .next()
                .is_some_and(context)
            || after
                .trim_start_matches(|ch: char| !ch.is_alphanumeric())
                .split(|ch: char| !ch.is_alphanumeric())
                .next()
                .is_some_and(context)
    })
}

pub fn derived_server_links(
    prd: &str,
    permitted: &BTreeSet<String>,
) -> BTreeMap<String, BTreeSet<String>> {
    let servers: BTreeSet<_> = permitted
        .iter()
        .filter_map(|token| complete_mcp_tool(token).and_then(|(server, _)| server))
        .collect();
    servers
        .into_iter()
        .map(|server| {
            let links = id_texts(prd)
                .into_iter()
                .filter(|(_, text)| text_names_server(text, server))
                .map(|(id, _)| id)
                .collect();
            (server.to_owned(), links)
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrdToolDefect {
    Undeclared { permitted: Vec<String> },
    NotPermitted { token: String },
    Unlinked { token: String },
}
impl PrdToolDefect {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Undeclared { .. } => "prd_tools_undeclared",
            Self::NotPermitted { .. } => "prd_tool_not_permitted",
            Self::Unlinked { .. } => "prd_tool_unlinked",
        }
    }
    pub fn message(&self, prd: &Path, mcp: &Path) -> String {
        let prd = prd.display();
        let mcp = mcp.display();
        match self {
            Self::Undeclared { permitted } => {
                let n = permitted.len();
                let preview = permitted
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                let tail = if n > 5 { ", ..." } else { "" };
                format!(
                    "PRD {prd} has no \"{REQUIRED_TOOLS_HEADING}\" section; the project MCP config {mcp} permits {n} tool(s) ({preview}{tail}); tool links will be derived from PRD id text"
                )
            }
            Self::NotPermitted { token } => format!(
                "PRD {prd} names MCP tool '{token}', which the project MCP config {mcp} does not permit (absent, disabled, not in tool_permissions, or marked dangerous); permit it there or remove it from the PRD"
            ),
            Self::Unlinked { token } => format!(
                "PRD {prd} names MCP tool '{token}' but links it to no PRD id; in the Required Tools section put the ids whose work calls it on the same line as the tool"
            ),
        }
    }
}
pub fn prd_tool_defects(
    decl: &PrdToolDeclaration,
    permitted: &BTreeSet<String>,
) -> Vec<PrdToolDefect> {
    let mut defects = Vec::new();
    if !permitted.is_empty() && decl.section_line.is_none() {
        defects.push(PrdToolDefect::Undeclared {
            permitted: permitted.iter().cloned().collect(),
        });
    }
    for (token, links) in &decl.tokens {
        if !permitted.contains(token) {
            defects.push(PrdToolDefect::NotPermitted {
                token: token.clone(),
            });
        } else if links.is_empty() {
            defects.push(PrdToolDefect::Unlinked {
                token: token.clone(),
            });
        }
    }
    defects.sort_by_key(|defect| {
        (
            defect.code(),
            match defect {
                PrdToolDefect::Undeclared { .. } => String::new(),
                PrdToolDefect::NotPermitted { token } | PrdToolDefect::Unlinked { token } => {
                    token.clone()
                }
            },
        )
    });
    defects
}

#[cfg(test)]
#[path = "prd_tools_tests.rs"]
mod tests;
