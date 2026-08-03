//! Extracting normative requirement IDs out of a PRD, by regex.
//!
//! No model is involved and none is wanted. The IDs are regular — every one of
//! the 93 in `PRD-TRADING-DATA-LAKE-AHDM-001.md` is a column-zero bullet of the
//! form `- REQ-<PREFIX>-<NNN>: <text>`, with wrapped continuation lines indented
//! two spaces. A regex over that grammar is exact, reproducible and free; an LLM
//! over it would be none of the three, and F1 is direct evidence of what happens
//! when a model is asked to decide which requirements exist.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::schema::EntityRecord;
use crate::{now_iso, stable_id};

/// `entity_type` for a requirement node in `kb_entities`.
pub const REQUIREMENT_ENTITY_TYPE: &str = "prd_requirement";

/// How severe a violation of this requirement is, for falsification scoping.
///
/// # The PRD does not declare this
///
/// PRD §21 attaches `severity` to a *validation check* inside `validation.json`,
/// not to a requirement. There is no per-requirement severity marker anywhere in
/// the document — no MoSCoW tag, no priority column, nothing between the colon
/// and the prose. So this is derived, and derived narrowly: a requirement is
/// [`Severity::Error`] only when its own text names §21's `error` severity class
/// or §32's fail-closed vocabulary, and the matched phrase is recorded on the
/// requirement so a reviewer can check the derivation rather than trust it.
///
/// Everything else is [`Severity::Unclassified`] and is out of falsification
/// scope. That is fail-closed in the direction that costs nothing: an
/// unclassified requirement still has to reach `Exercised` on evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Violation fails production eligibility (PRD §21) or breaches a
    /// fail-closed rule (PRD §32).
    Error,
    /// No severity phrase matched. Not "low" — *unknown*.
    Unclassified,
}

/// The phrases that classify a requirement as [`Severity::Error`].
///
/// Sourced from PRD §21's severity table and §32's residual-gap policy, and
/// deliberately short: every entry is a literal the PRD itself uses, so the
/// classification is auditable by grep. Matching is case-insensitive on an
/// otherwise verbatim substring — no stemming, no synonyms, no scoring.
const ERROR_SEVERITY_PHRASES: &[&str] =
    &["`error`", "status=failed", "fail closed", "fails closed"];

/// One normative requirement lifted from the PRD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirement {
    /// e.g. `REQ-DL-034`.
    pub id: String,
    /// e.g. `DL`. Present so a report can group without re-parsing the id.
    pub prefix: String,
    /// The requirement sentence, with wrapped continuation lines joined.
    pub text: String,
    /// 1-based line of the bullet in the PRD. This is the requirement's own
    /// anchor: a requirement is as citable as the code it points at.
    pub line: usize,
    pub severity: Severity,
    /// The literal phrase that produced [`Requirement::severity`], or `None`
    /// when unclassified. Recorded so the derivation can be disputed.
    pub severity_evidence: Option<String>,
}

impl Requirement {
    /// Whether this requirement is in falsification scope (PRD §21 `error`).
    pub fn is_error_severity(&self) -> bool {
        self.severity == Severity::Error
    }
}

fn bullet_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^- (?P<id>REQ-(?P<prefix>[A-Z]+)-[0-9]{3}): (?P<text>.*)$")
            .expect("requirement bullet regex is a literal")
    })
}

/// Pull every requirement out of PRD markdown.
///
/// Fenced code blocks are skipped. The PRD's only in-fence identifier is the
/// `GAP-DL-001` example in §32, which this grammar would not match anyway, but
/// skipping fences means a future PRD that shows a requirement bullet as an
/// *example* does not get that example counted as a requirement.
///
/// Continuation lines are two-space-indented and carry no bullet marker; they
/// are joined into [`Requirement::text`] with a single space so that a severity
/// phrase split across a line wrap is still found.
pub fn extract_requirements(prd: &str) -> Vec<Requirement> {
    let mut out: Vec<Requirement> = Vec::new();
    let mut in_fence = false;

    for (idx, raw_line) in prd.lines().enumerate() {
        if raw_line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }

        if let Some(caps) = bullet_re().captures(raw_line) {
            out.push(Requirement {
                id: caps["id"].to_string(),
                prefix: caps["prefix"].to_string(),
                text: caps["text"].trim().to_string(),
                line: idx + 1,
                severity: Severity::Unclassified,
                severity_evidence: None,
            });
            continue;
        }

        // A two-space-indented, unbulleted line directly under a requirement is
        // that requirement's wrapped tail.
        if let Some(last) = out.last_mut()
            && is_continuation(raw_line, last.line, idx + 1, prd)
        {
            last.text.push(' ');
            last.text.push_str(raw_line.trim());
        }
    }

    for requirement in &mut out {
        let (severity, evidence) = classify(&requirement.text);
        requirement.severity = severity;
        requirement.severity_evidence = evidence;
    }
    out
}

/// A continuation is indented exactly by the bullet's own hanging indent (two
/// spaces), is not itself a bullet, and is not separated from its requirement by
/// a blank line.
fn is_continuation(line: &str, req_line: usize, this_line: usize, prd: &str) -> bool {
    if !line.starts_with("  ") || line.trim().is_empty() {
        return false;
    }
    let trimmed = line.trim_start();
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with('#') {
        return false;
    }
    // Every line between the bullet and this one must also be non-blank, else
    // the paragraph ended and this indented text belongs to something else.
    prd.lines()
        .skip(req_line)
        .take(this_line.saturating_sub(req_line + 1))
        .all(|between| !between.trim().is_empty())
}

fn classify(text: &str) -> (Severity, Option<String>) {
    let haystack = text.to_ascii_lowercase();
    for phrase in ERROR_SEVERITY_PHRASES {
        if haystack.contains(*phrase) {
            return (Severity::Error, Some((*phrase).to_string()));
        }
    }
    (Severity::Unclassified, None)
}

/// Project a requirement into the knowledge graph as an entity.
///
/// `source_chunk_id` is `"<prd_path>#L<line>"` — the requirement's citation back
/// into the PRD, in the same shape as the `file:line` an anchor uses in the
/// other direction. A requirement that cannot be cited has no business being a
/// node.
pub fn requirement_entity(requirement: &Requirement, prd_path: &str) -> EntityRecord {
    requirement_entity_for(&requirement.id, requirement.line, prd_path)
}

/// The same projection from the parts a report row keeps, so persistence does
/// not have to reconstruct a [`Requirement`] it no longer holds.
pub fn requirement_entity_for(id: &str, line: usize, prd_path: &str) -> EntityRecord {
    EntityRecord {
        entity_id: stable_id("req", &[prd_path, id]),
        name: id.to_string(),
        entity_type: REQUIREMENT_ENTITY_TYPE.to_string(),
        source_chunk_id: format!("{prd_path}#L{line}"),
        mentions: 1,
        // 1.0 because extraction is a regex over a literal grammar, not an
        // estimate. This field means "how sure are we the node exists", and a
        // matched line is certain. It says nothing about satisfaction.
        confidence: 1.0,
        created_at: now_iso(),
    }
}

#[cfg(test)]
mod tests;
