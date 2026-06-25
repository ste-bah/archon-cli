//! Render a trained style profile into an Archon output-style `.md`.
//!
//! Faithful Rust port of `scripts/profile-to-output-style.mjs` — a deterministic
//! data-transform (no analyzer, no LLM). Golden-tested byte-for-byte against the
//! `.mjs` so the Mac can go text → output-style entirely offline in Rust.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;

#[derive(Deserialize, Default)]
pub struct Profile {
    #[serde(default)]
    pub metadata: Metadata,
    #[serde(default)]
    pub characteristics: Characteristics,
}

#[derive(Deserialize, Default)]
pub struct Metadata {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "suggestedLanhamTarget", default)]
    pub suggested_lanham_target: SuggestedTarget,
}

#[derive(Deserialize, Default)]
pub struct SuggestedTarget {
    #[serde(rename = "derivedFrom", default)]
    pub derived_from: Option<String>,
    #[serde(rename = "registerTarget", default)]
    pub register_target: Option<String>,
    #[serde(rename = "tacitPersuasionLevel", default)]
    pub tacit_persuasion_level: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct Characteristics {
    #[serde(rename = "lanhamMetrics", default)]
    pub lanham_metrics: LanhamMetricsJson,
    #[serde(default)]
    pub sentences: Sentences,
    #[serde(default)]
    pub tone: Tone,
    #[serde(rename = "argumentPatterns", default)]
    pub argument_patterns: ArgumentPatterns,
    #[serde(rename = "commonTransitions", default)]
    pub common_transitions: Vec<String>,
}

#[derive(Deserialize, Default)]
pub struct LanhamMetricsJson {
    #[serde(default)]
    pub labels: LabelsJson,
    #[serde(default)]
    pub explanations: Explanations,
}

#[derive(Deserialize, Default)]
pub struct LabelsJson {
    #[serde(rename = "nounVerb", default)]
    pub noun_verb: Option<String>,
    #[serde(rename = "parataxisHypotaxis", default)]
    pub parataxis_hypotaxis: Option<String>,
    #[serde(rename = "periodicRunning", default)]
    pub periodic_running: Option<String>,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(rename = "primaryRegister", default)]
    pub primary_register: Option<String>,
    #[serde(default)]
    pub opacity: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct Explanations {
    #[serde(rename = "nounVerb", default)]
    pub noun_verb: Option<String>,
    #[serde(rename = "periodicRunning", default)]
    pub periodic_running: Option<String>,
    #[serde(rename = "parataxisHypotaxis", default)]
    pub parataxis_hypotaxis: Option<String>,
    #[serde(default)]
    pub voice: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct Sentences {
    #[serde(rename = "averageLength", default)]
    pub average_length: f64,
    #[serde(rename = "longSentenceRatio", default)]
    pub long_sentence_ratio: f64,
    #[serde(rename = "complexSentenceRatio", default)]
    pub complex_sentence_ratio: f64,
}

#[derive(Deserialize, Default)]
pub struct Tone {
    #[serde(rename = "formalityScore", default)]
    pub formality_score: f64,
    #[serde(rename = "objectivityScore", default)]
    pub objectivity_score: f64,
}

#[derive(Deserialize, Default)]
pub struct ArgumentPatterns {
    #[serde(rename = "claimStructure", default)]
    pub claim_structure: ClaimStructure,
}

#[derive(Deserialize, Default)]
pub struct ClaimStructure {
    #[serde(rename = "claimStrength", default)]
    pub claim_strength: Option<String>,
    #[serde(rename = "hedgingPatterns", default)]
    pub hedging_patterns: Vec<String>,
}

static HEDGE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^[a-z][a-z '-]{1,30}$").unwrap());
static TRANSITION_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^[a-z][a-z ]{2,20}$").unwrap());

/// `s || fallback` where empty string counts as falsy (matches JS `||`).
fn or_else<'a>(opt: &'a Option<String>, fallback: &'a str) -> &'a str {
    match opt {
        Some(s) if !s.is_empty() => s,
        _ => fallback,
    }
}

/// Render the profile into the output-style `.md` body (faithful port of the `.mjs`).
pub fn render_output_style(profile: &Profile, profile_key: &str) -> String {
    let ch = &profile.characteristics;
    let lm = &ch.lanham_metrics;
    let labels = &lm.labels;
    let expl = &lm.explanations;
    let slt = &profile.metadata.suggested_lanham_target;

    let name = or_else(&profile.metadata.name, profile_key).to_string();
    let default_desc = format!("Trained style profile {name}");
    let desc = or_else(&profile.metadata.description, &default_desc).to_string();

    let genre = or_else(&slt.derived_from, "academic").to_string();
    let register = match (or_else(&slt.register_target, ""), or_else(&labels.primary_register, "")) {
        (r, _) if !r.is_empty() => r.to_string(),
        (_, p) if !p.is_empty() => p.to_string(),
        _ => "high".to_string(),
    };
    let voice_label = or_else(&labels.voice, "moderate voice").to_string();
    let effaced = voice_label.to_lowercase().contains("unvoiced") || voice_label.to_lowercase().contains("effaced");
    let parataxis = or_else(&labels.parataxis_hypotaxis, "mixed").to_string();
    let architecture = or_else(&labels.periodic_running, "mixed").to_string();
    let opacity = or_else(&labels.opacity, "mixed opacity").to_string();

    let at_through = if genre == "academic" { "transparent with AT moments" } else { "oscillating" };

    let mut tacit = or_else(&slt.tacit_persuasion_level, "moderate").to_string();
    if effaced && (tacit == "dense" || tacit == "moderate") {
        tacit = "some".to_string();
    }

    let s = &ch.sentences;
    let avg_len = s.average_length.round() as i64;
    let long_ratio = s.long_sentence_ratio;
    let complex_ratio = s.complex_sentence_ratio;
    let formal = ch.tone.formality_score > 0.55;
    let objective = ch.tone.objectivity_score > 0.55;

    let claim = &ch.argument_patterns.claim_structure;
    let hedges: Vec<String> = claim
        .hedging_patterns
        .iter()
        .filter(|h| HEDGE_RE.is_match(h))
        .take(4)
        .cloned()
        .collect();
    let claim_strength = claim.claim_strength.clone().unwrap_or_default();
    let transitions: Vec<String> = ch
        .common_transitions
        .iter()
        .filter(|t| TRANSITION_RE.is_match(t))
        .take(8)
        .cloned()
        .collect();

    let mut l: Vec<String> = Vec::new();
    l.push(format!("# {name}"));
    l.push(format!("Description: {desc} — {register} register, {parataxis}, {voice_label}."));
    l.push(String::new());
    l.push("When composing prose, write in the following trained scholarly voice. Treat these as binding stylistic constraints.".to_string());

    l.push("\n## REGISTER & DICTION".to_string());
    l.push(format!(
        "- Write in a {register}{} register. Prefer precise{} diction at conceptual turns; avoid colloquialism and contractions.",
        if genre == "academic" { ", formal academic" } else { "" },
        if register == "high" { ", frequently Latinate" } else { "" },
    ));
    if formal || objective {
        l.push(format!(
            "- Maintain a {}{}{} stance with minimal emotional coloring; keep the authorial \"I\" rare.",
            if formal { "formal" } else { "" },
            if formal && objective { ", " } else { "" },
            if objective { "objective" } else { "" },
        ));
    }

    l.push("\n## SENTENCES & ARCHITECTURE".to_string());
    if avg_len != 0 {
        l.push(format!(
            "- Favor {}{}sentences (typical length ~{avg_len} words{}), but vary length deliberately for rhythm.",
            if long_ratio > 0.45 { "long, " } else { "" },
            if complex_ratio > 0.45 { "syntactically complex " } else { "" },
            if long_ratio > 0.45 { "; over half should be long and complex" } else { "" },
        ));
    }
    l.push(format!(
        "- Architecture is {architecture}: {}",
        if architecture.contains("periodic") || architecture == "mixed" {
            "use periodic suspension (delaying the main clause) at argumentative turns, running delivery elsewhere."
        } else {
            "lead with the main clause; keep delivery running."
        },
    ));

    l.push("\n## CONNECTION".to_string());
    if parataxis.contains("paratactic") {
        l.push("- Connection is predominantly PARATACTIC: link independent clauses in coordinate chains (and / but / or), while admitting subordination only where the thought genuinely requires it.".to_string());
    } else if parataxis.contains("hypotactic") {
        l.push("- Connection is predominantly HYPOTACTIC: build nested subordinate structures; subordinate clauses carry the argumentative weight.".to_string());
    } else {
        l.push("- Balance coordinate (paratactic) chains with subordination as the thought requires.".to_string());
    }

    l.push("\n## VOICE".to_string());
    if effaced {
        l.push("- Effaced authorial presence (\"unvoiced\"): foreground the content and the argument over any sense of personality. Keep rhythmic self-display minimal.".to_string());
        l.push("- Do NOT add rhetorical flourish for its own sake; the prose should not read as performance.".to_string());
    } else {
        l.push(format!("- {voice_label}: let the prose reward reading aloud — vary rhythm and sentence length for vocal presence."));
    }

    l.push(format!("\n## AT/THROUGH MODE — {at_through}"));
    l.push("- Keep the prose mostly transparent: the reader should look THROUGH the language to the meaning.".to_string());
    if effaced {
        l.push("- Keep the prose mostly transparent; the reader should look THROUGH the language. Brief, restrained emphasis (a balanced or parallel construction) is permissible at major argumentative turns, but kept rare — the language should not become conspicuous or the object of attention.".to_string());
    } else {
        l.push("- At key argumentative turns (theses, conceptual pivots, conclusions) you may make the reader look AT the language via brief figures, register elevation, or periodic syntax — kept brief and occasional.".to_string());
    }

    l.push(format!("\n## {} BUDGET — {tacit}", if effaced { "EMPHASIS" } else { "TACIT PERSUASION" }));
    if effaced {
        l.push("- Use rhetorical figures sparingly and only where they serve the argument; let emphasis come mainly from sentence architecture and diction. Avoid sustained or decorative patterning.".to_string());
    } else {
        let budget_count = if tacit == "dense" {
            "throughout"
        } else if tacit == "moderate" {
            "2–3 per section"
        } else {
            "occasionally, at key turns"
        };
        l.push(format!("- Deploy notable patterns {budget_count}: parallelism, chiasmus, anaphora, polyptoton."));
    }

    if !claim_strength.is_empty() || !hedges.is_empty() {
        l.push("\n## CLAIMS".to_string());
        let strength = if claim_strength == "cautious" {
            "cautiously"
        } else if claim_strength == "strong" {
            "with confidence"
        } else {
            "with measured confidence"
        };
        let hedge_clause = if !hedges.is_empty() {
            let quoted: Vec<String> = hedges.iter().map(|h| format!("\"{h}\"")).collect();
            format!(", hedging where appropriate ({})", quoted.join(", "))
        } else {
            String::new()
        };
        l.push(format!("- Advance claims {strength}{hedge_clause}; reserve strong markers for genuinely settled points."));
    }

    if !transitions.is_empty() {
        l.push("\n## TRANSITIONS".to_string());
        l.push(format!("- Prefer these connectives for paragraph and argumentative transitions: {}.", transitions.join(", ")));
    }

    l.push("\n## PROSE STYLE DIMENSIONS (the measured profile to match)".to_string());
    let mut dim = |label: &str, val: &str, ex: &str| {
        if !val.is_empty() {
            if ex.is_empty() {
                l.push(format!("- {label}: {val}"));
            } else {
                l.push(format!("- {label}: {val} — {ex}"));
            }
        }
    };
    dim("Noun/Verb", or_else(&labels.noun_verb, ""), or_else(&expl.noun_verb, ""));
    dim("Architecture", &architecture, or_else(&expl.periodic_running, ""));
    dim("Connection", &parataxis, or_else(&expl.parataxis_hypotaxis, ""));
    dim(
        "Voice",
        &voice_label,
        if effaced { "effaced authorial presence; content over personality" } else { or_else(&expl.voice, "") },
    );
    dim("Register", if register == "high" { "high (academic)" } else { &register }, "");
    dim(
        "Opacity",
        if opacity.contains("opaque") { "foreground language only at conceptual pivots" } else { &opacity },
        "",
    );

    l.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Byte-for-byte parity with scripts/profile-to-output-style.mjs.
    /// tests/profile_dalton.json + tests/ref_dalton.md are produced by the Bash harness.
    #[test]
    fn render_matches_mjs_reference() {
        let dir = env!("CARGO_MANIFEST_DIR");
        let profile_json = std::fs::read_to_string(format!("{dir}/tests/profile_dalton.json"))
            .expect("profile_dalton.json missing — run the render-golden harness");
        let reference = std::fs::read_to_string(format!("{dir}/tests/ref_dalton.md"))
            .expect("ref_dalton.md missing — run the render-golden harness");
        let profile: Profile = serde_json::from_str(&profile_json).unwrap();
        let rendered = render_output_style(&profile, "dalton-philosophical-mo2fmhy2");
        assert_eq!(rendered, reference, "Rust renderer diverged from the .mjs converter");
    }
}
