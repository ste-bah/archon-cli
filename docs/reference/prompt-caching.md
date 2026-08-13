# Prompt caching and cost

Every provider Archon talks to caches prompts. They disagree about almost
everything else: how you ask for it, whether you ask at all, how large the
prefix has to be before the request qualifies, how long it lives, and what it
costs. This page is the whole picture — what Archon does per provider, what the
knobs mean, and which mistakes are expensive.

## Why this is worth caring about

A cache **write** is not free. On Claude models it bills at **1.25×** plain
input, and **2×** at the one-hour retention. A cache **read** bills at **0.1×**.

That arithmetic has a sharp consequence:

> **A checkpoint that is never read back costs more than not caching at all.**

A five-minute checkpoint pays for itself after one read. A one-hour checkpoint
pays for itself after two. Anything less and you have paid a premium to store
something you then threw away — and because a failed cache produces no error,
nothing tells you.

The failure this whole area exists to prevent is a deployment that writes a
fresh checkpoint every single turn, never reads one back, and shows a *falling*
per-turn cost while the invoice climbs.

---

## What each provider does

| Provider | Mechanism | Archon emits |
| --- | --- | --- |
| Anthropic API (key or OAuth) | `cache_control` on a content block | breakpoints |
| Amazon Bedrock (Converse) | `{"cachePoint":{"type":"default"}}` as its own **array element** | checkpoints |
| Google Vertex (Claude) | `cache_control`, same as the Messages API | breakpoints |
| Google Vertex (Gemini) | — | nothing |
| OpenAI API, GPT-5.6 and later | `prompt_cache_breakpoint` on a content part | breakpoints |
| OpenAI API, GPT-5.5 and earlier | automatic on a stable prefix | nothing |
| Codex subscription | — (see below) | nothing |
| DeepSeek, Z.ai, Qwen, Moonshot, SiliconFlow | automatic | nothing |
| OpenRouter | unknowable | nothing |
| Local / Ollama | — | nothing |

Three of these deserve a note.

**"Automatic" is not "no caching."** Providers that cache a stable prefix
without being asked genuinely cache; there is simply nothing to annotate.
Archon models this as a distinct state, so cost reporting does not show those
requests as uncached, and so no marker is sent that would be an unsupported
field rather than a helpful hint. For these providers, **prompt ordering is the
only lever there is** — see below.

**OpenRouter reports nothing** rather than guessing. Which upstream serves a
given request is invisible to the caller, so neither "caches" nor "does not
cache" would be honest.

**The Codex subscription path is excluded deliberately.** Archon's stable
content there is the top-level `instructions` field, which OpenAI documents as
unable to carry a breakpoint. Moving it into a developer message to make room
for one would change the request shape being impersonated. It is a finding, not
an oversight.

---

## Minimums, and why they are a table rather than a rule

A breakpoint below the model's minimum is **silently discarded**. The request
succeeds and is billed in full. Exceeding the maximum of four breakpoints is a
visible 400. The two failure modes are not symmetric, which is why every default
here leans conservative — caching slightly late costs a little, caching
invisibly costs everything.

The minimums are **not monotonic in the version number**:

| Model | Anthropic API | Bedrock |
| --- | --- | --- |
| Opus 5 | 512 | 512 |
| Sonnet 5 | 1,024 | 1,024 |
| Sonnet 4.6 | 1,024 | 1,024 |
| Sonnet 4.5 | 1,024 | **4,096** |
| Opus 4.7 | 2,048 | 2,048 |
| Opus 4.6 | 4,096 | 4,096 |
| Opus 4.5 | 4,096 | 4,096 |
| Haiku 4.5 | 4,096 | 4,096 |
| Haiku 3.5 | 2,048 | 2,048 |
| GPT-5.6+ | 1,024 | — |

Opus 4.5 needs 4,096 while Opus 5 needs 512. Sonnet 4.6 needs 1,024 while its
same-generation siblings need 4,096. Any rule that infers a minimum from a
version number gets several of these wrong, so each is listed individually in
`crates/archon-llm/src/cache_models_table.rs`, and the operator can override
any of them from config.

Note the **platform axis**. Sonnet 4.5 caches from 1,024 tokens on Anthropic's
own endpoint and 4,096 on Bedrock. Each vendor is the authority on its own
service, so the table carries both figures and Archon resolves against whichever
stack it is actually talking to. Where it cannot tell — a gateway — it takes the
**strictest** reading across all candidate stacks: the higher minimum, and the
extended TTL only where every candidate allows one.

---

## Where the breakpoints go

Archon places up to two, and where they go is the point.

**The conversation breakpoint** goes on the last message. That message advances
every turn, so the growing history behind it stays cached.

**The stable-head breakpoint** goes at the end of the configured system blocks —
*not* at the end of the system prompt.

The distinction matters because Archon appends per-turn content to the system
prompt: recalled memories, the inner voice, the critical reminder, the turn's
guardrail requirements. All of it changes most turns. A checkpoint placed behind
that content is rewritten every turn and almost never read back, which by the
arithmetic above costs more than not caching. A checkpoint placed *in front* of
it keeps the tools and the static system prompt — the largest genuinely fixed
part of the request — hitting regardless of what churns behind them.

This costs one of the four available breakpoints and moves no content, so
nothing about what the model sees changes.

---

## Prompt ordering: the lever for everyone else

The stable-head breakpoint patches over the problem where a breakpoint exists.
Where one does not — GPT-5.5 and earlier, DeepSeek, and every other implicitly
caching provider — there is nothing to place, and the volatile blocks still sit
*in front of* the tools and the entire message history. Every provider caches by
common prefix and a prefix is a hit only up to its first changed byte, so one
different recalled memory invalidates the whole conversation behind it.

`prompt_cache_reorder` (default **on**) moves those per-turn blocks onto the last
user message instead — where Archon's `<system-reminder>` content already goes,
and which changes every turn regardless. What remains is one uninterrupted
prefix: stable system prompt, tools, full history.

It does change **where the model sees that text** — later, and closer to the
user's message — which is why it is a switch rather than an assumption. Turn it
off if you need those blocks to stay in the system prompt.

```toml
[context]
prompt_cache_reorder = true
```

Nothing is ever dropped to improve a cache hit. If there is no user message to
carry the blocks, they stay in the system prompt.

---

## Subagents

Subagents get the same treatment on every provider: the system checkpoint, the
conversation checkpoint, and the provider-specific wire format.

They do **not** get the stable-head split, and that is correct rather than a gap.
A subagent's system prompt is built from fields set once on the runner before the
run — the billing header, the system prompt, the caller's request blocks, the
critical reminder — and rebuilt identically every round. With nothing volatile at
the tail, the marker on the last block already covers the whole prefix and a
second breakpoint would spend one of four for nothing.

---

## Configuration

```toml
[context]
# Master switch.
prompt_cache = true

# "explicit" | "hybrid" — see below.
prompt_cache_mode = "explicit"

# "5m" | "1h". An hour is requested only where the model supports it.
prompt_cache_ttl = "5m"

# Spend a checkpoint on the message history. Turning this off leaves the tools
# and system checkpoints in place.
prompt_cache_conversation = true

# Move this turn's volatile system blocks behind the message history.
prompt_cache_reorder = true

# Which wire format the endpoint accepts, when it cannot be inferred.
# "auto" | "anthropic" | "bedrock" | "responses" | "automatic" | "off"
prompt_cache_strategy = "auto"
```

### `prompt_cache_strategy`, and why `auto` is not enough for a gateway

`auto` asks the provider, which recognises the official Anthropic Messages
endpoint and nothing else. A gateway URL tells Archon nothing about what is
behind it, so caching stays off and every request is billed in full — which is
safe against a gateway that would reject `cache_control`, and ruinous against
one that would have honoured it.

Only the operator knows which of the two their endpoint is. Declaring it says
**how to phrase a breakpoint**; it says nothing about how many tokens the service
behind it needs before honouring one, and that ranges from 512 to 4,096. The
limits still come from the model and the stack.

An unrecognised value falls back to the provider's own answer with a warning,
rather than silently picking a default that changes what a deployment spends.

### `prompt_cache_mode`

On Anthropic and Bedrock, `explicit` and `hybrid` both emit breakpoints;
`automatic` emits none.

On **OpenAI GPT-5.6+** the distinction has teeth. `prompt_cache_options: {"mode":
"explicit"}` switches OpenAI's own *implicit* breakpoints **off** so that only
yours participate. That is strictly better when the placement is right and
strictly worse when it is not. Archon therefore sends it only in `explicit` mode.
In `hybrid` the breakpoint is added alongside the implicit ones, so a misjudged
placement costs nothing rather than costing the caching that was already
happening for free.

Archon also derives a stable `prompt_cache_key` from the content of the prefix
being cached. OpenAI routes on a hash of the leading tokens combined with that
key, and documents that GPT-5.6 needs one set to match reliably at all. Deriving
it from the prefix gives it exactly the property it needs: two turns of the same
conversation share a key because their stable prefix is identical, and two
different agents get different keys because theirs are not.

### Overriding the model table

The built-in table is right until it is stale — a model released after the
binary, or a figure a vendor revised. One config edit corrects every provider,
without a release:

```toml
[context.prompt_cache_models."claude-sonnet-4-5"]
min_tokens = 1024          # Anthropic's own endpoint
max_checkpoints = 4
ttl_1h = true
bedrock_min_tokens = 4096  # what AWS requires for the same model
bedrock_ttl_1h = false     # five minutes only on Bedrock
```

Keys are matched as **substrings of the model id**, most specific wins, so one
entry covers `claude-sonnet-4-5`, `anthropic.claude-sonnet-4-5-…`,
`eu.anthropic.…` and the full inference-profile ARN.

The override is applied in `request_cache::resolve_strategy` — the single choke
point every request passes through — so it reaches both the `auto` path and the
declared-format path, on every provider.

---

## Cost accounting

Archon's cost estimates price four buckets separately: uncached input, output,
cache writes and cache reads. Base prices per model come from the vendors' rate
cards; the cache tiers are derived from them by the published multipliers.

| Operation | Multiplier on base input |
| --- | --- |
| Cache read (hit) | 0.1× |
| Cache write, 5-minute | 1.25× |
| Cache write, 1-hour | 2× |

These are fixed across every Claude model. DeepSeek's read multiplier is far
lower (roughly 0.008× on V4 Pro) and it bills no separate write tier.

**Regional endpoints carry a 10% premium.** Bedrock and Google Cloud both charge
more for a regional or multi-region endpoint than a global one, for Sonnet 4.5,
Haiku 4.5, Opus 4.5 and everything since. The endpoint type is spelled into the
model id — `eu.anthropic.…` against `global.anthropic.…` — so Archon applies it
automatically. Verified against the live Bedrock rate card for Opus 5 in
eu-west-2:

| Dimension | $/Mtok | Ratio to input |
| --- | --- | --- |
| input | 5.50 | 1× |
| output | 27.50 | — |
| cache read | 0.55 | 0.1× |
| cache write (5m) | 6.875 | 1.25× |
| cache write (1h) | 11.00 | 2× |

Against Anthropic's global figures of $5 / $25 for the same model — the 10%
regional premium, exactly.

### Adding or correcting a price

```toml
[context.model_pricing."gpt-5.6"]
input_per_mtok = 1.25
output_per_mtok = 10.0
```

`input_per_mtok` and `output_per_mtok` are required. The three cache multipliers
default to Claude's published ratios, so most entries need only two numbers.
State the **global** price; Archon applies the regional premium on top from the
model id.

For a provider whose cache is priced unusually:

```toml
[context.model_pricing."some-model-with-a-cheap-cache"]
input_per_mtok = 0.4
output_per_mtok = 0.8
cache_read_multiplier = 0.01
cache_write_multiplier = 1.0
cache_write_1h_multiplier = 1.0
```

### What is not modelled

The Batch API's 50% discount, fast mode's premium on Opus 5 and 4.8, and the
1.1× `inference_geo: "us"` multiplier. Each needs a request-level flag the cost
call sites do not carry. The regional premium *is* modelled, because it is
inferable from the model id alone.

The five-minute write tier is assumed unless the caller states otherwise. The
counters providers return do not distinguish the two tiers, and guessing at 2×
would overstate every deployment that never asked for an hour.

---

## Reading the counters

The TUI status line shows `cache {creation}/{read}k`. The segment is hidden
entirely when both are zero.

`18/0k` is **not** a cache hit. It is 18k written and 0k read — a cold write,
the most expensive kind of turn. What you want to see is the read side climbing
from the second turn on, and the per-turn cost falling while the conversation
grows.

The counters are **per turn, not cumulative**: each turn resets them before the
request goes out, so a healthy session reads `21/0k` on the first turn and
`0/18k` thereafter.

### The providers disagree about what `input_tokens` means

This is the single easiest thing to get wrong, because both conventions look
identical in a JSON body:

| Provider | `input_tokens` | Cached tokens are |
| --- | --- | --- |
| Anthropic, Bedrock | **excludes** cached | a separate bucket |
| OpenAI (Responses) | **includes** cached | a *subset* of the total |

Archon sums the buckets to get the context size, so it needs them disjoint.
Bedrock proves its convention plainly — `inputTokens: 3` on a 4,424-token
request whose prefix was served from cache.

Passing OpenAI's total through unchanged counted every cached token twice, and
the effects compounded in the wrong direction: measured on a real ~12k prompt
with 9,728 cached, archon reported **21k of context** and charged **$0.07**
where the correct figures were 10k and **$0.02**. The context figure is what
sizes auto-compaction, so it compacted early; and the cost double-charged the
cached tokens at the full input rate on top of the cache-read rate — which makes
a working cache look *more* expensive than no cache at all, the exact illusion
this whole area exists to prevent.

If you add a provider, check which convention its usage block follows before
trusting the counters.

### When the counters are not enough

`None` and `Some(0)` are different — "the service reported nothing" versus "the
service reported a miss" — and both render as a bare zero. To tell them apart,
and to see the request body that was actually sent:

```bash
ARCHON_DEBUG_LOG_DIR=/tmp/archon-debug RUST_LOG=archon_llm=debug,archon_core=debug,info archon
```

Two things to know about that log:

- It is written by a non-blocking appender that flushes when the process exits,
  so the file stays empty while archon is running. Exit before reading it.
- Scope the filter to the crates you need. `archon_llm=debug` alone leaves
  `archon_core` at `info`, which hides the accumulated per-turn usage.

---

## See also

- [Amazon Bedrock](../providers/bedrock.md)
- [Configuration reference](config.md)
- [OpenAI-compatible providers](../providers/openai-compatible.md)
