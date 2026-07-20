use std::collections::BTreeSet;

use super::abbreviation::{abbreviate, apply_known_abbrev};
use super::extraction::Extracted;
use super::{CompressedMemory, estimate_tokens};


pub(super) fn build_compressed(
    raw: &str,
    ex: &Extracted,
    budget_tokens: usize,
    existing_context: Option<&str>,
) -> CompressedMemory {
    let input_tokens = estimate_tokens(raw);

    // Dedup filter: lowercase set of words from existing context.
    let dedup_set: BTreeSet<String> = existing_context
        .map(|ctx| ctx.split_whitespace().map(|w| w.to_lowercase()).collect())
        .unwrap_or_default();

    let should_keep = |name: &str| -> bool {
        if dedup_set.is_empty() {
            return true;
        }
        !dedup_set.contains(&name.to_lowercase())
    };

    // Build section strings.
    // ENT
    let ent_items: Vec<String> = ex
        .entities
        .iter()
        .filter(|e| should_keep(e))
        .map(|e| abbreviate(e))
        .collect();
    let ent_section = if ent_items.is_empty() {
        None
    } else {
        Some(format!("ENT:{}", ent_items.join("|")))
    };
    let entities_preserved = ent_items.len();

    // DEC
    let dec_items: Vec<String> = ex
        .decisions
        .iter()
        .filter(|(c, _)| should_keep(c))
        .map(|(choice, context)| {
            let c = apply_known_abbrev(choice);
            if context.is_empty() {
                c
            } else {
                let ctx = context
                    .split_whitespace()
                    .take(2)
                    .map(apply_known_abbrev)
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("{}>{}", c, ctx)
            }
        })
        .collect();
    let dec_section = if dec_items.is_empty() {
        None
    } else {
        Some(format!("DEC:{}", dec_items.join("|")))
    };

    // REL
    let rel_items: Vec<String> = ex
        .relationships
        .iter()
        .map(|(from, to)| format!("{}->{}", abbreviate(from), abbreviate(to)))
        .collect();
    let rel_section = if rel_items.is_empty() {
        None
    } else {
        Some(format!("REL:{}", rel_items.join("|")))
    };

    // PAT
    let pat_items: Vec<String> = ex
        .patterns
        .iter()
        .map(|(name, role)| {
            let n = apply_known_abbrev(name);
            if role.is_empty() {
                n
            } else {
                format!("{}>{}", n, apply_known_abbrev(role))
            }
        })
        .collect();
    let pat_section = if pat_items.is_empty() {
        None
    } else {
        Some(format!("PAT:{}", pat_items.join("|")))
    };

    // FIX
    let fix_items: Vec<String> = ex
        .corrections
        .iter()
        .map(|c| format!("!{}", c.replace(' ', "@")))
        .collect();
    let fix_section = if fix_items.is_empty() {
        None
    } else {
        Some(format!("FIX:{}", fix_items.join("|")))
    };

    // SH
    let sh_items: Vec<String> = ex
        .verdicts
        .iter()
        .map(|(label, verdict)| {
            let upper = verdict.to_uppercase();
            let v = match upper.as_str() {
                "INNOCENT" => "INNOC",
                "GUILTY" => "GUILT",
                "APPROVED" => "APRVD",
                "REJECTED" => "RJCTD",
                other => other,
            };
            format!("{}={}", label, v)
        })
        .collect();
    let sh_section = if sh_items.is_empty() {
        None
    } else {
        Some(format!("SH:{}", sh_items.join("|")))
    };

    // Phase tags
    let phase_tags: Vec<String> = ex
        .phase_entities
        .iter()
        .map(|(p, ents)| {
            let abbrevs: Vec<String> = ents.iter().map(|e| abbreviate(e)).collect();
            format!("@P{}:{}", p, abbrevs.join("+"))
        })
        .collect();
    let phase_section = if phase_tags.is_empty() {
        None
    } else {
        Some(phase_tags.join(" "))
    };

    // Assemble sections in priority order (last = lowest priority = removed first).
    // Order: ENT (highest), DEC, SH, REL, FIX, PAT (lowest), phases.
    let mut sections: Vec<Section> = Vec::new();

    if let Some(s) = ent_section {
        sections.push(Section {
            tag: "ENT".to_string(),
            text: s,
            priority: 10,
        });
    }
    if let Some(s) = dec_section {
        sections.push(Section {
            tag: "DEC".to_string(),
            text: s,
            priority: 8,
        });
    }
    if let Some(s) = sh_section {
        sections.push(Section {
            tag: "SH".to_string(),
            text: s,
            priority: 7,
        });
    }
    if let Some(s) = rel_section {
        sections.push(Section {
            tag: "REL".to_string(),
            text: s,
            priority: 6,
        });
    }
    if let Some(s) = fix_section {
        sections.push(Section {
            tag: "FIX".to_string(),
            text: s,
            priority: 4,
        });
    }
    if let Some(s) = pat_section {
        sections.push(Section {
            tag: "PAT".to_string(),
            text: s,
            priority: 2,
        });
    }
    if let Some(s) = phase_section {
        sections.push(Section {
            tag: "PHASE".to_string(),
            text: s,
            priority: 3,
        });
    }

    // Build output, truncating lowest-priority sections if over budget.
    let header = "[MEM|v1]";
    let header_tokens = estimate_tokens(header) + 1; // +1 for newline

    // Sort by priority ascending so we can pop lowest first.
    sections.sort_by_key(|s| s.priority);

    // Iteratively remove lowest-priority sections if over budget.
    loop {
        let total_text = build_output_text(header, &sections);
        let tokens = estimate_tokens(&total_text);
        if tokens <= budget_tokens || sections.len() <= 1 {
            break;
        }
        // Remove lowest priority.
        sections.remove(0);
    }

    // Final check: if still over budget with just header + 1 section, truncate section text.
    let mut output_text = build_output_text(header, &sections);
    let mut out_tokens = estimate_tokens(&output_text);
    if out_tokens > budget_tokens && budget_tokens > header_tokens {
        let max_chars = budget_tokens * 4;
        if output_text.len() > max_chars {
            output_text.truncate(max_chars);
            // Ensure we don't cut mid-line.
            if let Some(pos) = output_text.rfind('\n') {
                output_text.truncate(pos);
            }
            out_tokens = estimate_tokens(&output_text);
        }
    }

    let sections_present: Vec<String> = sections.iter().map(|s| s.tag.clone()).collect();
    let ratio = if out_tokens > 0 {
        input_tokens as f64 / out_tokens as f64
    } else {
        0.0
    };

    CompressedMemory {
        text: output_text,
        token_estimate: out_tokens,
        entities_preserved,
        compression_ratio: ratio,
        sections_present,
    }
}

fn build_output_text(header: &str, sections: &[Section]) -> String {
    let mut out = String::from(header);
    for s in sections {
        out.push('\n');
        out.push_str(&s.text);
    }
    out
}

// ---------------------------------------------------------------------------
// Section helper
// ---------------------------------------------------------------------------

struct Section {
    tag: String,
    text: String,
    priority: u8, // lower = removed first during truncation
}
