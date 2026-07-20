use regex::Regex;
use std::collections::BTreeSet;


#[derive(Debug, Default)]
pub(super) struct Extracted {
    pub(super) entities: BTreeSet<String>,
    pub(super) decisions: Vec<(String, String)>,     // (choice, context)
    pub(super) relationships: Vec<(String, String)>, // (from, to)
    pub(super) patterns: Vec<(String, String)>,      // (name, role)
    pub(super) corrections: Vec<String>,
    pub(super) verdicts: Vec<(String, String)>,         // (phase/label, verdict)
    pub(super) phase_entities: Vec<(u32, Vec<String>)>, // (phase, entities)
}

pub(super) fn extract(raw: &str) -> Extracted {
    let mut ex = Extracted::default();

    extract_entities(raw, &mut ex);
    extract_decisions(raw, &mut ex);
    extract_relationships(raw, &mut ex);
    extract_patterns(raw, &mut ex);
    extract_corrections(raw, &mut ex);
    extract_verdicts(raw, &mut ex);
    extract_phase_entities(raw, &mut ex);

    ex
}

/// Extract CamelCase names and words ending in known suffixes.
fn extract_entities(raw: &str, ex: &mut Extracted) {
    // CamelCase: at least two uppercase-starting components.
    let camel_re = Regex::new(r"\b([A-Z][a-z]+(?:[A-Z][a-z0-9]+)+)\b").unwrap();
    for cap in camel_re.captures_iter(raw) {
        ex.entities.insert(cap[1].to_string());
    }

    // Words ending in known suffixes (at least prefix + suffix).
    let suffix_re = Regex::new(
        r"\b([A-Z][a-zA-Z]*(?:Service|Handler|Manager|Controller|Store|Repository|Validator|Middleware))\b",
    )
    .unwrap();
    for cap in suffix_re.captures_iter(raw) {
        ex.entities.insert(cap[1].to_string());
    }
}

/// Extract decision phrases: "decided to ...", "chose ...", "will use ...", "selected ...".
fn extract_decisions(raw: &str, ex: &mut Extracted) {
    let dec_re = Regex::new(
        r"(?i)(?:decided to|chose|will use|selected)\s+([a-zA-Z0-9_]+)(?:\s+(?:for|as|to|over)\s+([a-zA-Z0-9_ ]+))?"
    ).unwrap();
    for cap in dec_re.captures_iter(raw) {
        let choice = cap[1].to_string();
        let context = cap.get(2).map_or(String::new(), |m| {
            // Take first 3 words of context.
            m.as_str()
                .split_whitespace()
                .take(3)
                .collect::<Vec<_>>()
                .join(" ")
        });
        ex.decisions.push((choice, context));
    }
}

/// Extract relationship phrases: "depends on", "calls", "uses", "requires", "->", "implements".
fn extract_relationships(raw: &str, ex: &mut Extracted) {
    // Arrow syntax: A -> B
    let arrow_re = Regex::new(r"\b([A-Z][a-zA-Z0-9]+)\s*->\s*([A-Z][a-zA-Z0-9]+)\b").unwrap();
    for cap in arrow_re.captures_iter(raw) {
        ex.relationships
            .push((cap[1].to_string(), cap[2].to_string()));
    }

    // Natural language: X depends on / calls / uses / requires / implements Y
    let rel_re = Regex::new(
        r"\b([A-Z][a-zA-Z0-9]+)\s+(?:depends on|calls|uses|requires|implements)\s+([A-Z][a-zA-Z0-9]+)\b",
    )
    .unwrap();
    for cap in rel_re.captures_iter(raw) {
        ex.relationships
            .push((cap[1].to_string(), cap[2].to_string()));
    }
}

/// Extract pattern mentions.
fn extract_patterns(raw: &str, ex: &mut Extracted) {
    let pat_re = Regex::new(
        r"(?i)([a-zA-Z]+)\s+(?:pattern|strategy|approach)\s+(?:for\s+)?([a-zA-Z ]{2,30})",
    )
    .unwrap();
    for cap in pat_re.captures_iter(raw) {
        let name = cap[1].to_string();
        let role = cap.get(2).map_or(String::new(), |m| {
            m.as_str()
                .split_whitespace()
                .take(2)
                .collect::<Vec<_>>()
                .join(" ")
        });
        ex.patterns.push((name, role));
    }
}

/// Extract correction phrases.
fn extract_corrections(raw: &str, ex: &mut Extracted) {
    let fix_re = Regex::new(
        r"(?i)(?:fixed|don't|avoid|instead of)\s+([a-zA-Z0-9_.!]+(?:\s+[a-zA-Z0-9_.]+){0,3})",
    )
    .unwrap();
    for cap in fix_re.captures_iter(raw) {
        ex.corrections.push(cap[1].to_string());
    }
}

/// Extract Sherlock verdicts.
fn extract_verdicts(raw: &str, ex: &mut Extracted) {
    // Look for "INNOCENT", "GUILTY", "APPROVED", "REJECTED" possibly near phase/sherlock info.
    let verdict_re = Regex::new(
        r"(?i)(?:(?:phase|P)\s*(\d).*?(INNOCENT|GUILTY|APPROVED|REJECTED)|(INNOCENT|GUILTY|APPROVED|REJECTED).*?(?:phase|P)\s*(\d)|[Ss]herlock.*?(INNOCENT|GUILTY|APPROVED|REJECTED))"
    ).unwrap();
    for cap in verdict_re.captures_iter(raw) {
        // Try the various groups.
        if let Some(phase) = cap.get(1) {
            let verdict = cap[2].to_string();
            ex.verdicts.push((format!("P{}", phase.as_str()), verdict));
        } else if let Some(phase) = cap.get(4) {
            let verdict = cap[3].to_string();
            ex.verdicts.push((format!("P{}", phase.as_str()), verdict));
        } else if let Some(verdict) = cap.get(5) {
            ex.verdicts
                .push(("SH".to_string(), verdict.as_str().to_string()));
        }
    }
}

/// Extract phase-tagged entities (e.g. "Phase 1: UserService, AuthMiddleware").
fn extract_phase_entities(raw: &str, ex: &mut Extracted) {
    let phase_re = Regex::new(r"(?i)phase\s+(\d)\s*[:\-]\s*([^\n]+)").unwrap();
    let entity_re = Regex::new(r"\b([A-Z][a-zA-Z0-9]+(?:[A-Z][a-z0-9]+)*)\b").unwrap();
    for cap in phase_re.captures_iter(raw) {
        if let Ok(phase_num) = cap[1].parse::<u32>() {
            let content = &cap[2];
            let mut ents = Vec::new();
            for e in entity_re.captures_iter(content) {
                ents.push(e[1].to_string());
            }
            if !ents.is_empty() {
                ex.phase_entities.push((phase_num, ents));
            }
        }
    }
}
