# Trading and asset analysis with `/gametheory`

End-to-end TUI walkthrough for using the game-theory pipeline as a trading and asset-analysis aid. The pipeline doesn't predict prices — it decomposes a strategic situation through ~7 specialist lenses (asymmetric information, auction dynamics, behavioural bias, signalling, mechanism design, bargaining, deception) and gives you a multi-perspective view that a single LLM monologue can't.

> **TUI parity.** Every `/gametheory <subcommand>` here has an equivalent `archon gametheory <subcommand>` shell form. Both forms drive the same persisted state. See [CLI and TUI Command Parity](real-world-evidence-engine.md#cli-and-tui-command-parity).
>
> **This is a reasoning aid, not a forecasting model.** LLM-based analysis has a training cutoff and no access to live market data unless you feed it via `--kb`. Don't trade off the output without your own data + risk discipline.

## Why the gametheory pipeline fits market problems

Markets are the canonical multi-agent strategic game. The Tier 1 fingerprint axes line up with market structure:

| Axis | What it picks up in markets |
|---|---|
| `information.symmetry` | Asymmetric — insiders, sell-side analysts, HFTs, retail all see different things |
| `information.public_signals` | Present — price, volume, filings, news, central-bank statements |
| `payoffs.zero_sum` | True intraday (someone's ask is your bid). False long-term (markets create value) |
| `repetition` | Repeated — every trading session, every earnings cycle |
| `actors.count` | Many — institutional + retail + market makers + algos |
| `actors.identity_known` | Pseudonymous — most flow is anonymous; you infer types from order book behaviour |
| `horizon` | Configurable — intraday vs swing vs long term |
| `payoffs.alignment` | Misaligned — your gain is sometimes the counterparty's loss |
| `moves.sequencing` | Both — markets are simultaneous (continuous order book) AND sequential (announcements → reactions) |
| `moves.commitment` | Mostly none — orders are easily cancelled; positions can be closed |

That fingerprint pulls in the market-relevant specialists automatically:

| Specialist | What it adds for trading |
|---|---|
| `asymmetric-info-detective` | Adverse selection, signal-jamming, insider flow detection, lemon's-market dynamics |
| `bayesian-belief-updater` | How the market updates on news; efficient-market dynamics; surprise vs expectation |
| `auction-strategist` | The limit order book IS a continuous double auction; first-price vs second-price logic for bids |
| `behavioral-bias-detector` | Momentum, herding, anchoring, recency bias, disposition effect |
| `bluff-and-deception-analyst` | Spoofing, layering, wash trading, fake-out moves |
| `cheap-talk-evaluator` | Earnings guidance, analyst forecasts, central-bank communication, CEO statements |
| `business-strategy-gamifier` | Competitive dynamics between issuers; capacity decisions affecting margins |
| `bayesian-equilibrium-analyst` | Incomplete-info pricing models — what's the equilibrium given uncertain types? |
| `auction-format-comparer` | Call auctions vs continuous trading vs dark pools vs lit markets |
| `coalition-stability-checker` | Activist investor dynamics, board takeover game theory |

Tier 11 (`--enable-tier11`) brings in civilizational/Jiang-frameworks specialists — overkill for tactical trading, useful for macro/geopolitical timeframes.

## Five concrete trading workflows

Each workflow shows the actual `/gametheory` command, the rough cost, and the kind of output you should expect. All assume you've authenticated with Anthropic (gametheory is gated to Anthropic on agentic surfaces — Codex is blocked per the v0.1.44 capability matrix).

### 1. Pre-trade position assessment

You're considering a meaningful position. You want a multi-lens decomposition before you commit.

```
> /gametheory run Should I take a 6-month long position in NVDA at current valuation given (a) the AI capex cycle showing late-stage signs, (b) China export controls expanding, (c) major holders quietly trimming over the last two quarters? Treat this as a sequential game where my entry signals to other holders. --budget 5 --style technical
[gametheory] queued run gt-XXXXXXXXXXXX
[gametheory] cost cap: $5.00, max-concurrent: 4, style: technical
[gametheory] use /gametheory status gt-XXXXXXXXXXXX to monitor
```

Expected specialists fired: `asymmetric-info-detective` (insider trim signal), `bayesian-belief-updater` (capex-cycle priors), `bluff-and-deception-analyst` (is the trim genuine or jawboning?), `behavioral-bias-detector` (your own anchoring on prior price), `business-strategy-gamifier` (competitive dynamics in semis).

**Cost**: $3-7 typical, $5 cap above protects you. Time: 2-5 min.

### 2. Post-event analysis — why did this move actually happen

A position moved in a way that surprises you. You want to decompose what the market knows that you didn't.

```
> /gametheory run Decompose what just happened to TSLA after the Q3 earnings beat: stock dropped 8% despite revenue beat by 3%. Who knew what before the print? What's the steady-state implication for institutional positioning? --budget 4 --style executive
```

Expected output: cheap-talk-evaluator unpacks the guidance vs. realised gap, asymmetric-info-detective flags the institutional positioning before the print, behavioral-bias-detector calls out the "buy-the-rumour-sell-the-news" pattern, bayesian-belief-updater explains why a 3% beat couldn't override the implicit higher expectation.

**Cost**: $2-5. Useful for trade journaling — link the run-id in your post-trade review.

### 3. Counterparty / market-maker analysis

You see suspicious order-book behaviour and want to model what's happening on the other side.

```
> /gametheory run Analyse the strategic dynamics around a thinly-traded mid-cap where I observe persistent layered bids 0.5% below the inside market that pull every time I cross. What is the market maker's information set, what are they probing for, and what should my execution strategy be? --budget 5 --style technical
```

Expected lenses: `auction-strategist` (limit order book mechanics), `bluff-and-deception-analyst` (layering pattern), `asymmetric-info-detective` (what does the puller know about your flow?), `bayesian-belief-updater` (their belief about your sophistication updates with each crossing). The output should give you concrete execution adjustments (iceberg orders, randomised timing, alternative venues).

**Cost**: $3-7.

### 4. Strategy / rule viability — will this edge survive

You've back-tested a rule and it works historically. Will it keep working when others adopt it?

```
> /gametheory run Evaluate the strategic durability of a momentum rule that buys after 3 consecutive up days on volume and exits on the first down day. If the rule becomes widely known and other traders run it, what is the steady-state outcome? Frame as a repeated game with adaptation. --budget 7 --style academic
```

Expected lenses: `bayesian-belief-updater` (how the market updates if the signal becomes consensus), `coalition-stability-checker` (does adoption equilibrium hold?), `auction-strategist` (does the rule create predictable order flow that gets front-run?), `behavioral-bias-detector` (does the rule exploit a real bias or just a sample artefact?).

**Cost**: $5-10. Higher because academic style produces longer specialist outputs.

### 5. Macro / central-bank reaction function

You want a structured view of how a multi-actor macro game will play out.

```
> /gametheory run Model the FOMC's reaction function as a strategic game between the Fed (committed to 2% inflation), labour markets (signalling via wage growth), and bond markets (priced rate path). If the Fed cuts in December, what does each player's optimal response say about the equilibrium 12 months out? --budget 7 --style academic --enable-tier11
```

`--enable-tier11` brings in the civilizational specialists — useful here because central-bank credibility games are explicitly multi-decade.

**Cost**: $5-10 base, can run higher with Tier 11.

## Grounding in real source material with `--kb`

The pipeline is much sharper if you feed it actual filings, sell-side research, central-bank statements, and earnings transcripts rather than letting it use only model priors.

```
> /docs ingest /path/to/nvda-10k-2024.pdf
> /docs ingest /path/to/nvda-q3-2025-earnings-call.txt
> /docs ingest /path/to/jpm-equity-research-semis-2025/
> /docs ingest /path/to/fomc-statements-2024-2025/
> /kb process --claims --entities --relations --contradictions
```

Then bind the gametheory run to the pack:

```
> /gametheory run Should I take a 6-month long position in NVDA... --kb default --budget 5
```

With `--kb` set, every specialist analysis grounds in the ingested chunks. The final report cites specific filings and research notes by chunk-id rather than hand-waving. The `kb process` step extracts claims/entities/relations into the knowledge base so the agent can later answer questions like "what was NVDA's data-centre revenue growth in 2024?" via `memory_recall` without needing the gametheory pipeline at all.

If you want one knowledge pack per ticker / sector / regime, use `--pack` at ingest time:

```
> /docs ingest --pack nvda /path/to/nvda-filings/
> /docs ingest --pack semis /path/to/semis-research/
> /docs ingest --pack macro-2026 /path/to/fomc-statements/
> /gametheory run "<question>" --kb nvda --budget 5
```

## Workflow for systematic use

If you want to use this pipeline as a routine part of your trading process:

1. **Maintain ingested KB packs per scope** — one pack per ticker, sector, or macro regime. Re-ingest quarterly when 10-Qs/10-Ks land.

2. **Always classify-only first** (~$0.01) before paying for specialists:
   ```
   > /gametheory classify-only Your situation framing here
   ```
   Then `/gametheory inspect-fingerprint <id>` and `/gametheory inspect-routing <id>` to confirm the right specialists will fire. If the fingerprint axes are wrong, refine the prompt — cheap to iterate.

3. **Run the full pipeline only after fingerprint validates** — saves money on misframed questions.

4. **Persist the run-id in your trade journal** — link it next to the position. Later use `/gametheory show <id>` as part of post-trade review.

5. **Run `/gametheory replay <id> --rerun-specialist <key>`** if you disagree with one specialist's reasoning — you only re-pay for that specialist, not the whole pipeline.

6. **Use `--style executive`** for board / IC review (board-ready summaries), `--style technical` for trade-floor decisions (mechanism + assumptions), `--style academic` for strategy R&D (theory-heavy, longer).

## Caveats — read these before trading off the output

- **No live market data.** The model has a training cutoff. Real-time prices, intraday news, and post-cutoff filings must come in via `--kb` or the analysis is operating on stale priors.
- **No price prediction.** The pipeline tells you how to think about a situation, not where the price will go. Use the multi-lens decomposition to find blind spots in your own thesis, not to outsource judgement.
- **LLM cost ≠ trading edge.** Spending $7 on a gametheory run to size a $500 position is dumb. Match the spend to the position size and the time horizon.
- **Anthropic-only on agentic surfaces.** Per the v0.1.44 capability matrix, gametheory blocks Codex. If you've configured `[llm].provider = "openai-codex"` for general TUI use, you'll need to switch back for gametheory runs.
- **Provenance is your friend.** Always read the citations. If a specialist makes a numerical claim and the citation chunk doesn't actually support it, the claim is hallucinated. `--kb` + `archon completion verify <run-id>` is the way to catch this.
- **Replay is cheaper than re-running** — favour `replay --rerun-specialist` over starting fresh.

## Optional: combine with `/archon-research` for deep due diligence

For longer-horizon investment theses (not tactical trades), the research pipeline produces a structured PhD-style document with citations. Workflow:

1. Ingest the source pack: `/docs ingest /path/to/source-pack/` + `/kb process ...`
2. Run a research pass: `/archon-research "Strategic position assessment of NVDA's 2026-2030 datacenter capex cycle, with attention to TSMC supply, China export controls, and competitive response from AMD and custom silicon" --budget 15 --kb nvda`
3. Use the resulting research artefact as the situation prompt for `/gametheory run` — the gametheory run then operates over a much richer, fact-grounded framing

This is overkill for day trading. Useful for "should we initiate a position in this name and hold it for two years" — the research pipeline produces the thesis document; gametheory stress-tests the strategic assumptions.

## See also

- [Game-theory pipeline reference](gametheory-pipeline.md) — the underlying pipeline mechanics, full TUI walkthrough, replay/resume semantics
- [Game theory CLI reference](../gametheory.md) — full CLI/slash surface, Cozo source-of-truth tables
- [Research pipeline (`/archon-research`)](archon-research-pipeline.md) — sibling 46-agent pipeline for deeper PhD-style due diligence
- [Real-world Evidence Engine](real-world-evidence-engine.md) — composing docs + KB + gametheory + provenance into one workflow
- [Document intelligence (`/docs`)](../docs.md) — ingest filings, research notes, central-bank statements
- [Knowledge base (`/kb`)](../knowledge.md) — extract structured claims for `memory_recall` lookups during gametheory runs
