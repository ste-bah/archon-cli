use super::super::fingerprint::{
    AmbiguityNote, AxisVerdict, GameTheoryFingerprint, HiddenGameDetection,
};

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

    let cooperation = if s.contains("collaborate")
        || s.contains("cooperate")
        || s.contains("alliance")
        || s.contains("cartel")
    {
        AxisVerdict::new("cooperative", "medium", "cooperation keywords detected")
    } else {
        AxisVerdict::new(
            "non-cooperative",
            "medium",
            "default for unmarked situations",
        )
    };

    let payoff_sum =
        if s.contains("zero-sum") || s.contains("winner-take") || s.contains("all or nothing") {
            AxisVerdict::new("zero-sum", "medium", "zero-sum keywords detected")
        } else if s.contains("win-win") || s.contains("mutual gain") || s.contains("positive-sum") {
            AxisVerdict::new("positive-sum", "medium", "positive-sum keywords detected")
        } else {
            AxisVerdict::new("variable-sum", "low", "insufficient payoff information")
        };

    let symmetry = if s.contains("symmetric") || s.contains("identical") || s.contains("same") {
        AxisVerdict::new("symmetric", "medium", "symmetry keywords detected")
    } else if s.contains("asymmetric") || s.contains("different") {
        AxisVerdict::new("asymmetric", "medium", "asymmetry keywords detected")
    } else {
        AxisVerdict::new("unknown", "low", "insufficient symmetry information")
    };

    let timing = if s.contains("simultaneous") || s.contains("at the same time") {
        AxisVerdict::new("simultaneous", "medium", "simultaneous keyword detected")
    } else if s.contains("sequential") || s.contains("take turns") || s.contains("first mover") {
        AxisVerdict::new("sequential", "medium", "sequential keyword detected")
    } else if s.contains("repeated") || s.contains("ongoing") {
        AxisVerdict::new("repeated", "medium", "repeated keyword detected")
    } else {
        AxisVerdict::new("simultaneous", "low", "default assumption")
    };

    let perfect_info = if s.contains("perfect information")
        || s.contains("knows everything")
        || s.contains("full information")
    {
        AxisVerdict::new("perfect", "medium", "perfect information keywords")
    } else if s.contains("imperfect") || s.contains("hidden") || s.contains("private") {
        AxisVerdict::new("imperfect", "medium", "imperfect information keywords")
    } else {
        AxisVerdict::new(
            "imperfect",
            "low",
            "most real situations have imperfect info",
        )
    };

    let complete_info = if s.contains("incomplete")
        || s.contains("doesn't know")
        || s.contains("unknown")
        || s.contains("private type")
        || s.contains("asymmetric information")
    {
        AxisVerdict::new("incomplete", "medium", "incomplete information keywords")
    } else if s.contains("complete information") || s.contains("knows everything about") {
        AxisVerdict::new("complete", "medium", "complete information keywords")
    } else {
        AxisVerdict::new(
            "incomplete",
            "low",
            "most real situations have incomplete info",
        )
    };

    let cardinality = if s.contains("two player")
        || s.contains("two firm")
        || s.contains("bilateral")
        || s.contains("duopoly")
        || (s.contains("two") && s.contains("player"))
    {
        AxisVerdict::new("2-player", "medium", "two-player keywords")
    } else if s.contains("n-player")
        || s.contains("multi")
        || s.contains("many")
        || s.contains("oligopoly")
        || s.contains("market")
    {
        AxisVerdict::new("n-player", "medium", "multi-player keywords")
    } else {
        AxisVerdict::new("2-player", "low", "default assumption")
    };

    let strategy_space = if s.contains("continuous")
        || s.contains("price")
        || s.contains("quantity")
        || s.contains("amount")
    {
        AxisVerdict::new("continuous", "medium", "continuous strategy indicators")
    } else if s.contains("discrete")
        || s.contains("binary")
        || s.contains("yes/no")
        || s.contains("choice")
    {
        AxisVerdict::new("discrete", "medium", "discrete strategy indicators")
    } else {
        AxisVerdict::new("discrete", "low", "default assumption")
    };

    let horizon = if s.contains("one-shot") || s.contains("once") || s.contains("single") {
        AxisVerdict::new("one-shot", "medium", "one-shot keywords")
    } else if s.contains("repeated")
        || s.contains("ongoing")
        || s.contains("infinitely")
        || s.contains("recurrent")
    {
        AxisVerdict::new("repeated", "medium", "repeated keywords")
    } else {
        AxisVerdict::new("one-shot", "low", "default assumption")
    };

    let (primary_family, nearest_classic) = if s.contains("price") && s.contains("simultaneous") {
        (
            "Bertrand competition".into(),
            Some("Bertrand duopoly".into()),
        )
    } else if s.contains("quantity") && s.contains("simultaneous") {
        ("Cournot competition".into(), Some("Cournot duopoly".into()))
    } else if s.contains("price") && s.contains("sequential") {
        (
            "Stackelberg price leadership".into(),
            Some("Stackelberg duopoly".into()),
        )
    } else if s.contains("dilemma") || s.contains("defect") || s.contains("cooperate vs") {
        ("Social dilemma".into(), Some("Prisoner's Dilemma".into()))
    } else if s.contains("coordinate") || s.contains("standard") || s.contains("compatible") {
        (
            "Coordination game".into(),
            Some("Battle of the Sexes".into()),
        )
    } else if s.contains("auction") || s.contains("bid") {
        (
            "Auction".into(),
            Some("First-price sealed-bid auction".into()),
        )
    } else if s.contains("negotiate") || s.contains("bargain") || s.contains("offer") {
        ("Bargaining".into(), Some("Ultimatum Game".into()))
    } else if s.contains("deter") || s.contains("threat") || s.contains("retaliate") {
        ("Deterrence".into(), Some("Chicken / Hawk-Dove".into()))
    } else {
        ("Strategic interaction".into(), None::<String>)
    };

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

    let shadow_games: Vec<String> =
        if s.contains("price") && !s.contains("collude") && !s.contains("cartel") {
            vec!["Prisoner's Dilemma (tacit collusion shadow)".into()]
        } else {
            vec![]
        };

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
