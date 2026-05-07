use super::super::agents::{GameTheoryAgent, GameTheoryToolAccess};
use GameTheoryToolAccess::{Glob, Grep, Read, WebFetch, WebSearch};

const READ_GREP_GLOB: &[GameTheoryToolAccess] = &[Read, Grep, Glob];
const READ_GREP_GLOB_WEB_SEARCH: &[GameTheoryToolAccess] = &[Read, Grep, Glob, WebSearch];
const READ_GREP_GLOB_WEB_FETCH: &[GameTheoryToolAccess] = &[Read, Grep, Glob, WebFetch];
const READ_GREP_GLOB_WEB_FETCH_WEB_SEARCH: &[GameTheoryToolAccess] =
    &[Read, Grep, Glob, WebFetch, WebSearch];
const READ_WEB_FETCH_WEB_SEARCH: &[GameTheoryToolAccess] = &[Read, WebFetch, WebSearch];

macro_rules! gt_agent {
    ($key:literal, $tier:literal) => {
        gt_agent!($key, $tier, &[], &[], READ_GREP_GLOB, false)
    };
    (
        $key:literal,
        $tier:literal,
        $memory_keys:expr,
        $output_artifacts:expr,
        $tool_access:expr,
        $mandatory:expr
    ) => {
        GameTheoryAgent {
            key: $key,
            display_name: $key,
            tier: $tier,
            file: concat!($key, ".md"),
            memory_keys: $memory_keys,
            output_artifacts: $output_artifacts,
            prompt_source_path: concat!(".archon/agents/gametheory/", $key, ".md"),
            tool_access: $tool_access,
            model: "opus",
            condition: None,
            depends_on: &[],
            mandatory: $mandatory,
        }
    };
}

pub static GAMETHEORY_AGENTS: &[GameTheoryAgent] = &[
    gt_agent!("asymmetric-info-detective", 6),
    gt_agent!("auction-strategist", 7),
    gt_agent!("backward-induction-solver", 5),
    gt_agent!("banzhaf-power-auditor", 3),
    gt_agent!("battle-of-sexes-coordinator", 4),
    gt_agent!("bayesian-belief-updater", 6),
    gt_agent!("bayesian-equilibrium-analyst", 2),
    gt_agent!("behavioral-bias-detector", 8),
    gt_agent!("bluff-and-deception-analyst", 9),
    gt_agent!("brinkmanship-tactician", 9),
    gt_agent!(
        "business-strategy-gamifier",
        10,
        &[],
        &[],
        READ_GREP_GLOB_WEB_SEARCH,
        false
    ),
    gt_agent!("centipede-game-analyst", 4),
    gt_agent!("cheap-talk-evaluator", 6),
    gt_agent!("chicken-brinksmanship-tactician", 4),
    gt_agent!("coalition-formation-strategist", 3),
    gt_agent!("cohesion-discipline-devotion-auditor", 11),
    gt_agent!("commitment-device-engineer", 9),
    gt_agent!("common-knowledge-analyst", 12),
    gt_agent!("conflict-resolution-theorist", 10),
    gt_agent!("cooperation-emergence-analyst", 5),
    gt_agent!("coopetition-strategist", 9),
    gt_agent!("core-stability-analyst", 3),
    gt_agent!("correlated-equilibrium-designer", 2),
    gt_agent!("counterfactual-simulator", 12),
    gt_agent!("credibility-assessor", 6),
    gt_agent!("deterrence-theorist", 9),
    gt_agent!("dialectic-tension-mapper", 11),
    gt_agent!("diaspora-dynamics-analyst", 11),
    gt_agent!("dominant-strategy-identifier", 2),
    gt_agent!("elite-overproduction-diagnostician", 11),
    gt_agent!("equilibrium-selector", 12),
    gt_agent!("ess-detector", 8),
    gt_agent!("evolutionary-strategy-analyst", 8),
    gt_agent!("extensive-form-modeler", 1),
    gt_agent!("fairness-preferences-analyst", 8),
    gt_agent!("father-son-dynastic-analyst", 11),
    gt_agent!("first-mover-analyst", 9),
    gt_agent!("focal-point-identifier", 9),
    gt_agent!("folk-theorem-applier", 5),
    gt_agent!(
        "game-classifier",
        1,
        &["gametheory/situation"],
        &["gametheory/tier1/fingerprint"],
        READ_GREP_GLOB_WEB_FETCH_WEB_SEARCH,
        true
    ),
    gt_agent!("game-tree-archaeologist", 12),
    gt_agent!(
        "geopolitical-game-analyst",
        10,
        &[],
        &[],
        READ_GREP_GLOB_WEB_SEARCH,
        false
    ),
    gt_agent!("incentive-compatibility-auditor", 7),
    gt_agent!(
        "information-structure-mapper",
        1,
        &["gametheory/situation"],
        &["gametheory/tier1/information-structure"],
        READ_GREP_GLOB_WEB_FETCH,
        true
    ),
    gt_agent!("legitimacy-crisis-analyst", 11),
    gt_agent!("level-k-reasoning-profiler", 8),
    gt_agent!("loss-aversion-analyst", 8),
    gt_agent!("market-competition-modeler", 10),
    gt_agent!("matching-market-designer", 7),
    gt_agent!("matching-pennies-randomizer", 4),
    gt_agent!("mechanism-designer", 7),
    gt_agent!("meta-game-designer", 12),
    gt_agent!("mixed-strategy-calculator", 2),
    gt_agent!("myth-making-strategist", 11),
    gt_agent!(
        "nash-equilibrium-finder",
        2,
        &["gametheory/tier1/payoffs", "gametheory/tier1/strategies"],
        &["gametheory/tier2/nash-equilibrium"],
        READ_GREP_GLOB,
        false
    ),
    gt_agent!("negotiation-strategist", 9),
    gt_agent!("nucleolus-calculator", 3),
    gt_agent!(
        "payoff-elicitor",
        1,
        &["gametheory/situation"],
        &["gametheory/tier1/payoffs"],
        READ_WEB_FETCH_WEB_SEARCH,
        true
    ),
    gt_agent!(
        "payoff-matrix-builder",
        1,
        &["gametheory/tier1/payoffs", "gametheory/tier1/strategies"],
        &["gametheory/tier1/payoff-matrix"],
        READ_GREP_GLOB,
        false
    ),
    gt_agent!("poor-conquers-rich-analyst", 11),
    gt_agent!("power-transition-analyst", 11),
    gt_agent!("prisoners-dilemma-detector", 4),
    gt_agent!(
        "propaganda-detector",
        11,
        &[],
        &[],
        READ_GREP_GLOB_WEB_SEARCH,
        false
    ),
    gt_agent!("public-goods-diagnostician", 4),
    gt_agent!("quantal-response-modeler", 8),
    gt_agent!("reputation-game-modeler", 5),
    gt_agent!("revenue-equivalence-analyst", 7),
    gt_agent!("screening-mechanism-designer", 6),
    gt_agent!("shapley-value-calculator", 3),
    gt_agent!("signaling-game-analyst", 6),
    gt_agent!("social-interaction-gamifier", 10),
    gt_agent!("stag-hunt-analyst", 4),
    gt_agent!("stochastic-game-analyst", 5),
    gt_agent!(
        "strategy-space-enumerator",
        1,
        &["gametheory/situation"],
        &["gametheory/tier1/strategies"],
        READ_GREP_GLOB,
        true
    ),
    gt_agent!("subgame-perfect-analyzer", 2),
    gt_agent!("threat-credibility-assessor", 9),
    gt_agent!("tit-for-tat-strategist", 5),
    gt_agent!("tragedy-commons-analyst", 4),
    gt_agent!("trembling-hand-refiner", 2),
    gt_agent!("trust-game-analyst", 4),
    gt_agent!("ultimatum-bargainer", 4),
    gt_agent!("vcg-architect", 7),
    gt_agent!("voting-strategy-analyst", 10),
    gt_agent!("war-of-attrition-analyst", 9),
];
