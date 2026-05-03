use super::agents::{GameTheoryAgent, GameTheoryTier, GameTheoryToolAccess};

pub static GAMETHEORY_TIERS: &[GameTheoryTier] = &[
    GameTheoryTier {
        id: 1,
        name: "Core Analysts",
        description: "",
        agent_keys: &[
        "extensive-form-modeler",
        "game-classifier",
        "information-structure-mapper",
        "payoff-elicitor",
        "payoff-matrix-builder",
        "strategy-space-enumerator"
    ] },
    GameTheoryTier {
        id: 2,
        name: "Equilibrium Finders",
        description: "",
        agent_keys: &[
        "bayesian-equilibrium-analyst",
        "correlated-equilibrium-designer",
        "dominant-strategy-identifier",
        "mixed-strategy-calculator",
        "nash-equilibrium-finder",
        "subgame-perfect-analyzer",
        "trembling-hand-refiner"
    ] },
    GameTheoryTier {
        id: 3,
        name: "Cooperative Game Specialists",
        description: "",
        agent_keys: &[
        "banzhaf-power-auditor",
        "coalition-formation-strategist",
        "core-stability-analyst",
        "nucleolus-calculator",
        "shapley-value-calculator"
    ] },
    GameTheoryTier {
        id: 4,
        name: "Classic Game Pattern Recognizers",
        description: "",
        agent_keys: &[
        "battle-of-sexes-coordinator",
        "centipede-game-analyst",
        "chicken-brinksmanship-tactician",
        "matching-pennies-randomizer",
        "prisoners-dilemma-detector",
        "public-goods-diagnostician",
        "stag-hunt-analyst",
        "tragedy-commons-analyst",
        "trust-game-analyst",
        "ultimatum-bargainer"
    ] },
    GameTheoryTier {
        id: 5,
        name: "Dynamic & Repeated Games",
        description: "",
        agent_keys: &[
        "backward-induction-solver",
        "cooperation-emergence-analyst",
        "folk-theorem-applier",
        "reputation-game-modeler",
        "stochastic-game-analyst",
        "tit-for-tat-strategist"
    ] },
    GameTheoryTier {
        id: 6,
        name: "Information & Bayesian Games",
        description: "",
        agent_keys: &[
        "asymmetric-info-detective",
        "bayesian-belief-updater",
        "cheap-talk-evaluator",
        "credibility-assessor",
        "screening-mechanism-designer",
        "signaling-game-analyst"
    ] },
    GameTheoryTier {
        id: 7,
        name: "Mechanism Design & Auctions",
        description: "",
        agent_keys: &[
        "auction-strategist",
        "incentive-compatibility-auditor",
        "matching-market-designer",
        "mechanism-designer",
        "revenue-equivalence-analyst",
        "vcg-architect"
    ] },
    GameTheoryTier {
        id: 8,
        name: "Behavioral & Evolutionary Games",
        description: "",
        agent_keys: &[
        "behavioral-bias-detector",
        "ess-detector",
        "evolutionary-strategy-analyst",
        "fairness-preferences-analyst",
        "level-k-reasoning-profiler",
        "loss-aversion-analyst",
        "quantal-response-modeler"
    ] },
    GameTheoryTier {
        id: 9,
        name: "Strategic Tactics & Conflict",
        description: "",
        agent_keys: &[
        "bluff-and-deception-analyst",
        "brinkmanship-tactician",
        "commitment-device-engineer",
        "coopetition-strategist",
        "deterrence-theorist",
        "first-mover-analyst",
        "focal-point-identifier",
        "negotiation-strategist",
        "threat-credibility-assessor",
        "war-of-attrition-analyst"
    ] },
    GameTheoryTier {
        id: 10,
        name: "Applied Game Theory",
        description: "",
        agent_keys: &[
        "business-strategy-gamifier",
        "conflict-resolution-theorist",
        "geopolitical-game-analyst",
        "market-competition-modeler",
        "social-interaction-gamifier",
        "voting-strategy-analyst"
    ] },
    GameTheoryTier {
        id: 11,
        name: "Social & Political Games",
        description: "",
        agent_keys: &[
        "cohesion-discipline-devotion-auditor",
        "dialectic-tension-mapper",
        "diaspora-dynamics-analyst",
        "elite-overproduction-diagnostician",
        "father-son-dynastic-analyst",
        "legitimacy-crisis-analyst",
        "myth-making-strategist",
        "poor-conquers-rich-analyst",
        "power-transition-analyst",
        "propaganda-detector"
    ] },
    GameTheoryTier {
        id: 12,
        name: "Meta-Game & Synthesis",
        description: "",
        agent_keys: &[
        "common-knowledge-analyst",
        "counterfactual-simulator",
        "equilibrium-selector",
        "game-tree-archaeologist",
        "meta-game-designer"
    ] },
];

pub static GAMETHEORY_AGENTS: &[GameTheoryAgent] = &[
    GameTheoryAgent {
        key: "asymmetric-info-detective",
        display_name: "asymmetric-info-detective",
        tier: 6,
        file: "asymmetric-info-detective.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/asymmetric-info-detective.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "auction-strategist",
        display_name: "auction-strategist",
        tier: 7,
        file: "auction-strategist.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/auction-strategist.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "backward-induction-solver",
        display_name: "backward-induction-solver",
        tier: 5,
        file: "backward-induction-solver.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/backward-induction-solver.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "banzhaf-power-auditor",
        display_name: "banzhaf-power-auditor",
        tier: 3,
        file: "banzhaf-power-auditor.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/banzhaf-power-auditor.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "battle-of-sexes-coordinator",
        display_name: "battle-of-sexes-coordinator",
        tier: 4,
        file: "battle-of-sexes-coordinator.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/battle-of-sexes-coordinator.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "bayesian-belief-updater",
        display_name: "bayesian-belief-updater",
        tier: 6,
        file: "bayesian-belief-updater.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/bayesian-belief-updater.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "bayesian-equilibrium-analyst",
        display_name: "bayesian-equilibrium-analyst",
        tier: 2,
        file: "bayesian-equilibrium-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/bayesian-equilibrium-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "behavioral-bias-detector",
        display_name: "behavioral-bias-detector",
        tier: 8,
        file: "behavioral-bias-detector.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/behavioral-bias-detector.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "bluff-and-deception-analyst",
        display_name: "bluff-and-deception-analyst",
        tier: 9,
        file: "bluff-and-deception-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/bluff-and-deception-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "brinkmanship-tactician",
        display_name: "brinkmanship-tactician",
        tier: 9,
        file: "brinkmanship-tactician.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/brinkmanship-tactician.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "business-strategy-gamifier",
        display_name: "business-strategy-gamifier",
        tier: 10,
        file: "business-strategy-gamifier.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/business-strategy-gamifier.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob, GameTheoryToolAccess::WebSearch],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "centipede-game-analyst",
        display_name: "centipede-game-analyst",
        tier: 4,
        file: "centipede-game-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/centipede-game-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "cheap-talk-evaluator",
        display_name: "cheap-talk-evaluator",
        tier: 6,
        file: "cheap-talk-evaluator.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/cheap-talk-evaluator.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "chicken-brinksmanship-tactician",
        display_name: "chicken-brinksmanship-tactician",
        tier: 4,
        file: "chicken-brinksmanship-tactician.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/chicken-brinksmanship-tactician.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "coalition-formation-strategist",
        display_name: "coalition-formation-strategist",
        tier: 3,
        file: "coalition-formation-strategist.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/coalition-formation-strategist.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "cohesion-discipline-devotion-auditor",
        display_name: "cohesion-discipline-devotion-auditor",
        tier: 11,
        file: "cohesion-discipline-devotion-auditor.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/cohesion-discipline-devotion-auditor.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "commitment-device-engineer",
        display_name: "commitment-device-engineer",
        tier: 9,
        file: "commitment-device-engineer.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/commitment-device-engineer.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "common-knowledge-analyst",
        display_name: "common-knowledge-analyst",
        tier: 12,
        file: "common-knowledge-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/common-knowledge-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "conflict-resolution-theorist",
        display_name: "conflict-resolution-theorist",
        tier: 10,
        file: "conflict-resolution-theorist.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/conflict-resolution-theorist.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "cooperation-emergence-analyst",
        display_name: "cooperation-emergence-analyst",
        tier: 5,
        file: "cooperation-emergence-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/cooperation-emergence-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "coopetition-strategist",
        display_name: "coopetition-strategist",
        tier: 9,
        file: "coopetition-strategist.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/coopetition-strategist.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "core-stability-analyst",
        display_name: "core-stability-analyst",
        tier: 3,
        file: "core-stability-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/core-stability-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "correlated-equilibrium-designer",
        display_name: "correlated-equilibrium-designer",
        tier: 2,
        file: "correlated-equilibrium-designer.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/correlated-equilibrium-designer.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "counterfactual-simulator",
        display_name: "counterfactual-simulator",
        tier: 12,
        file: "counterfactual-simulator.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/counterfactual-simulator.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "credibility-assessor",
        display_name: "credibility-assessor",
        tier: 6,
        file: "credibility-assessor.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/credibility-assessor.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "deterrence-theorist",
        display_name: "deterrence-theorist",
        tier: 9,
        file: "deterrence-theorist.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/deterrence-theorist.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "dialectic-tension-mapper",
        display_name: "dialectic-tension-mapper",
        tier: 11,
        file: "dialectic-tension-mapper.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/dialectic-tension-mapper.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "diaspora-dynamics-analyst",
        display_name: "diaspora-dynamics-analyst",
        tier: 11,
        file: "diaspora-dynamics-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/diaspora-dynamics-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "dominant-strategy-identifier",
        display_name: "dominant-strategy-identifier",
        tier: 2,
        file: "dominant-strategy-identifier.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/dominant-strategy-identifier.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "elite-overproduction-diagnostician",
        display_name: "elite-overproduction-diagnostician",
        tier: 11,
        file: "elite-overproduction-diagnostician.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/elite-overproduction-diagnostician.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "equilibrium-selector",
        display_name: "equilibrium-selector",
        tier: 12,
        file: "equilibrium-selector.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/equilibrium-selector.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "ess-detector",
        display_name: "ess-detector",
        tier: 8,
        file: "ess-detector.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/ess-detector.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "evolutionary-strategy-analyst",
        display_name: "evolutionary-strategy-analyst",
        tier: 8,
        file: "evolutionary-strategy-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/evolutionary-strategy-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "extensive-form-modeler",
        display_name: "extensive-form-modeler",
        tier: 1,
        file: "extensive-form-modeler.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/extensive-form-modeler.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "fairness-preferences-analyst",
        display_name: "fairness-preferences-analyst",
        tier: 8,
        file: "fairness-preferences-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/fairness-preferences-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "father-son-dynastic-analyst",
        display_name: "father-son-dynastic-analyst",
        tier: 11,
        file: "father-son-dynastic-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/father-son-dynastic-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "first-mover-analyst",
        display_name: "first-mover-analyst",
        tier: 9,
        file: "first-mover-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/first-mover-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "focal-point-identifier",
        display_name: "focal-point-identifier",
        tier: 9,
        file: "focal-point-identifier.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/focal-point-identifier.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "folk-theorem-applier",
        display_name: "folk-theorem-applier",
        tier: 5,
        file: "folk-theorem-applier.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/folk-theorem-applier.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "game-classifier",
        display_name: "game-classifier",
        tier: 1,
        file: "game-classifier.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/game-classifier.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob, GameTheoryToolAccess::WebFetch, GameTheoryToolAccess::WebSearch],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: true,
    },
    GameTheoryAgent {
        key: "game-tree-archaeologist",
        display_name: "game-tree-archaeologist",
        tier: 12,
        file: "game-tree-archaeologist.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/game-tree-archaeologist.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "geopolitical-game-analyst",
        display_name: "geopolitical-game-analyst",
        tier: 10,
        file: "geopolitical-game-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/geopolitical-game-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob, GameTheoryToolAccess::WebSearch],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "incentive-compatibility-auditor",
        display_name: "incentive-compatibility-auditor",
        tier: 7,
        file: "incentive-compatibility-auditor.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/incentive-compatibility-auditor.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "information-structure-mapper",
        display_name: "information-structure-mapper",
        tier: 1,
        file: "information-structure-mapper.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/information-structure-mapper.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob, GameTheoryToolAccess::WebFetch],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: true,
    },
    GameTheoryAgent {
        key: "legitimacy-crisis-analyst",
        display_name: "legitimacy-crisis-analyst",
        tier: 11,
        file: "legitimacy-crisis-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/legitimacy-crisis-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "level-k-reasoning-profiler",
        display_name: "level-k-reasoning-profiler",
        tier: 8,
        file: "level-k-reasoning-profiler.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/level-k-reasoning-profiler.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "loss-aversion-analyst",
        display_name: "loss-aversion-analyst",
        tier: 8,
        file: "loss-aversion-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/loss-aversion-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "market-competition-modeler",
        display_name: "market-competition-modeler",
        tier: 10,
        file: "market-competition-modeler.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/market-competition-modeler.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "matching-market-designer",
        display_name: "matching-market-designer",
        tier: 7,
        file: "matching-market-designer.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/matching-market-designer.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "matching-pennies-randomizer",
        display_name: "matching-pennies-randomizer",
        tier: 4,
        file: "matching-pennies-randomizer.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/matching-pennies-randomizer.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "mechanism-designer",
        display_name: "mechanism-designer",
        tier: 7,
        file: "mechanism-designer.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/mechanism-designer.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "meta-game-designer",
        display_name: "meta-game-designer",
        tier: 12,
        file: "meta-game-designer.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/meta-game-designer.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "mixed-strategy-calculator",
        display_name: "mixed-strategy-calculator",
        tier: 2,
        file: "mixed-strategy-calculator.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/mixed-strategy-calculator.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "myth-making-strategist",
        display_name: "myth-making-strategist",
        tier: 11,
        file: "myth-making-strategist.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/myth-making-strategist.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "nash-equilibrium-finder",
        display_name: "nash-equilibrium-finder",
        tier: 2,
        file: "nash-equilibrium-finder.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/nash-equilibrium-finder.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "negotiation-strategist",
        display_name: "negotiation-strategist",
        tier: 9,
        file: "negotiation-strategist.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/negotiation-strategist.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "nucleolus-calculator",
        display_name: "nucleolus-calculator",
        tier: 3,
        file: "nucleolus-calculator.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/nucleolus-calculator.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "payoff-elicitor",
        display_name: "payoff-elicitor",
        tier: 1,
        file: "payoff-elicitor.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/payoff-elicitor.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::WebFetch, GameTheoryToolAccess::WebSearch],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: true,
    },
    GameTheoryAgent {
        key: "payoff-matrix-builder",
        display_name: "payoff-matrix-builder",
        tier: 1,
        file: "payoff-matrix-builder.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/payoff-matrix-builder.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "poor-conquers-rich-analyst",
        display_name: "poor-conquers-rich-analyst",
        tier: 11,
        file: "poor-conquers-rich-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/poor-conquers-rich-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "power-transition-analyst",
        display_name: "power-transition-analyst",
        tier: 11,
        file: "power-transition-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/power-transition-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "prisoners-dilemma-detector",
        display_name: "prisoners-dilemma-detector",
        tier: 4,
        file: "prisoners-dilemma-detector.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/prisoners-dilemma-detector.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "propaganda-detector",
        display_name: "propaganda-detector",
        tier: 11,
        file: "propaganda-detector.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/propaganda-detector.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob, GameTheoryToolAccess::WebSearch],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "public-goods-diagnostician",
        display_name: "public-goods-diagnostician",
        tier: 4,
        file: "public-goods-diagnostician.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/public-goods-diagnostician.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "quantal-response-modeler",
        display_name: "quantal-response-modeler",
        tier: 8,
        file: "quantal-response-modeler.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/quantal-response-modeler.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "reputation-game-modeler",
        display_name: "reputation-game-modeler",
        tier: 5,
        file: "reputation-game-modeler.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/reputation-game-modeler.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "revenue-equivalence-analyst",
        display_name: "revenue-equivalence-analyst",
        tier: 7,
        file: "revenue-equivalence-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/revenue-equivalence-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "screening-mechanism-designer",
        display_name: "screening-mechanism-designer",
        tier: 6,
        file: "screening-mechanism-designer.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/screening-mechanism-designer.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "shapley-value-calculator",
        display_name: "shapley-value-calculator",
        tier: 3,
        file: "shapley-value-calculator.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/shapley-value-calculator.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "signaling-game-analyst",
        display_name: "signaling-game-analyst",
        tier: 6,
        file: "signaling-game-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/signaling-game-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "social-interaction-gamifier",
        display_name: "social-interaction-gamifier",
        tier: 10,
        file: "social-interaction-gamifier.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/social-interaction-gamifier.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "stag-hunt-analyst",
        display_name: "stag-hunt-analyst",
        tier: 4,
        file: "stag-hunt-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/stag-hunt-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "stochastic-game-analyst",
        display_name: "stochastic-game-analyst",
        tier: 5,
        file: "stochastic-game-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/stochastic-game-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "strategy-space-enumerator",
        display_name: "strategy-space-enumerator",
        tier: 1,
        file: "strategy-space-enumerator.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/strategy-space-enumerator.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: true,
    },
    GameTheoryAgent {
        key: "subgame-perfect-analyzer",
        display_name: "subgame-perfect-analyzer",
        tier: 2,
        file: "subgame-perfect-analyzer.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/subgame-perfect-analyzer.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "threat-credibility-assessor",
        display_name: "threat-credibility-assessor",
        tier: 9,
        file: "threat-credibility-assessor.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/threat-credibility-assessor.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "tit-for-tat-strategist",
        display_name: "tit-for-tat-strategist",
        tier: 5,
        file: "tit-for-tat-strategist.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/tit-for-tat-strategist.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "tragedy-commons-analyst",
        display_name: "tragedy-commons-analyst",
        tier: 4,
        file: "tragedy-commons-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/tragedy-commons-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "trembling-hand-refiner",
        display_name: "trembling-hand-refiner",
        tier: 2,
        file: "trembling-hand-refiner.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/trembling-hand-refiner.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "trust-game-analyst",
        display_name: "trust-game-analyst",
        tier: 4,
        file: "trust-game-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/trust-game-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "ultimatum-bargainer",
        display_name: "ultimatum-bargainer",
        tier: 4,
        file: "ultimatum-bargainer.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/ultimatum-bargainer.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "vcg-architect",
        display_name: "vcg-architect",
        tier: 7,
        file: "vcg-architect.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/vcg-architect.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "voting-strategy-analyst",
        display_name: "voting-strategy-analyst",
        tier: 10,
        file: "voting-strategy-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/voting-strategy-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
    GameTheoryAgent {
        key: "war-of-attrition-analyst",
        display_name: "war-of-attrition-analyst",
        tier: 9,
        file: "war-of-attrition-analyst.md",
        memory_keys: &[],
        output_artifacts: &[],
        prompt_source_path: ".archon/agents/gametheory/war-of-attrition-analyst.md",
        tool_access: &[GameTheoryToolAccess::Read, GameTheoryToolAccess::Grep, GameTheoryToolAccess::Glob],
        model: "opus",
        condition: None,
        depends_on: &[],
        mandatory: false,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_count_is_84() {
        assert_eq!(
            GAMETHEORY_AGENTS.len(),
            84,
            "registry must contain exactly 84 agents (86 .md files minus sherlock-holmes + code-simplifier)"
        );
    }

    #[test]
    fn test_registry_excludes_sherlock_and_simplifier() {
        for agent in GAMETHEORY_AGENTS.iter() {
            assert!(
                !agent.key.contains("sherlock"),
                "agent '{}' must not be sherlock-holmes",
                agent.key
            );
            assert!(
                !agent.key.contains("code-simplifier"),
                "agent '{}' must not be code-simplifier",
                agent.key
            );
        }
    }

    #[test]
    fn test_tier1_has_four_mandatory() {
        let mandatory: Vec<&GameTheoryAgent> = GAMETHEORY_AGENTS
            .iter()
            .filter(|a| a.mandatory)
            .collect();
        assert_eq!(
            mandatory.len(),
            4,
            "Tier 1 must have exactly 4 mandatory agents"
        );
        let keys: Vec<&str> = mandatory.iter().map(|a| a.key).collect();
        assert!(keys.contains(&"game-classifier"));
        assert!(keys.contains(&"payoff-elicitor"));
        assert!(keys.contains(&"information-structure-mapper"));
        assert!(keys.contains(&"strategy-space-enumerator"));
    }

    #[test]
    fn test_tiers_have_unique_ids() {
        let mut ids: Vec<u8> = GAMETHEORY_TIERS.iter().map(|t| t.id).collect();
        ids.sort();
        let orig_len = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), orig_len, "all tier IDs must be unique");
    }

    #[test]
    fn test_no_docs2_paths_in_registry() {
        for agent in GAMETHEORY_AGENTS.iter() {
            assert!(
                !agent.prompt_source_path.contains("docs2/"),
                "agent '{}' has docs2/ in path: {}",
                agent.key,
                agent.prompt_source_path
            );
        }
    }

    #[test]
    fn test_all_registry_paths_resolve_to_existing_files() {
        // CARGO_MANIFEST_DIR is crates/archon-pipeline/; workspace root is two levels up
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();

        for agent in GAMETHEORY_AGENTS.iter() {
            let full_path = workspace_root.join(agent.prompt_source_path);
            assert!(
                full_path.exists(),
                "agent '{}' path does not exist: {}",
                agent.key,
                full_path.display()
            );
        }
    }

    #[test]
    fn test_registry_path_format_matches_other_categories() {
        // Verify gametheory paths use same ".archon/agents/<category>/" prefix as research agents
        let research_agent = &crate::research::agents::RESEARCH_AGENTS[0];
        let gt_agent = &GAMETHEORY_AGENTS[0];

        // Both paths start with ".archon/agents/"
        assert!(
            research_agent.prompt_source_path.starts_with(".archon/agents/"),
            "research agent path must use .archon/agents/ prefix"
        );
        assert!(
            gt_agent.prompt_source_path.starts_with(".archon/agents/"),
            "gametheory agent path must use .archon/agents/ prefix, got: {}",
            gt_agent.prompt_source_path
        );

        // Both follow the pattern .archon/agents/<category>/<name>.md
        let re_parts: Vec<&str> = research_agent.prompt_source_path.split('/').collect();
        let gt_parts: Vec<&str> = gt_agent.prompt_source_path.split('/').collect();
        assert_eq!(re_parts.len(), 4, "research path must have 4 segments");
        assert_eq!(
            gt_parts.len(), 4,
            "gametheory path must have 4 segments, got: {}",
            gt_agent.prompt_source_path
        );
        assert!(gt_parts[3].ends_with(".md"), "last segment must be .md file");
    }

    #[test]
    fn test_yaml_tier1_matches_registry_mandatory_set() {
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let yaml_path = workspace_root.join(".archon/specs/gametheory.yaml");
        assert!(
            yaml_path.exists(),
            "gametheory.yaml not found at {}",
            yaml_path.display()
        );

        let spec =
            crate::gametheory::routing::load_spec(&yaml_path).expect("must load gametheory.yaml");

        // Collect Tier 1 mandatory agents from YAML
        let yaml_tier1: std::collections::HashSet<&str> = spec
            .tiers
            .iter()
            .filter(|t| t.id == 1)
            .flat_map(|t| t.agents.iter())
            .filter(|a| a.mandatory)
            .map(|a| a.key.as_str())
            .collect();

        // Collect mandatory agents from registry
        let registry_mandatory: std::collections::HashSet<&str> = GAMETHEORY_AGENTS
            .iter()
            .filter(|a| a.mandatory)
            .map(|a| a.key)
            .collect();

        assert_eq!(
            yaml_tier1, registry_mandatory,
            "YAML Tier 1 mandatory agents must exactly match registry mandatory agents.\n\
             YAML Tier 1 mandatory: {:?}\n\
             Registry mandatory:    {:?}",
            yaml_tier1, registry_mandatory
        );
    }

    #[test]
    fn test_no_excluded_agent_files_present() {
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let sherlock_path = workspace_root.join(".archon/agents/gametheory/sherlock-holmes.md");
        let simplifier_path = workspace_root.join(".archon/agents/gametheory/code-simplifier.md");

        assert!(
            !sherlock_path.exists(),
            "sherlock-holmes.md must NOT exist at {} — it is a non-game-theory agent excluded per OQ-GT-001",
            sherlock_path.display()
        );
        assert!(
            !simplifier_path.exists(),
            "code-simplifier.md must NOT exist at {} — it is a non-game-theory agent excluded per OQ-GT-001",
            simplifier_path.display()
        );
    }

    // ── Group 2: Full 12-tier YAML coverage tests ────────────────────────────

    fn load_yaml_spec() -> crate::gametheory::routing::GameTheorySpec {
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let yaml_path = workspace_root.join(".archon/specs/gametheory.yaml");
        crate::gametheory::routing::load_spec(&yaml_path).expect("must load gametheory.yaml")
    }

    /// Every registry agent key must appear in the YAML spec.
    #[test]
    fn test_yaml_covers_all_84_agents() {
        let spec = load_yaml_spec();
        let yaml_keys: std::collections::HashSet<&str> = spec
            .tiers
            .iter()
            .flat_map(|t| t.agents.iter().map(|a| a.key.as_str()))
            .collect();

        let registry_keys: std::collections::HashSet<&str> =
            GAMETHEORY_AGENTS.iter().map(|a| a.key).collect();

        let missing: Vec<_> = registry_keys
            .difference(&yaml_keys)
            .copied()
            .collect();

        assert!(
            missing.is_empty(),
            "YAML spec is missing {} registry agent(s): {:?}",
            missing.len(),
            missing
        );
        assert_eq!(yaml_keys.len(), 84, "YAML must cover exactly 84 agents");
    }

    /// Every YAML agent key must have a matching registry entry.
    #[test]
    fn test_yaml_no_orphan_agents() {
        let spec = load_yaml_spec();
        let registry_keys: std::collections::HashSet<&str> =
            GAMETHEORY_AGENTS.iter().map(|a| a.key).collect();

        for tier in &spec.tiers {
            for agent in &tier.agents {
                assert!(
                    registry_keys.contains(agent.key.as_str()),
                    "YAML agent '{}' in tier {} has no matching registry entry",
                    agent.key,
                    tier.id
                );
            }
        }
    }

    /// The full spec depends_on graph must contain no cycles.
    #[test]
    fn test_yaml_no_cycles() {
        let spec = load_yaml_spec();
        let fp = crate::gametheory::fingerprint::GameTheoryFingerprint {
            run_id: "cycle-check".into(),
            cooperation: crate::gametheory::fingerprint::AxisVerdict::new(
                "non-cooperative", "high", "",
            ),
            payoff_sum: crate::gametheory::fingerprint::AxisVerdict::new(
                "non-zero-sum", "high", "",
            ),
            symmetry: crate::gametheory::fingerprint::AxisVerdict::new(
                "asymmetric", "high", "",
            ),
            timing: crate::gametheory::fingerprint::AxisVerdict::new(
                "simultaneous", "high", "",
            ),
            perfect_info: crate::gametheory::fingerprint::AxisVerdict::new(
                "imperfect", "high", "",
            ),
            complete_info: crate::gametheory::fingerprint::AxisVerdict::new(
                "incomplete", "high", "",
            ),
            cardinality: crate::gametheory::fingerprint::AxisVerdict::new(
                "2-player", "high", "",
            ),
            strategy_space: crate::gametheory::fingerprint::AxisVerdict::new(
                "discrete", "high", "",
            ),
            horizon: crate::gametheory::fingerprint::AxisVerdict::new(
                "repeated", "high", "",
            ),
            primary_family: "test".into(),
            nearest_classic: None,
            shadow_games: vec![],
            hidden_game_scan: None,
            ambiguities: vec![],
            created_at: "2026-05-03T00:00:00Z".into(),
        };

        let result = crate::gametheory::routing::evaluate_routing(
            &spec,
            &fp,
            "cycle-check",
            "2026-05-03T00:00:00Z",
        );

        match result {
            Ok(decision) => {
                assert!(!decision.enabled_specialists.is_empty(),
                    "routing must enable at least the 4 mandatory Tier 1 agents");
            }
            Err(e) => panic!("YAML spec has errors (possibly a cycle): {e:?}"),
        }
    }

    /// Every condition expression in the YAML must parse successfully.
    #[test]
    fn test_yaml_conditions_parse() {
        let spec = load_yaml_spec();
        let mut failed = Vec::new();

        for tier in &spec.tiers {
            for agent in &tier.agents {
                if let Some(ref cond) = agent.condition {
                    if let Err(e) = crate::gametheory::routing::parse_condition(cond) {
                        failed.push((agent.key.clone(), cond.clone(), e));
                    }
                }
            }
        }

        assert!(
            failed.is_empty(),
            "{} condition(s) failed to parse:\n{}",
            failed.len(),
            failed
                .iter()
                .map(|(key, cond, err)| format!("  {key}: '{cond}' → {err}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
