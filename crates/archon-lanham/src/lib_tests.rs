
use super::*;
use serde::Deserialize;

fn golden(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/{}", env!("CARGO_MANIFEST_DIR"), name))
        .unwrap_or_else(|_| panic!("{name} missing — run scripts/lanham-golden-gen.ts"))
}

#[derive(Deserialize)]
struct Foundation {
    text: String,
    tokens: Vec<String>,
    sentences: Vec<String>,
}

#[test]
fn foundation_matches_ts_reference() {
    let fixtures: Vec<Foundation> =
        serde_json::from_str(&golden("golden_foundation.json")).unwrap();
    assert!(!fixtures.is_empty());
    for fx in &fixtures {
        assert_eq!(tokenize(&fx.text), fx.tokens, "tokenize:\n  {}", fx.text);
        assert_eq!(
            split_sentences(&fx.text),
            fx.sentences,
            "split_sentences:\n  {}",
            fx.text
        );
    }
}

#[derive(Deserialize)]
struct Lexical {
    words: Vec<String>,
    #[serde(rename = "isVerb")]
    is_verb: Vec<bool>,
    #[serde(rename = "isNominalization")]
    is_nominalization: Vec<bool>,
    #[serde(rename = "isLatinate")]
    is_latinate: Vec<bool>,
    #[serde(rename = "roughStem")]
    rough_stem: Vec<String>,
    #[serde(rename = "contentWords")]
    content_words: Vec<String>,
}

#[test]
fn lexical_matches_ts_reference() {
    let lx: Lexical = serde_json::from_str(&golden("golden_lexical.json")).unwrap();
    for (i, w) in lx.words.iter().enumerate() {
        assert_eq!(is_verb(w), lx.is_verb[i], "is_verb({w})");
        assert_eq!(
            is_nominalization(w),
            lx.is_nominalization[i],
            "is_nominalization({w})"
        );
        assert_eq!(is_latinate(w), lx.is_latinate[i], "is_latinate({w})");
        assert_eq!(rough_stem(w), lx.rough_stem[i], "rough_stem({w})");
    }
    assert_eq!(
        get_content_words(&lx.words),
        lx.content_words,
        "get_content_words"
    );
}

#[derive(Deserialize)]
struct AxisFixture {
    text: String,
    #[serde(rename = "latinateGermanicRatio")]
    latinate_germanic_ratio: f64,
    #[serde(rename = "nounVerbRatio")]
    noun_verb_ratio: f64,
    #[serde(rename = "nominalizationDensity")]
    nominalization_density: f64,
    #[serde(rename = "prepositionalPhraseDensity")]
    prepositional_phrase_density: f64,
    #[serde(rename = "beVerbRatio")]
    be_verb_ratio: f64,
    #[serde(rename = "parataxisHypotaxisRatio")]
    parataxis_hypotaxis_ratio: f64,
    #[serde(rename = "coordinatingConjunctionDensity")]
    coordinating_conjunction_density: f64,
    #[serde(rename = "subordinatingConjunctionDensity")]
    subordinating_conjunction_density: f64,
    #[serde(rename = "periodicRunningRatio")]
    periodic_running_ratio: f64,
    #[serde(rename = "preMainVerbClauseCount")]
    pre_main_verb_clause_count: f64,
    #[serde(rename = "voiceScore")]
    voice_score: f64,
    #[serde(rename = "dynamicRange")]
    dynamic_range: f64,
    #[serde(rename = "registerMarkednessScore")]
    register_markedness_score: f64,
}

#[test]
fn axes_match_ts_reference() {
    let fixtures: Vec<AxisFixture> = serde_json::from_str(&golden("golden_axes.json")).unwrap();
    assert!(!fixtures.is_empty());
    let approx = |a: f64, b: f64, name: &str, t: &str| {
        assert!(
            (a - b).abs() < 1e-9,
            "{name}: got {a}, want {b} for:\n  {t}"
        );
    };
    for fx in &fixtures {
        approx(
            latinate_germanic_ratio(&fx.text),
            fx.latinate_germanic_ratio,
            "latinate_germanic_ratio",
            &fx.text,
        );
        let nv = analyze_noun_verb(&fx.text);
        approx(
            nv.noun_verb_ratio,
            fx.noun_verb_ratio,
            "noun_verb_ratio",
            &fx.text,
        );
        approx(
            nv.nominalization_density,
            fx.nominalization_density,
            "nominalization_density",
            &fx.text,
        );
        approx(
            nv.prepositional_phrase_density,
            fx.prepositional_phrase_density,
            "prepositional_phrase_density",
            &fx.text,
        );
        approx(
            nv.be_verb_ratio,
            fx.be_verb_ratio,
            "be_verb_ratio",
            &fx.text,
        );
        let pp = analyze_parataxis(&fx.text);
        approx(
            pp.parataxis_hypotaxis_ratio,
            fx.parataxis_hypotaxis_ratio,
            "parataxis_hypotaxis_ratio",
            &fx.text,
        );
        approx(
            pp.coordinating_conjunction_density,
            fx.coordinating_conjunction_density,
            "coordinating_conjunction_density",
            &fx.text,
        );
        approx(
            pp.subordinating_conjunction_density,
            fx.subordinating_conjunction_density,
            "subordinating_conjunction_density",
            &fx.text,
        );
        let per = analyze_periodic(&fx.text);
        approx(
            per.periodic_running_ratio,
            fx.periodic_running_ratio,
            "periodic_running_ratio",
            &fx.text,
        );
        approx(
            per.pre_main_verb_clause_count,
            fx.pre_main_verb_clause_count,
            "pre_main_verb_clause_count",
            &fx.text,
        );
        let vo = analyze_voice(&fx.text);
        approx(vo.voice_score, fx.voice_score, "voice_score", &fx.text);
        approx(
            vo.dynamic_range,
            fx.dynamic_range,
            "dynamic_range",
            &fx.text,
        );
        approx(
            analyze_register(&fx.text).register_markedness_score,
            fx.register_markedness_score,
            "register_markedness_score",
            &fx.text,
        );
    }
}

#[derive(Deserialize)]
struct TacitFx {
    #[serde(rename = "alliterationDensity")]
    alliteration_density: f64,
    #[serde(rename = "polyptotonDensity")]
    polyptoton_density: f64,
    #[serde(rename = "chiasmusCount")]
    chiasmus_count: i64,
    #[serde(rename = "antithesisCount")]
    antithesis_count: i64,
    #[serde(rename = "anaphoraCount")]
    anaphora_count: i64,
    #[serde(rename = "isocolonCount")]
    isocolon_count: i64,
    #[serde(rename = "climaxPatternCount")]
    climax_pattern_count: i64,
}
#[derive(Deserialize)]
struct LabelsFx {
    #[serde(rename = "nounVerb")]
    noun_verb: String,
    #[serde(rename = "parataxisHypotaxis")]
    parataxis_hypotaxis: String,
    #[serde(rename = "periodicRunning")]
    periodic_running: String,
    voice: String,
    #[serde(rename = "primaryRegister")]
    primary_register: String,
    #[serde(rename = "registerMixed")]
    register_mixed: bool,
    opacity: String,
}
#[derive(Deserialize)]
struct FullFx {
    text: String,
    #[serde(rename = "nounVerbRatio")]
    noun_verb_ratio: f64,
    #[serde(rename = "nominalizationDensity")]
    nominalization_density: f64,
    #[serde(rename = "prepositionalPhraseDensity")]
    prepositional_phrase_density: f64,
    #[serde(rename = "beVerbRatio")]
    be_verb_ratio: f64,
    #[serde(rename = "parataxisHypotaxisRatio")]
    parataxis_hypotaxis_ratio: f64,
    #[serde(rename = "coordinatingConjunctionDensity")]
    coordinating_conjunction_density: f64,
    #[serde(rename = "subordinatingConjunctionDensity")]
    subordinating_conjunction_density: f64,
    #[serde(rename = "periodicRunningRatio")]
    periodic_running_ratio: f64,
    #[serde(rename = "preMainVerbClauseCount")]
    pre_main_verb_clause_count: f64,
    #[serde(rename = "voiceScore")]
    voice_score: f64,
    #[serde(rename = "dynamicRange")]
    dynamic_range: f64,
    #[serde(rename = "latinateGermanicRatio")]
    latinate_germanic_ratio: f64,
    #[serde(rename = "registerMarkednessScore")]
    register_markedness_score: f64,
    #[serde(rename = "opacityScore")]
    opacity_score: f64,
    #[serde(rename = "selfConsciousnessScore")]
    self_consciousness_score: f64,
    #[serde(rename = "tacitPatterns")]
    tacit_patterns: TacitFx,
    labels: LabelsFx,
}

#[test]
fn full_analysis_matches_ts_reference() {
    let fixtures: Vec<FullFx> = serde_json::from_str(&golden("golden_full.json")).unwrap();
    assert!(!fixtures.is_empty());
    let approx = |a: f64, b: f64, name: &str, t: &str| {
        assert!(
            (a - b).abs() < 1e-9,
            "{name}: got {a}, want {b} for:\n  {t}"
        );
    };
    for fx in &fixtures {
        let m = full_analysis(&fx.text);
        approx(
            m.noun_verb_ratio,
            fx.noun_verb_ratio,
            "noun_verb_ratio",
            &fx.text,
        );
        approx(
            m.nominalization_density,
            fx.nominalization_density,
            "nominalization_density",
            &fx.text,
        );
        approx(
            m.prepositional_phrase_density,
            fx.prepositional_phrase_density,
            "prepositional_phrase_density",
            &fx.text,
        );
        approx(m.be_verb_ratio, fx.be_verb_ratio, "be_verb_ratio", &fx.text);
        approx(
            m.parataxis_hypotaxis_ratio,
            fx.parataxis_hypotaxis_ratio,
            "parataxis_hypotaxis_ratio",
            &fx.text,
        );
        approx(
            m.coordinating_conjunction_density,
            fx.coordinating_conjunction_density,
            "coord_density",
            &fx.text,
        );
        approx(
            m.subordinating_conjunction_density,
            fx.subordinating_conjunction_density,
            "subord_density",
            &fx.text,
        );
        approx(
            m.periodic_running_ratio,
            fx.periodic_running_ratio,
            "periodic_running_ratio",
            &fx.text,
        );
        approx(
            m.pre_main_verb_clause_count,
            fx.pre_main_verb_clause_count,
            "pre_main_verb_clause_count",
            &fx.text,
        );
        approx(m.voice_score, fx.voice_score, "voice_score", &fx.text);
        approx(m.dynamic_range, fx.dynamic_range, "dynamic_range", &fx.text);
        approx(
            m.latinate_germanic_ratio,
            fx.latinate_germanic_ratio,
            "latinate_germanic_ratio",
            &fx.text,
        );
        approx(
            m.register_markedness_score,
            fx.register_markedness_score,
            "register_markedness_score",
            &fx.text,
        );
        approx(m.opacity_score, fx.opacity_score, "opacity_score", &fx.text);
        approx(
            m.self_consciousness_score,
            fx.self_consciousness_score,
            "self_consciousness_score",
            &fx.text,
        );
        let tp = &m.tacit_patterns;
        let fp = &fx.tacit_patterns;
        approx(
            tp.alliteration_density,
            fp.alliteration_density,
            "alliteration_density",
            &fx.text,
        );
        approx(
            tp.polyptoton_density,
            fp.polyptoton_density,
            "polyptoton_density",
            &fx.text,
        );
        assert_eq!(
            tp.chiasmus_count, fp.chiasmus_count,
            "chiasmus_count: {}",
            fx.text
        );
        assert_eq!(
            tp.antithesis_count, fp.antithesis_count,
            "antithesis_count: {}",
            fx.text
        );
        assert_eq!(
            tp.anaphora_count, fp.anaphora_count,
            "anaphora_count: {}",
            fx.text
        );
        assert_eq!(
            tp.isocolon_count, fp.isocolon_count,
            "isocolon_count: {}",
            fx.text
        );
        assert_eq!(
            tp.climax_pattern_count, fp.climax_pattern_count,
            "climax_pattern_count: {}",
            fx.text
        );
        let (lm, lf) = (&m.labels, &fx.labels);
        assert_eq!(lm.noun_verb, lf.noun_verb, "label nounVerb: {}", fx.text);
        assert_eq!(
            lm.parataxis_hypotaxis, lf.parataxis_hypotaxis,
            "label parataxis: {}",
            fx.text
        );
        assert_eq!(
            lm.periodic_running, lf.periodic_running,
            "label periodic: {}",
            fx.text
        );
        assert_eq!(lm.voice, lf.voice, "label voice: {}", fx.text);
        assert_eq!(
            lm.primary_register, lf.primary_register,
            "label register: {}",
            fx.text
        );
        assert_eq!(
            lm.register_mixed, lf.register_mixed,
            "label registerMixed: {}",
            fx.text
        );
        assert_eq!(lm.opacity, lf.opacity, "label opacity: {}", fx.text);
    }
}

#[test]
fn clamp_basic() {
    assert_eq!(clamp(1.5, 0.0, 1.0), 1.0);
    assert_eq!(clamp(-0.2, 0.0, 1.0), 0.0);
}
