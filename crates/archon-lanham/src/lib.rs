//! Lanham prose analyzer — Rust port (POS-free axes, L2).
//!
//! Faithful port of the god-agent TypeScript analyzer (`lanham-shared.ts`).
//! Every public function is golden-tested against the TS reference (see `tests/`).
//! The POS tagger is behind the `tag_pos` seam (returns empty in L2; en-pos
//! reimplementation is L3).

pub mod render;

#[path = "lexicon.rs"]
mod lexicon;
pub use lexicon::*;
#[path = "axis_syntax.rs"]
mod axis_syntax;
pub use axis_syntax::*;
#[path = "axis_style.rs"]
mod axis_style;
pub use axis_style::*;
#[path = "axis_opacity.rs"]
mod axis_opacity;
use axis_opacity::*;
#[path = "axis_tacit.rs"]
mod axis_tacit;
pub use axis_tacit::*;

// ── Labels + full analysis ───────────────────────────────────────────────────
#[derive(Clone, Default)]
pub struct Labels {
    pub noun_verb: String,
    pub parataxis_hypotaxis: String,
    pub periodic_running: String,
    pub voice: String,
    pub primary_register: String,
    pub register_mixed: bool,
    pub opacity: String,
}

#[derive(Clone)]
pub struct LanhamMetrics {
    pub noun_verb_ratio: f64,
    pub nominalization_density: f64,
    pub prepositional_phrase_density: f64,
    pub be_verb_ratio: f64,
    pub parataxis_hypotaxis_ratio: f64,
    pub coordinating_conjunction_density: f64,
    pub subordinating_conjunction_density: f64,
    pub periodic_running_ratio: f64,
    pub pre_main_verb_clause_count: f64,
    pub voice_score: f64,
    pub dynamic_range: f64,
    pub latinate_germanic_ratio: f64,
    pub register_markedness_score: f64,
    pub opacity_score: f64,
    pub self_consciousness_score: f64,
    pub tacit_patterns: TacitPatterns,
    pub labels: Labels,
}

// Academic genre thresholds (GENRE_THRESHOLDS["academic"]).
fn derive_labels(m: &LanhamMetrics) -> Labels {
    let band = |v: f64, lo: f64, hi: f64, a: &str, b: &str, c: &str| {
        (if v < lo {
            a
        } else if v > hi {
            b
        } else {
            c
        })
        .to_string()
    };
    let primary_register = if m.register_markedness_score >= 0.62 {
        "high"
    } else if m.register_markedness_score <= 0.38 {
        "low"
    } else {
        "middle"
    };
    let mut noun_verb = band(
        m.noun_verb_ratio,
        0.35,
        0.65,
        "predominantly noun-style",
        "predominantly verb-style",
        "balanced",
    );
    if noun_verb == "balanced"
        && m.nominalization_density > 8.0
        && m.be_verb_ratio > 0.25
        && m.prepositional_phrase_density > 3.0
    {
        noun_verb = "predominantly noun-style".to_string();
    }
    Labels {
        noun_verb,
        parataxis_hypotaxis: band(
            m.parataxis_hypotaxis_ratio,
            0.35,
            0.65,
            "predominantly paratactic",
            "predominantly hypotactic",
            "mixed",
        ),
        periodic_running: band(
            m.periodic_running_ratio,
            0.35,
            0.65,
            "predominantly periodic",
            "predominantly running",
            "mixed",
        ),
        voice: band(
            m.voice_score,
            0.30,
            0.70,
            "unvoiced",
            "strongly voiced",
            "moderate voice",
        ),
        primary_register: primary_register.to_string(),
        register_mixed: false,
        opacity: band(
            m.opacity_score,
            0.25,
            0.60,
            "transparent",
            "opaque",
            "mixed opacity",
        ),
    }
}

/// Full analysis: all axes + the opacity-tacit blend + academic labels.
pub fn full_analysis(text: &str) -> LanhamMetrics {
    let nv = analyze_noun_verb(text);
    let pp = analyze_parataxis(text);
    let per = analyze_periodic(text);
    let vo = analyze_voice(text);
    let rg = analyze_register(text);
    let op = analyze_opacity(text);
    let tacit = detect_tacit(text);

    // Blend tacit pattern density into opacity (fullAnalysis).
    let sent_count = SENT_TERM_RE.find_iter(text).count().max(1) as f64;
    let tacit_total = (tacit.anaphora_count
        + tacit.chiasmus_count
        + tacit.antithesis_count
        + tacit.isocolon_count
        + tacit.climax_pattern_count) as f64
        / sent_count;
    let tacit_density = clamp(tacit_total / 0.2, 0.0, 1.0);
    let allit = clamp(tacit.alliteration_density / 0.15, 0.0, 1.0);
    let polyp = clamp(tacit.polyptoton_density / 0.1, 0.0, 1.0);
    let opacity_score = clamp(
        op.opacity_score * 0.50 + tacit_density * 0.25 + allit * 0.15 + polyp * 0.10,
        0.0,
        1.0,
    );

    let mut m = LanhamMetrics {
        noun_verb_ratio: nv.noun_verb_ratio,
        nominalization_density: nv.nominalization_density,
        prepositional_phrase_density: nv.prepositional_phrase_density,
        be_verb_ratio: nv.be_verb_ratio,
        parataxis_hypotaxis_ratio: pp.parataxis_hypotaxis_ratio,
        coordinating_conjunction_density: pp.coordinating_conjunction_density,
        subordinating_conjunction_density: pp.subordinating_conjunction_density,
        periodic_running_ratio: per.periodic_running_ratio,
        pre_main_verb_clause_count: per.pre_main_verb_clause_count,
        voice_score: vo.voice_score,
        dynamic_range: vo.dynamic_range,
        latinate_germanic_ratio: rg.latinate_germanic_ratio,
        register_markedness_score: rg.register_markedness_score,
        opacity_score,
        self_consciousness_score: op.self_consciousness_score,
        tacit_patterns: tacit,
        labels: Labels::default(),
    };
    m.labels = derive_labels(&m);
    m
}

/// Result of training: the rendered output-style `.md` plus a few labels for a summary line.
pub struct TrainResult {
    pub md: String,
    pub voice: String,
    pub register: String,
    pub parataxis: String,
}

/// Train an output-style from raw sample prose: measure Lanham style + base sentence/tone
/// stats, assemble a profile, render to the Archon output-style `.md`. All-Rust, offline.
pub fn train_to_output_style(text: &str, name: &str, genre: &str) -> TrainResult {
    use crate::render::{
        ArgumentPatterns, Characteristics, ClaimStructure, Explanations, LabelsJson,
        LanhamMetricsJson, Metadata, Profile, Sentences, SuggestedTarget, Tone,
    };

    let m = full_analysis(text);

    // Base sentence stats.
    let sents = split_sentences(text);
    let lens: Vec<usize> = sents.iter().map(|s| s.split_whitespace().count()).collect();
    let n = sents.len().max(1) as f64;
    let avg_len = if lens.is_empty() {
        0.0
    } else {
        lens.iter().sum::<usize>() as f64 / lens.len() as f64
    };
    let long_ratio = lens.iter().filter(|&&l| l > 25).count() as f64 / n;
    let complex_ratio = sents
        .iter()
        .filter(|s| s.contains(';') || s.contains(':') || s.matches(',').count() >= 3)
        .count() as f64
        / n;

    // Base tone stats (formality proxy: Latinate ratio stands in for the academic word-list).
    let words = tokenize(text);
    let wn = words.len().max(1) as f64;
    let first_person = ["i", "we", "my", "our", "me", "us"];
    let fp = words
        .iter()
        .filter(|w| first_person.contains(&w.as_str()))
        .count() as f64;
    let objectivity = (1.0 - fp / wn * 20.0).clamp(0.0, 1.0);
    let contractions = words.iter().filter(|w| w.contains('\'')).count() as f64;
    let formality =
        (0.3 + (1.0 - contractions / wn * 10.0) * 0.3 + latinate_germanic_ratio(text) * 0.4)
            .clamp(0.0, 1.0);

    let profile = Profile {
        metadata: Metadata {
            name: Some(name.to_string()),
            description: Some(format!("Trained style profile {name}")),
            suggested_lanham_target: SuggestedTarget {
                derived_from: Some(genre.to_string()),
                register_target: None, // converter falls back to the MEASURED primaryRegister
                tacit_persuasion_level: Some("moderate".into()),
            },
        },
        characteristics: Characteristics {
            lanham_metrics: LanhamMetricsJson {
                labels: LabelsJson {
                    noun_verb: Some(m.labels.noun_verb.clone()),
                    parataxis_hypotaxis: Some(m.labels.parataxis_hypotaxis.clone()),
                    periodic_running: Some(m.labels.periodic_running.clone()),
                    voice: Some(m.labels.voice.clone()),
                    primary_register: Some(m.labels.primary_register.clone()),
                    opacity: Some(m.labels.opacity.clone()),
                },
                explanations: Explanations::default(),
            },
            sentences: Sentences {
                average_length: avg_len,
                long_sentence_ratio: long_ratio,
                complex_sentence_ratio: complex_ratio,
            },
            tone: Tone {
                formality_score: formality,
                objectivity_score: objectivity,
            },
            argument_patterns: ArgumentPatterns {
                claim_structure: ClaimStructure::default(),
            },
            common_transitions: Vec::new(),
        },
    };

    TrainResult {
        md: crate::render::render_output_style(&profile, name),
        voice: m.labels.voice,
        register: m.labels.primary_register,
        parataxis: m.labels.parataxis_hypotaxis,
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
