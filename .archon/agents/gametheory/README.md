# Game Theory Agent Arsenal

*"Most situations look adversarial on the surface but contain hidden cooperative structure. Game theory's power is in formalizing that."*

A full-spectrum toolkit of **84 specialist worker agents** for analyzing any strategic situation — from two-person negotiations to civilizational conflicts, from market pricing to evolutionary biology, from boardroom politics to nuclear deterrence.

Each agent is a narrow specialist. **You** (or main Claude Code at your direction) pick the right specialist for the job. There is no orchestrator here by design — routing is your job, not the agents'.

Every agent is built on the Sherlock 8-layer methodology: **methodology before procedure, principles before steps** — Identity, Memory Architecture, Epistemology, Cardinal Rule, Bias Prevention, Mental Models, Phased Protocol, Self-Verification + Structured Output.

---

## HOW TO USE THIS ARSENAL

**You pick the specialist.** Find the agent whose specialty matches your situation in the catalog below, and invoke it:

- Natural language: *"Use the prisoners-dilemma-detector to analyze this pricing situation."*
- @-mention: `@prisoners-dilemma-detector`

For complex situations with multiple game-theoretic aspects, call multiple specialists in parallel from main Claude Code and synthesize their findings yourself. **No agent here will route to another** — that is your job or main Claude Code's job.

### Quick triage questions

Before picking an agent, answer these:

1. **Is it one-shot or repeated?** → repeated-games agents (Tier 5)
2. **Is information complete?** → Bayesian/signaling agents if not (Tier 6)
3. **Sequential or simultaneous?** → extensive-form vs normal-form agents (Tier 1)
4. **Cooperative (binding agreements possible) or non-cooperative?** → different tiers
5. **Do I need to find equilibria, design rules, or predict behavior?** → different tiers

---

## COMPLETE AGENT CATALOG (84 agents across 12 tiers)

---

## TIER 1 — CORE ANALYSTS (start here for any analysis)

Before solving, classify. Before classifying, elicit payoffs. Before equilibrium, specify strategies and information.

### `game-classifier`
GAME STRUCTURE CLASSIFICATION specialist. Use PROACTIVELY as the first step in any strategic analysis. MUST BE USED when the user presents a situation and wants to understand what game-theoretic structure it exhibits. Identifies the full multi-dimensional classification (cooperative vs non-cooperative, zero-sum vs positive-sum, symmetric vs asymmetric, simultaneous vs sequential, perfect vs imperfect information, complete vs incomplete information, finite vs infinite, one-shot vs repeated) and returns a structured fingerprint.

### `payoff-elicitor`
PAYOFF QUANTIFICATION specialist. Use PROACTIVELY when a real-world situation needs to be turned into a game with numerical payoffs. MUST BE USED when the user describes stakes in qualitative terms ("we'd lose face", "they might retaliate", "I'd feel bad") and wants a tractable game. Extracts cardinal or ordinal payoffs by interrogating preferences, risk attitudes, time discounting, and social/reputational costs.

### `payoff-matrix-builder`
NORMAL-FORM MATRIX CONSTRUCTION specialist. Use PROACTIVELY for any simultaneous (or strategically-simultaneous) game once players and payoffs are known. MUST BE USED before invoking equilibrium-finder agents on simultaneous games. Turns strategy sets + payoff functions into a clean normal-form matrix ready for Nash, dominance, and mixed-strategy analysis.

### `extensive-form-modeler`
EXTENSIVE-FORM GAME TREE specialist. Use PROACTIVELY for sequential games, games with imperfect information, or multi-stage interactions. MUST BE USED before invoking backward-induction-solver or subgame-perfect-analyzer. Constructs a complete, properly labeled game tree with decision nodes, action labels, information sets, and terminal payoffs.

### `information-structure-mapper`
INFORMATION STRUCTURE ANALYSIS specialist. Use PROACTIVELY whenever a situation may involve asymmetric information, hidden types, hidden actions, or private knowledge. MUST BE USED for any Bayesian game, signaling game, or mechanism design problem. Maps who knows what, when, and what each player believes about others' knowledge — producing a complete epistemic structure.

### `strategy-space-enumerator`
STRATEGY SPACE ENUMERATION specialist. Use PROACTIVELY when it's unclear what actions each player actually has, or when the obvious action list is suspiciously small. MUST BE USED before any equilibrium analysis to confirm the strategy space is correctly specified. Expands "what could I/they do?" into an exhaustive, de-duplicated, mutually-exclusive action set per player.

---

## TIER 2 — EQUILIBRIUM FINDERS

Once a game is specified, these agents solve it.

### `nash-equilibrium-finder`
NASH EQUILIBRIUM specialist. Use PROACTIVELY for any finite non-cooperative game once the payoff matrix or extensive form is known. MUST BE USED to enumerate all pure-strategy Nash equilibria and (when relevant) flag the need for mixed-strategy calculation. Returns the complete NE set with verification and stability notes.

### `dominant-strategy-identifier`
DOMINANT AND DOMINATED STRATEGY specialist. Use PROACTIVELY as the first analytical pass on any normal-form game before invoking Nash-finders. MUST BE USED to detect strictly-dominant-strategy equilibria (the strongest form of prediction) and to simplify games via iterated elimination of dominated strategies.

### `mixed-strategy-calculator`
MIXED-STRATEGY NASH EQUILIBRIUM specialist. Use PROACTIVELY when no pure-strategy Nash equilibrium exists, or when the game has multiple pure NE and a mixed one is also wanted. MUST BE USED for zero-sum games like matching pennies, for chicken and battle-of-sexes mixed equilibria, and for any game where randomization is a credible strategy. Computes equilibrium mixing probabilities using the indifference condition.

### `subgame-perfect-analyzer`
SUBGAME-PERFECT EQUILIBRIUM specialist. Use PROACTIVELY for any sequential or extensive-form game where Nash equilibrium admits non-credible threats. MUST BE USED to identify and eliminate non-credible threats via backward induction, and to find the unique SPE in finite perfect-information games. Core tool for analyzing commitment, deterrence, and Stackelberg-style games.

### `bayesian-equilibrium-analyst`
BAYESIAN NASH AND PERFECT BAYESIAN EQUILIBRIUM specialist. Use PROACTIVELY for any game with incomplete information, private types, or hidden characteristics. MUST BE USED for auctions, signaling games, screening problems, and any situation where players know their own payoff relevant attribute but not others'. Finds Bayesian Nash equilibria, Perfect Bayesian equilibria, and sequential equilibria.

### `correlated-equilibrium-designer`
CORRELATED EQUILIBRIUM specialist. Use PROACTIVELY when Nash equilibrium yields bad outcomes but a public signal could coordinate players on a Pareto-superior strategy profile. MUST BE USED for coordination problems with multiple equilibria, traffic-light-style situations, and any scenario where a mediator or common signal is available. Designs signal distributions that Pareto-improve on Nash.

### `trembling-hand-refiner`
TREMBLING-HAND PERFECT EQUILIBRIUM specialist. Use PROACTIVELY when Nash equilibria include weakly-dominated strategies, or when you suspect some equilibria are sustained only by zero-probability events. MUST BE USED to prune equilibria that cannot survive small "trembles" — accidental deviations with tiny probability. Implements Selten's perfection refinement and Myerson's proper equilibrium.

---

## TIER 3 — COOPERATIVE GAME SPECIALISTS

When binding agreements are possible, the questions change: who forms coalitions, how is value allocated fairly, is the grand coalition stable.

### `shapley-value-calculator`
SHAPLEY VALUE FAIR-DIVISION specialist. Use PROACTIVELY for any cooperative game requiring a fair allocation of the coalition's value. MUST BE USED for profit-sharing in joint ventures, cost allocation across business units, airport landing-fee splitting, voting power analysis, and machine-learning feature attribution (SHAP). Computes each player's Shapley value via marginal contribution averaging.

### `core-stability-analyst`
COALITION CORE STABILITY specialist. Use PROACTIVELY for cooperative games to determine whether the grand coalition will hold together or fragment. MUST BE USED when assessing whether an alliance, joint venture, cartel, or treaty is stable against subgroup defection. Tests core non-emptiness (Bondareva-Shapley), computes the core when it exists, and identifies profitable sub-coalition defections when it does not.

### `coalition-formation-strategist`
COALITION FORMATION DYNAMICS specialist. Use PROACTIVELY for multi-player situations where not all players will end up in one grand coalition. MUST BE USED for legislative coalitions, merger & acquisition strategy, cartel composition, faction politics, and any n-player setting where sub-group structure matters. Predicts which coalitions will form using stability concepts, hedonic preferences, and network formation.

### `banzhaf-power-auditor`
BANZHAF POWER INDEX specialist. Use PROACTIVELY for weighted voting bodies, shareholder voting, EU Council, UN Security Council, boards of directors, and any situation where formal vote weights obscure actual decision power. MUST BE USED to compute each voter's probability of being the pivotal "swing" vote. Complements Shapley-Shubik index (which differs in weighting scheme).

### `nucleolus-calculator`
NUCLEOLUS ALLOCATION specialist. Use PROACTIVELY for cooperative games where you need a unique, always-existing allocation that minimizes maximum coalitional dissatisfaction. MUST BE USED as an alternative to Shapley value when the core is empty and you need a principled fair allocation. Computes the leximin nucleolus: minimize the maximum excess, then the second-maximum, and so on.

---

## TIER 4 — CLASSIC GAME PATTERN RECOGNIZERS

Most real situations match one of the classic games. These agents recognize the pattern and apply the matching solution.

### `prisoners-dilemma-detector`
PRISONER'S DILEMMA pattern recognition specialist. Use PROACTIVELY whenever mutual cooperation Pareto-dominates mutual defection but individual defection dominates. MUST BE USED for arms races, price wars, advertising spending, doping in sports, climate negotiation, overfishing, tax evasion, and any situation with social dilemma structure. Identifies PD payoff structure (T > R > P > S with 2R > T+S), predicts the dilemma, and prescribes mitigations.

### `stag-hunt-analyst`
STAG HUNT pattern recognition specialist. Use PROACTIVELY when cooperation yields the largest payoff but requires mutual trust, while defection provides a safe but smaller guaranteed payoff. MUST BE USED for startup co-founders, alliance trust-building, technology standards adoption, team commitment, and any situation with the "risk vs payoff dominance" tradeoff. Identifies Stag Hunt structure, analyzes the trust problem, and prescribes trust-building interventions.

### `chicken-brinksmanship-tactician`
CHICKEN / HAWK-DOVE brinksmanship specialist. Use PROACTIVELY for any standoff where both parties would rather "swerve" than collide, but each wants the other to swerve first. MUST BE USED for nuclear deterrence analysis, strikes / lockouts, political showdowns, Cuban Missile Crisis-style standoffs, and hostile takeover battles. Identifies Chicken structure, analyzes commitment credibility, and prescribes brinksmanship tactics.

### `battle-of-sexes-coordinator`
BATTLE OF SEXES coordination-with-conflict specialist. Use PROACTIVELY when both players want to coordinate but disagree on the coordination point. MUST BE USED for standards wars (VHS vs Betamax, USB-C adoption), merger integration decisions, meeting locations, joint project direction, and any situation where being together matters more than where. Identifies BoS structure, compares focal points, and designs coordination mechanisms.

### `ultimatum-bargainer`
ULTIMATUM GAME specialist. Use PROACTIVELY for take-it-or-leave-it negotiations, final-offer contract disputes, severance offers, acquisition price demands, and situations where one party has sole proposal power. MUST BE USED to analyze the gap between subgame-perfect prediction (offer the minimum) and behavioral reality (reject unfair offers). Identifies fairness thresholds, cultural norms, and strategic proposal levels.

### `public-goods-diagnostician`
PUBLIC GOODS and FREE-RIDER problem specialist. Use PROACTIVELY for any multi-player situation involving shared contribution to a common benefit — tax compliance, conservation, vaccination, public broadcasting funding, open-source projects, team effort. MUST BE USED to diagnose free-riding incentives, estimate contribution decay over time, and design punishment/reward mechanisms that sustain cooperation.

### `tragedy-commons-analyst`
TRAGEDY OF THE COMMONS specialist. Use PROACTIVELY for depletable common-pool resources — fisheries, aquifers, atmospheric emissions, antibiotic effectiveness, groundwater, overgrazing, server capacity. MUST BE USED to diagnose resource-collapse risk, compute carrying capacity thresholds, and identify Ostrom-style institutional solutions. Distinct from generic public goods — the resource itself can be destroyed.

### `matching-pennies-randomizer`
PURE ZERO-SUM RANDOMIZATION specialist. Use PROACTIVELY for situations of pure opposition where predictability kills you — tax audits, penalty kicks, pitcher-batter, hide-and-seek, surprise inspections, security screening. MUST BE USED when one player's gain is exactly another's loss, no pure NE exists, and randomization is the only rational strategy. Computes minimax-optimal mixed strategies.

### `centipede-game-analyst`
CENTIPEDE GAME and backward-induction-failure specialist. Use PROACTIVELY for sequential take-vs-pass situations where the pot grows each round but either player can terminate. MUST BE USED for escrow, investment rounds, trust-building dynamics, extended contract negotiations, and any situation where rational backward induction predicts immediate defection but real players cooperate for many rounds. Identifies centipede structure and analyzes the gap between BI prediction and empirical behavior.

### `trust-game-analyst`
TRUST GAME and reciprocity specialist. Use PROACTIVELY for sequential-move situations where one party must commit value before knowing if the other will reciprocate. MUST BE USED for venture capital investments, advance payments, hiring decisions, contractor relationships, diplomatic overtures, and any scenario where trust-sending precedes trust-returning. Analyzes trust-sending amounts and return probabilities using reciprocity + stake models.

---

## TIER 5 — DYNAMIC & REPEATED GAMES

When the same game is played repeatedly, cooperation becomes possible even among self-interested agents. These agents handle the time dimension.

### `backward-induction-solver`
BACKWARD INDUCTION specialist for finite sequential games of perfect information. Use PROACTIVELY as the primary solver for any finite-horizon extensive-form game with full observation. MUST BE USED for ultimatum games, Stackelberg models, finite centipedes, alternating-offers bargaining, and any situation solvable by "solve the end first, work backward." Returns SPE strategy + predicted play.

### `folk-theorem-applier`
FOLK THEOREM and infinitely-repeated games specialist. Use PROACTIVELY when players interact repeatedly with no fixed end and sufficient patience. MUST BE USED for ongoing business relationships, long-term alliances, sustained cartels, cooperative agreements without external enforcement, and any situation where "the shadow of the future" sustains cooperation. Computes minimum discount factor and identifies sustainable equilibria.

### `tit-for-tat-strategist`
TIT-FOR-TAT and iterated PD strategy specialist. Use PROACTIVELY for iterated prisoner's dilemma scenarios and repeated cooperative/competitive relationships. MUST BE USED to design concrete behavioral strategies (nice, retaliatory, forgiving, clear) for ongoing business, diplomatic, or personal relationships. Picks optimal strategy variant (TFT, generous TFT, tit-for-two-tats, Pavlov) based on noise level and opponent type.

### `reputation-game-modeler`
REPUTATION DYNAMICS specialist. Use PROACTIVELY for finite-horizon or short-term interactions where sustaining cooperation seems impossible via BI but reputation effects can save it. MUST BE USED for Kreps-Milgrom-Roberts-Wilson-style reputation models, CEO reputation effects, brand trust, diplomatic credibility. Identifies how uncertainty about player types sustains cooperation that would fail under complete information.

### `cooperation-emergence-analyst`
COOPERATION EMERGENCE specialist. Use PROACTIVELY to understand HOW cooperation arises in populations of self-interested agents without central enforcement. MUST BE USED for evolutionary analysis of cooperation, group-selection dynamics, norm emergence, virality of cooperative strategies, and designing conditions that foster emergence. Synthesizes kin selection, reciprocal altruism, group selection, and cultural evolution mechanisms.

### `stochastic-game-analyst`
STOCHASTIC and state-dependent dynamic games specialist. Use PROACTIVELY for games where payoffs depend on evolving state variables — inventory games, pursuit-evasion, market dynamics, bargaining with shifting BATNAs, Markov decision processes with strategic opponents. MUST BE USED for dynamic programming solutions to games and when stage-game payoffs are not fixed but depend on a state that changes based on actions.

---

## TIER 6 — INFORMATION & SIGNALING

Games where what players know (and know others know) is the key variable.

### `signaling-game-analyst`
SIGNALING GAMES specialist. Use PROACTIVELY when one party has private information and chooses a costly action to reveal (or conceal) it. MUST BE USED for Spence-style job market signaling, brand advertising as quality signal, peacock-tail biology, warranty as quality signal, tattoos / club initiations as commitment signals, and any sender-receiver with private type. Identifies separating, pooling, and hybrid equilibria.

### `cheap-talk-evaluator`
CHEAP TALK and costless communication specialist. Use PROACTIVELY to determine whether non-binding, costless pre-play talk will transmit information. MUST BE USED for press conferences, public announcements, negotiation openers, sales pitches, and any communication that carries no enforcement. Applies Crawford-Sobel model to identify information transmission limits based on interest alignment.

### `asymmetric-info-detective`
ASYMMETRIC INFORMATION specialist for adverse selection and moral hazard. Use PROACTIVELY for insurance markets, credit markets, labor contracting, principal-agent problems, used-car / lemon markets. MUST BE USED to diagnose whether the problem is hidden information (adverse selection) or hidden action (moral hazard), and to design contract solutions.

### `credibility-assessor`
CREDIBILITY assessment specialist for threats, promises, commitments, and claims. Use PROACTIVELY to evaluate whether opponent's threat/promise is backed by interest/capability or is cheap talk. MUST BE USED before reacting to any strategic announcement — deterrent threats, commitment to prices, promised rewards, exit threats. Evaluates credibility via incentive compatibility, capability, reputation, and binding mechanism.

### `bayesian-belief-updater`
BAYESIAN BELIEF UPDATING specialist. Use PROACTIVELY whenever new evidence / observations should change a probability assessment. MUST BE USED for integrating incoming data with prior beliefs, forecasting with updating, and interpreting actions as signals. Computes posterior distributions from priors + likelihoods, and applies the results to strategic decisions.

### `screening-mechanism-designer`
SCREENING MECHANISM DESIGN specialist. Use PROACTIVELY for the uninformed party (principal) who needs to design contracts or menus that induce informed agents to self-select. MUST BE USED for insurance menus, loan product design, tiered pricing, nonlinear contracts, second-degree price discrimination, and reverse engineering agent types via menu choices.

---

## TIER 7 — MECHANISM DESIGN (engineering the game itself)

Inverse game theory: given a desired outcome, design the rules.

### `mechanism-designer`
GENERAL MECHANISM DESIGN specialist. Use PROACTIVELY when the question is not "what will players do?" but "what rules will make players do what we want?" MUST BE USED for institutional design, platform rules, voting systems, resource allocation, tournament structure, incentive schemes, and any situation where rules can be engineered. Applies the revelation principle to reduce arbitrary mechanisms to direct truthful ones.

### `auction-strategist`
AUCTION THEORY and strategy specialist. Use PROACTIVELY for any auction — English, Dutch, first-price sealed, second-price sealed, all-pay, combinatorial. MUST BE USED for auction design, bidding strategy, collusion risk assessment, and revenue comparison across formats. Covers Vickrey/Revenue-Equivalence, winner's curse, and real-world complications.

### `vcg-architect`
VICKREY-CLARKE-GROVES mechanism specialist. Use PROACTIVELY for multi-item or multi-agent allocation where efficiency matters and truthful reporting must be dominant. MUST BE USED for combinatorial auctions, public project allocation, task assignment, and any setting requiring dominant-strategy IC + efficient allocation. Designs VCG mechanism and flags its limitations.

### `incentive-compatibility-auditor`
INCENTIVE COMPATIBILITY verification specialist. Use PROACTIVELY to audit any proposed mechanism, contract, or policy for whether agents have incentive to truthfully reveal preferences / behave as intended. MUST BE USED after mechanism design to verify DSIC, BIC, or Nash IC claims. Detects manipulation opportunities via strategic misreporting.

### `matching-market-designer`
STABLE MATCHING and market design specialist. Use PROACTIVELY for two-sided matching problems (workers-jobs, students-schools, doctors-hospitals, kidney exchange, dating apps). MUST BE USED for Gale-Shapley deferred acceptance, stable-matching analysis, strategy-proofness audits, and real-world market clearinghouse design. Channels Al Roth's market design principles.

### `revenue-equivalence-analyst`
REVENUE EQUIVALENCE theorem specialist. Use PROACTIVELY when comparing auction formats or considering when revenue ranking is sensitive to format choice. MUST BE USED to identify which standard auction formats yield identical expected revenue and when the equivalence breaks (asymmetric bidders, risk aversion, correlated values). Recommends format choice based on seller objectives and bidder characteristics.

---

## TIER 8 — EVOLUTIONARY & BEHAVIORAL

How real agents (biological or human) deviate from and converge to game-theoretic predictions.

### `evolutionary-strategy-analyst`
EVOLUTIONARY GAME THEORY specialist for population-level strategy dynamics. Use PROACTIVELY when analyzing how strategies spread in populations via imitation, learning, selection — biology, cultural evolution, market strategy adoption, norm emergence. MUST BE USED for replicator dynamics, ESS identification, and long-run strategy frequencies. Bridges individual rationality and population dynamics.

### `ess-detector`
EVOLUTIONARILY STABLE STRATEGY detection specialist. Use PROACTIVELY to determine whether a proposed strategy is evolutionarily stable — resistant to invasion by small mutant populations. MUST BE USED to identify which strategies survive long-run evolutionary pressure and when Nash equilibria fail the stricter ESS test. Computes invasion barriers and identifies invasion paths.

### `behavioral-bias-detector`
BEHAVIORAL GAME THEORY specialist. Use PROACTIVELY to anticipate where real human players will deviate from classical game-theoretic predictions. MUST BE USED before committing to strategies that rely on full rationality — fairness preferences, loss aversion, bounded reasoning depth, anchoring, framing effects. Flags likely deviations and recommends strategies robust to behavioral biases.

### `level-k-reasoning-profiler`
LEVEL-K and COGNITIVE HIERARCHY reasoning-depth specialist. Use PROACTIVELY to estimate how many strategic levels your opponents can reason through. MUST BE USED for p-beauty contests, pricing wars, centipede games, any strategic setting where "they think that I think that they think" matters. Profiles opponent's likely reasoning level and prescribes best-response accordingly.

### `quantal-response-modeler`
QUANTAL RESPONSE EQUILIBRIUM specialist for noisy rationality. Use PROACTIVELY when players make systematic but noisy strategic errors. MUST BE USED for predicting behavior in experimental conditions, modeling human strategic noise, calibrating strategies to imperfect opponents. Applies McKelvey-Palfrey QRE to compute equilibria with bounded precision.

### `fairness-preferences-analyst`
FAIRNESS and social preferences specialist. Use PROACTIVELY when outcomes depend not just on player's own payoff but on how outcomes compare across players. MUST BE USED for ultimatum rejection prediction, public goods contribution, trust game reciprocity, dictator game sharing, and any situation where inequity aversion or reciprocity matters. Applies Fehr-Schmidt and ERC models.

### `loss-aversion-analyst`
LOSS AVERSION and prospect-theory specialist. Use PROACTIVELY when outcomes are framed as gains or losses from a reference point — negotiation, threats of loss, status quo vs change, risk-taking decisions. MUST BE USED to predict behavior in scenarios where losses loom larger than gains (typically 2x). Applies Kahneman-Tversky prospect theory to strategic situations.

---

## TIER 9 — STRATEGIC ACTION

Concrete tactical playbooks: negotiation, brinkmanship, deterrence, commitment, bluffing.

### `negotiation-strategist`
NEGOTIATION and bargaining strategy specialist. Use PROACTIVELY for any structured or informal negotiation — contract terms, salary, M&A deal, settlement. MUST BE USED to identify BATNA, ZOPA, reservation values, anchor points, concession patterns, and Rubinstein-style alternating offers. Translates game-theoretic bargaining models into concrete negotiation tactics.

### `brinkmanship-tactician`
BRINKMANSHIP and edge-of-cliff strategy specialist. Use PROACTIVELY for high-stakes standoffs, government shutdowns, strike / lockout, credit default threats, and escalation scenarios where both sides walk toward catastrophe. MUST BE USED to design credible brinksmanship tactics, navigate escalation ladders, and identify off-ramps. Channels Schelling's brinksmanship theory.

### `deterrence-theorist`
DETERRENCE THEORY specialist for threat-based prevention. Use PROACTIVELY for nuclear strategy, security policy, legal sanctions, corporate retaliation policies, and contract enforcement. MUST BE USED to design credible threats that prevent unwanted actions without having to execute them. Applies Schelling's strategy-of-conflict framework to deterrence design.

### `commitment-device-engineer`
COMMITMENT DEVICE design specialist. Use PROACTIVELY when you need to credibly tie your own hands to strengthen a strategic position. MUST BE USED for pre-commitment in negotiation, public promises, self-control devices, and strategic moves where visible binding increases leverage. Engineers specific mechanisms that make future actions credible and irreversible.

### `focal-point-identifier`
SCHELLING FOCAL POINT specialist. Use PROACTIVELY for coordination problems with multiple equilibria where players must converge without communication. MUST BE USED for meeting locations, joint project defaults, tie-breaking in coordination, standard emergence, and any situation requiring "we both picked X without discussing it." Identifies salient features that tip coordination to specific equilibria.

### `coopetition-strategist`
COOPETITION (simultaneous competition + cooperation) specialist. Use PROACTIVELY for cases where firms, nations, or individuals must both compete and cooperate with the same parties. MUST BE USED for standards bodies, joint ventures with competitors, research consortiums, industry associations, and supply-chain relationships. Applies Brandenburger-Nalebuff framework to identify value-creation vs value-capture moves.

### `first-mover-analyst`
FIRST-MOVER ADVANTAGE / DISADVANTAGE specialist. Use PROACTIVELY when considering whether to move first or wait. MUST BE USED for Stackelberg competition, market entry timing, preemptive capacity investment, public commitments, and any sequential-move scenario. Evaluates when first-move commitment helps vs when waiting for information is better.

### `threat-credibility-assessor`
THREAT CREDIBILITY specialist for evaluating opponent's threats (offensive or defensive). Use PROACTIVELY when opponent has announced a threat and you need to evaluate whether to comply, call the bluff, or counter-threat. MUST BE USED before reacting to any threat — price war, lawsuit, walkout, retaliation. Audits capability, incentive, binding, and reputation.

### `bluff-and-deception-analyst`
BLUFFING, DECEPTION, and information concealment specialist. Use PROACTIVELY in poker-like games, negotiation posturing, strategic misdirection, and pitch situations. MUST BE USED to design optimal bluffing frequencies, detect opponent bluffs, and manage reveal/conceal trade-offs in games of private information.

### `war-of-attrition-analyst`
WAR OF ATTRITION specialist. Use PROACTIVELY when two (or more) parties bear ongoing costs until one drops out. MUST BE USED for strikes, siege warfare, patent battles, protracted lawsuits, long bidding contests, corporate acquisitions, and any scenario where whoever quits first loses. Computes expected duration, cost estimates, and exit strategy.

---

## TIER 10 — APPLIED DOMAINS

Game theory specialized to specific application areas.

### `geopolitical-game-analyst`
GEOPOLITICAL GAME THEORY specialist. Use PROACTIVELY for international relations analysis — wars, treaties, alliances, trade disputes, sanctions, nuclear posturing. MUST BE USED to model state behavior game-theoretically, applying Schelling, Bueno de Mesquita, and Turchin-Jiang frameworks. Integrates classical IR theories with game-theoretic structure.

### `business-strategy-gamifier`
BUSINESS STRATEGY through game theory specialist. Use PROACTIVELY for corporate strategy analysis — market entry, pricing, product positioning, M&A, partnerships, competitive response. MUST BE USED to translate business situations into game-theoretic structure and identify dominant strategies, mixed strategies, and coopetition opportunities in the business context.

### `market-competition-modeler`
OLIGOPOLY competition modeling specialist. Use PROACTIVELY for duopoly and oligopoly analysis — Cournot quantity competition, Bertrand price competition, Stackelberg leadership, differentiated products. MUST BE USED to model industry competition, compute market equilibria, and predict responses to cost shocks, entry, or capacity changes.

### `voting-strategy-analyst`
VOTING and collective-choice game theory specialist. Use PROACTIVELY for elections, legislative votes, committee decisions, shareholder votes, and any formal decision-making body. MUST BE USED to analyze strategic voting, sincere vs insincere preferences, Arrow's impossibility, median voter theorem, coalition formation in legislatures, and vote manipulation.

### `social-interaction-gamifier`
EVERYDAY SOCIAL INTERACTION game theory specialist. Use PROACTIVELY for personal, romantic, family, friendship, and workplace dynamics analyzed through game theory. MUST BE USED for dating, marriage, family conflict, office politics, friendship reciprocity, and any informal interaction with strategic structure. Applies repeated-game, reciprocity, and signaling frameworks to everyday life.

### `conflict-resolution-theorist`
CONFLICT RESOLUTION and peace-building specialist. Use PROACTIVELY for mediating disputes, designing peace agreements, labor-management settlements, lawsuits, international treaties. MUST BE USED to identify positive-sum solutions in apparently zero-sum conflicts and to design mechanisms that stabilize agreements after reached. Applies Fisher-Ury and game-theoretic frameworks.

---

## TIER 11 — CIVILIZATIONAL / HISTORICAL (Jiang frameworks)

Professor Jiang Xueqin's strategic frameworks from his @PredictiveHistory channel — the long-arc patterns that explain empires, succession, revolutions, and narrative warfare.

### `myth-making-strategist`
MYTH-MAKING and narrative-reality-construction specialist (Jiang framework). Use PROACTIVELY when strategic outcomes depend on constructing a compelling narrative about identity, legitimacy, or destiny — political campaigns, corporate branding, founder stories, national narratives, revolutionary movements. MUST BE USED when reality itself must be reshaped through story. Applies Julius Caesar's myth-making genius and Augustus Caesar's Aeneid playbook to contemporary situations.

### `father-son-dynastic-analyst`
FATHER-SON DYNASTIC dynamics specialist (Jiang framework). Use PROACTIVELY to analyze founder-successor, builder-expander, or first-second-generation transitions in any organization, empire, or institution. MUST BE USED when the founder's qualities differ sharply from what the successor needs, and when generational transition threatens continuity. Applies Jiang's Philip II / Alexander the Great pattern to contemporary succession analysis.

### `cohesion-discipline-devotion-auditor`
MILITARY / ORGANIZATIONAL STRENGTH specialist (Jiang framework). Use PROACTIVELY to assess relative strength between tribal vs imperial, startup vs incumbent, challenger vs hegemon. MUST BE USED when trying to understand why a weaker-looking force defeats a stronger one. Audits cohesion (unity), discipline (training), and devotion (purpose) — Jiang's three-factor model of military / organizational effectiveness.

### `power-transition-analyst`
POWER TRANSITION specialist for rise-and-fall dynamics (Jiang + Organski framework). Use PROACTIVELY when analyzing rising powers challenging incumbents — geopolitical shifts, industry disruption, generational leadership changes. MUST BE USED for Thucydides trap analysis, US-China dynamics, disruptor vs incumbent, and long-arc power transitions across decades. Integrates Jiang's civilizational patterns with formal power-transition theory.

### `elite-overproduction-diagnostician`
ELITE OVERPRODUCTION specialist (Turchin / Jiang framework). Use PROACTIVELY to diagnose social instability driven by too many ambitious elites competing for limited top positions. MUST BE USED for analyzing civil unrest, political polarization, corporate dysfunction, and institutional breakdown. Applies Peter Turchin's structural-demographic theory with Jiang's historical pattern recognition.

### `dialectic-tension-mapper`
HEGELIAN DIALECTIC and opposing-forces specialist (Jiang framework). Use PROACTIVELY to identify opposing tendencies in a society, organization, or situation whose interaction drives historical change. MUST BE USED when "both sides seem right" or when a conflict seems intractable but both positions contain truth. Maps thesis, antithesis, and potential syntheses.

### `legitimacy-crisis-analyst`
LEGITIMACY CRISIS specialist (Jiang framework). Use PROACTIVELY when an authority (government, CEO, institution, leader) faces eroding acceptance of its right to rule. MUST BE USED for succession disputes, revolutions, corporate governance crises, and credibility collapses. Applies Jiang's analysis of David's apology, Roman civil wars, Caesar's cult-of-personality problem.

### `diaspora-dynamics-analyst`
DIASPORA and exile-dynamics specialist (Jiang framework). Use PROACTIVELY to analyze why minorities, exiles, and displaced populations often achieve disproportionate influence, wealth, or religious fanaticism. MUST BE USED for immigrant-community dynamics, religious revival movements, refugee politics, and out-group-then-succeed patterns. Applies Jiang's Jewish-Diaspora analysis and broader minority-success patterns.

### `poor-conquers-rich-analyst`
POOR-CONQUERS-RICH pattern specialist (Jiang framework). Use PROACTIVELY for asymmetric competitions where the materially-weaker side defeats the materially-stronger. MUST BE USED for startup-vs-incumbent, Macedon-vs-Greek-cities, Mongol-vs-settled-empires, North-Korea-vs-South-Korea thought experiment, and any contest where hunger, unity, and obedience may beat wealth and technology.

### `propaganda-detector`
PROPAGANDA and narrative-engineering detection specialist (Jiang framework). Use PROACTIVELY to identify when messages, media, curricula, or cultural products are engineered to shape beliefs rather than inform. MUST BE USED when evaluating political speeches, campaign materials, corporate communications, national myths, and educational content. Applies Jiang's Aeneid-style analysis to contemporary propaganda detection.

---

## TIER 12 — META & INTEGRATION

Higher-order agents that work across multiple games or redesign the game itself.

### `meta-game-designer`
META-GAME DESIGN specialist — changing the game itself. Use PROACTIVELY when the equilibrium of the current game is unfavorable and the structure can be changed. MUST BE USED when game-theoretic analysis shows bad outcomes are "rational" under current rules — rather than play better, change the game. Designs rule changes, new players, altered payoffs, shifted information, and structural moves.

### `counterfactual-simulator`
COUNTERFACTUAL analysis specialist. Use PROACTIVELY to simulate "what if X had played differently" in past or present strategic situations. MUST BE USED for learning from past games, stress-testing current strategy, exploring decision tree alternatives. Traces alternate-play consequences through the game tree to reveal robustness / fragility of outcomes.

### `equilibrium-selector`
EQUILIBRIUM SELECTION specialist for games with multiple Nash equilibria. Use PROACTIVELY when nash-equilibrium-finder returns more than one equilibrium and you need to predict WHICH one emerges. MUST BE USED for coordination games, battle of the sexes, stag hunt, and situations where multiple equilibria coexist. Applies Harsanyi-Selten selection criteria, Schelling focal points, and empirical patterns.

### `game-tree-archaeologist`
GAME RECONSTRUCTION specialist — reverse-engineer the game from observed outcomes. Use PROACTIVELY when you only see what happened (outcomes, actions) and need to infer the underlying game structure, payoffs, and beliefs. MUST BE USED for analyzing historical events, business case studies, or competitors' strategic moves where the game structure must be deduced from behavior. Applies revealed-preference and structural-estimation logic.

### `common-knowledge-analyst`
COMMON KNOWLEDGE specialist. Use PROACTIVELY to analyze what is actually common knowledge vs merely known privately or mutually. MUST BE USED for coordination problems, revolutions and mass movements (requires common knowledge of discontent), market panics, and any situation where "everyone knowing" differs from "everyone knowing everyone knows." Applies Aumann's agreement theorem and common-knowledge generator analysis.

---

## DESIGN PHILOSOPHY

Every agent follows the 8-layer Sherlock architecture:

1. **Identity** — named persona anchoring the methodology
2. **Memory architecture** — structured recall for patterns
3. **Epistemology** — explicit reasoning method
4. **Axiomatic stance** — non-negotiable cardinal rule
5. **Bias prevention** — named biases + countermeasures
6. **Mental models** — 3-7 named frameworks
7. **Phased protocols** — procedure invoking the frameworks
8. **Self-verification + structured output**

Every agent is a **worker** — it does its narrow job with rigor and returns a structured verdict. It will not route to other agents, it will not coordinate a swarm, it will not pretend to orchestrate. That is your job, and main Claude Code's job.

---

## COMMON WORKFLOWS

### Starting from a real-world situation
1. `game-classifier` → what kind of game is this?
2. `payoff-elicitor` → what are the stakes numerically?
3. `strategy-space-enumerator` → what can each player do?
4. `information-structure-mapper` → who knows what?
5. Based on classifier output: pick Tier 2 equilibrium finder
6. Apply Tier 4/5/6 specialist if pattern matches

### For a business decision
1. `business-strategy-gamifier` → translate to game structure
2. Specialized tier (pricing / entry / M&A)
3. `commitment-device-engineer` / `first-mover-analyst` → tactics

### For a negotiation
1. `negotiation-strategist` → framework
2. `payoff-elicitor` → quantify
3. `credibility-assessor` / `threat-credibility-assessor` → assess opponent
4. `commitment-device-engineer` → bind yourself
5. `fairness-preferences-analyst` → behavioral realism

### For geopolitical / civilizational analysis
1. `geopolitical-game-analyst` → formal framework
2. `power-transition-analyst` → long-arc dynamics
3. `cohesion-discipline-devotion-auditor` → force comparison
4. `myth-making-strategist` / `propaganda-detector` → narrative layer
5. `elite-overproduction-diagnostician` → internal stress

### When current equilibrium is bad
1. Diagnose why via equilibrium-finder
2. `meta-game-designer` → can we change the game?
3. `mechanism-designer` → engineer new rules
4. `commitment-device-engineer` → make new rules binding

---

## THE UNIFYING INSIGHT

From the Prisoner's Dilemma to the Fall of Rome: **the structure of a strategic situation dictates the outcome more than its surface content**. Two firms in a price war, two nations in an arms race, and two brothers fighting over an inheritance are all playing structurally identical games. The arsenal's job is to expose the structure — then help you change it if you don't like what it predicts.

*"Game theory is a flashlight, not a crystal ball. It illuminates the structure of a situation; it rarely tells you the unique correct move in the messy real world. Use it to frame decisions, not to make them mechanically."*
