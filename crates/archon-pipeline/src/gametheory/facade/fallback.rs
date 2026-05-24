use super::super::fingerprint::{
    AmbiguityNote, AxisVerdict, GameTheoryFingerprint, HiddenGameDetection,
};

#[derive(Clone, Copy)]
struct AxisRule {
    keywords: &'static [&'static str],
    verdict: &'static str,
    confidence: &'static str,
    rationale: &'static str,
}

#[derive(Clone, Copy)]
struct FamilyRule {
    all: &'static [&'static str],
    any: &'static [&'static str],
    primary_family: &'static str,
    nearest_classic: Option<&'static str>,
}

const COOPERATION_RULES: &[AxisRule] = &[AxisRule {
    keywords: &["collaborate", "cooperate", "alliance", "cartel"],
    verdict: "cooperative",
    confidence: "medium",
    rationale: "cooperation keywords detected",
}];

const PAYOFF_SUM_RULES: &[AxisRule] = &[
    AxisRule {
        keywords: &["zero-sum", "winner-take", "all or nothing"],
        verdict: "zero-sum",
        confidence: "medium",
        rationale: "zero-sum keywords detected",
    },
    AxisRule {
        keywords: &["win-win", "mutual gain", "positive-sum"],
        verdict: "positive-sum",
        confidence: "medium",
        rationale: "positive-sum keywords detected",
    },
];

const SYMMETRY_RULES: &[AxisRule] = &[
    AxisRule {
        keywords: &["symmetric", "identical", "same"],
        verdict: "symmetric",
        confidence: "medium",
        rationale: "symmetry keywords detected",
    },
    AxisRule {
        keywords: &["asymmetric", "different"],
        verdict: "asymmetric",
        confidence: "medium",
        rationale: "asymmetry keywords detected",
    },
];

const TIMING_RULES: &[AxisRule] = &[
    AxisRule {
        keywords: &["simultaneous", "at the same time"],
        verdict: "simultaneous",
        confidence: "medium",
        rationale: "simultaneous keyword detected",
    },
    AxisRule {
        keywords: &["sequential", "take turns", "first mover"],
        verdict: "sequential",
        confidence: "medium",
        rationale: "sequential keyword detected",
    },
    AxisRule {
        keywords: &["repeated", "ongoing"],
        verdict: "repeated",
        confidence: "medium",
        rationale: "repeated keyword detected",
    },
];

const PERFECT_INFO_RULES: &[AxisRule] = &[
    AxisRule {
        keywords: &[
            "perfect information",
            "knows everything",
            "full information",
        ],
        verdict: "perfect",
        confidence: "medium",
        rationale: "perfect information keywords",
    },
    AxisRule {
        keywords: &["imperfect", "hidden", "private"],
        verdict: "imperfect",
        confidence: "medium",
        rationale: "imperfect information keywords",
    },
];

const COMPLETE_INFO_RULES: &[AxisRule] = &[
    AxisRule {
        keywords: &[
            "incomplete",
            "doesn't know",
            "unknown",
            "private type",
            "asymmetric information",
        ],
        verdict: "incomplete",
        confidence: "medium",
        rationale: "incomplete information keywords",
    },
    AxisRule {
        keywords: &["complete information", "knows everything about"],
        verdict: "complete",
        confidence: "medium",
        rationale: "complete information keywords",
    },
];

const CARDINALITY_RULES: &[AxisRule] = &[
    AxisRule {
        keywords: &["two player", "two firm", "bilateral", "duopoly"],
        verdict: "2-player",
        confidence: "medium",
        rationale: "two-player keywords",
    },
    AxisRule {
        keywords: &["n-player", "multi", "many", "oligopoly", "market"],
        verdict: "n-player",
        confidence: "medium",
        rationale: "multi-player keywords",
    },
];

const STRATEGY_SPACE_RULES: &[AxisRule] = &[
    AxisRule {
        keywords: &["continuous", "price", "quantity", "amount"],
        verdict: "continuous",
        confidence: "medium",
        rationale: "continuous strategy indicators",
    },
    AxisRule {
        keywords: &["discrete", "binary", "yes/no", "choice"],
        verdict: "discrete",
        confidence: "medium",
        rationale: "discrete strategy indicators",
    },
];

const HORIZON_RULES: &[AxisRule] = &[
    AxisRule {
        keywords: &["one-shot", "once", "single"],
        verdict: "one-shot",
        confidence: "medium",
        rationale: "one-shot keywords",
    },
    AxisRule {
        keywords: &["repeated", "ongoing", "infinitely", "recurrent"],
        verdict: "repeated",
        confidence: "medium",
        rationale: "repeated keywords",
    },
];

const FAMILY_RULES: &[FamilyRule] = &[
    FamilyRule {
        all: &["price", "simultaneous"],
        any: &[],
        primary_family: "Bertrand competition",
        nearest_classic: Some("Bertrand duopoly"),
    },
    FamilyRule {
        all: &["quantity", "simultaneous"],
        any: &[],
        primary_family: "Cournot competition",
        nearest_classic: Some("Cournot duopoly"),
    },
    FamilyRule {
        all: &["price", "sequential"],
        any: &[],
        primary_family: "Stackelberg price leadership",
        nearest_classic: Some("Stackelberg duopoly"),
    },
    FamilyRule {
        all: &[],
        any: &["dilemma", "defect", "cooperate vs"],
        primary_family: "Social dilemma",
        nearest_classic: Some("Prisoner's Dilemma"),
    },
    FamilyRule {
        all: &[],
        any: &["coordinate", "standard", "compatible"],
        primary_family: "Coordination game",
        nearest_classic: Some("Battle of the Sexes"),
    },
    FamilyRule {
        all: &[],
        any: &["auction", "bid"],
        primary_family: "Auction",
        nearest_classic: Some("First-price sealed-bid auction"),
    },
    FamilyRule {
        all: &[],
        any: &["negotiate", "bargain", "offer"],
        primary_family: "Bargaining",
        nearest_classic: Some("Ultimatum Game"),
    },
    FamilyRule {
        all: &[],
        any: &["deter", "threat", "retaliate"],
        primary_family: "Deterrence",
        nearest_classic: Some("Chicken / Hawk-Dove"),
    },
];

fn axis_verdict(text: &str, rules: &[AxisRule], default: AxisRule) -> AxisVerdict {
    let rule = rules
        .iter()
        .find(|rule| any_keyword(text, rule.keywords))
        .copied()
        .unwrap_or(default);
    AxisVerdict::new(rule.verdict, rule.confidence, rule.rationale)
}

fn family_verdict(text: &str) -> (String, Option<String>) {
    let Some(rule) = FAMILY_RULES.iter().find(|rule| {
        all_keywords(text, rule.all) && (rule.any.is_empty() || any_keyword(text, rule.any))
    }) else {
        return ("Strategic interaction".into(), None);
    };

    (
        rule.primary_family.into(),
        rule.nearest_classic.map(String::from),
    )
}

fn cardinality_verdict(text: &str) -> AxisVerdict {
    if all_keywords(text, &["two", "player"]) {
        return AxisVerdict::new("2-player", "medium", "two-player keywords");
    }
    axis_verdict(
        text,
        CARDINALITY_RULES,
        AxisRule {
            keywords: &[],
            verdict: "2-player",
            confidence: "low",
            rationale: "default assumption",
        },
    )
}

fn any_keyword(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|keyword| text.contains(keyword))
}

fn all_keywords(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().all(|keyword| text.contains(keyword))
}

/// Generate a keyword-based fingerprint as fallback when no LLM provider is available.
///
/// Performs simple keyword analysis of the situation text. Less accurate than
/// real Tier 1 classification but requires no external dependencies.
pub(super) fn keyword_fallback_fingerprint(
    run_id: &str,
    situation: &str,
    now: &str,
) -> GameTheoryFingerprint {
    let s = situation.to_lowercase();

    let cooperation = axis_verdict(
        &s,
        COOPERATION_RULES,
        AxisRule {
            keywords: &[],
            verdict: "non-cooperative",
            confidence: "medium",
            rationale: "default for unmarked situations",
        },
    );
    let payoff_sum = axis_verdict(
        &s,
        PAYOFF_SUM_RULES,
        AxisRule {
            keywords: &[],
            verdict: "variable-sum",
            confidence: "low",
            rationale: "insufficient payoff information",
        },
    );
    let symmetry = axis_verdict(
        &s,
        SYMMETRY_RULES,
        AxisRule {
            keywords: &[],
            verdict: "unknown",
            confidence: "low",
            rationale: "insufficient symmetry information",
        },
    );
    let timing = axis_verdict(
        &s,
        TIMING_RULES,
        AxisRule {
            keywords: &[],
            verdict: "simultaneous",
            confidence: "low",
            rationale: "default assumption",
        },
    );
    let perfect_info = axis_verdict(
        &s,
        PERFECT_INFO_RULES,
        AxisRule {
            keywords: &[],
            verdict: "imperfect",
            confidence: "low",
            rationale: "most real situations have imperfect info",
        },
    );
    let complete_info = axis_verdict(
        &s,
        COMPLETE_INFO_RULES,
        AxisRule {
            keywords: &[],
            verdict: "incomplete",
            confidence: "low",
            rationale: "most real situations have incomplete info",
        },
    );
    let cardinality = cardinality_verdict(&s);
    let strategy_space = axis_verdict(
        &s,
        STRATEGY_SPACE_RULES,
        AxisRule {
            keywords: &[],
            verdict: "discrete",
            confidence: "low",
            rationale: "default assumption",
        },
    );
    let horizon = axis_verdict(
        &s,
        HORIZON_RULES,
        AxisRule {
            keywords: &[],
            verdict: "one-shot",
            confidence: "low",
            rationale: "default assumption",
        },
    );
    let (primary_family, nearest_classic) = family_verdict(&s);

    let ambiguities = if situation.len() < 50 {
        vec![AmbiguityNote {
            axis: "all".into(),
            note: "situation too brief for confident classification".into(),
        }]
    } else if !s.contains("payoff")
        && !s.contains("utility")
        && !s.contains("profit")
        && !s.contains("cost")
    {
        vec![AmbiguityNote {
            axis: "payoff_sum".into(),
            note: "no payoff or utility information provided".into(),
        }]
    } else {
        vec![]
    };

    let shadow_games = shadow_games(&s);

    let hidden_game_scan = if !shadow_games.is_empty() {
        Some(HiddenGameDetection {
            game_name: shadow_games[0].clone(),
            confidence: "low".into(),
            description: "potential hidden cooperative structure in competitive framing".into(),
        })
    } else {
        None
    };

    GameTheoryFingerprint {
        run_id: run_id.to_string(),
        cooperation,
        payoff_sum,
        symmetry,
        timing,
        perfect_info,
        complete_info,
        cardinality,
        strategy_space,
        horizon,
        primary_family,
        nearest_classic,
        shadow_games,
        hidden_game_scan,
        ambiguities,
        created_at: now.to_string(),
    }
}

fn shadow_games(text: &str) -> Vec<String> {
    if text.contains("price") && !text.contains("collude") && !text.contains("cartel") {
        vec!["Prisoner's Dilemma (tacit collusion shadow)".into()]
    } else {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_fallback_classifies_bertrand_shadow_game() {
        let fp = keyword_fallback_fingerprint(
            "run-1",
            "Two firms set price simultaneously for profit in a duopoly market.",
            "now",
        );

        assert_eq!(fp.primary_family, "Bertrand competition");
        assert_eq!(fp.nearest_classic.as_deref(), Some("Bertrand duopoly"));
        assert_eq!(fp.cardinality.value, "2-player");
        assert_eq!(fp.strategy_space.value, "continuous");
        assert_eq!(
            fp.hidden_game_scan
                .as_ref()
                .map(|scan| scan.game_name.as_str()),
            Some("Prisoner's Dilemma (tacit collusion shadow)")
        );
    }

    #[test]
    fn keyword_fallback_preserves_two_and_player_cardinality_rule() {
        let fp = keyword_fallback_fingerprint(
            "run-2",
            "Two firms each choose a player role with unknown payoff information.",
            "now",
        );

        assert_eq!(fp.cardinality.value, "2-player");
        assert_eq!(fp.cardinality.confidence, "medium");
    }
}
