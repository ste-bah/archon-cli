# Amazon Bedrock

Archon talks to Bedrock through its own native client — the ConverseStream API,
signed with hand-rolled SigV4, in `crates/archon-llm/src/providers/bedrock.rs`.
It is not an OpenAI-compatible shim and it does not go through the Anthropic
Messages path. Converse has a different body shape, a different response
encoding, and a different prompt-cache mechanism, and every one of those
differences has its own failure mode that looks like success.

This page covers what you have to get right: the config, the credentials, the
model id, and how to prove the cache is actually working.

For the provider-neutral runtime contract Bedrock shares with Vertex, OpenAI and
the local providers, see [Cloud and local providers](cloud-and-local.md) and
[Provider runtime](runtime.md).

---

## Configuration

Bedrock is selected by `[llm].provider` and configured by the `[llm.bedrock]`
sub-table:

```toml
[llm]
provider = "bedrock"

[llm.bedrock]
region = "eu-west-2"
model_id = "eu.anthropic.claude-sonnet-4-6"
```

The section has exactly two keys. Both carry serde defaults, so an empty or
absent `[llm.bedrock]` is legal and silently means `us-east-1` plus a Sonnet 4.6
model id.

| Key | Type | Default | What it does |
| --- | --- | --- | --- |
| `region` | string | `"us-east-1"` | Names the runtime host (`bedrock-runtime.<region>.amazonaws.com`) **and** the SigV4 credential scope. This is the only region input on the request path — `AWS_REGION` is not read by the provider. |
| `model_id` | string | `"anthropic.claude-sonnet-4-6-v1:0"` | Goes straight into the request path. See [Model ids](#model-ids) for the spellings that resolve. |

Defined in `crates/archon-core/src/config/sections.rs` as `LlmBedrockConfig`.
See the [configuration reference](../reference/config.md) for how `[llm]` sits
alongside the rest of the file.

### What does *not* configure Bedrock

**`[api].default_model` does not choose the Bedrock model.** The provider dials
exactly one model: the endpoint URL is built from `model_id` regardless of what
an individual request asks for. A request carrying an alias, or a bare
`claude-sonnet-4-6` without the vendor prefix, still lands on `model_id` — which
is also why the cache strategy is resolved against `model_id` rather than the
request's spelling.

**There is no `[providers.bedrock]` section.** Bedrock's settings live only under
`[llm.bedrock]`. Anything keyed on a `[providers.*]` table — including the
context-window catalogue — reaches Bedrock through model-id normalisation
instead, not through a provider section.

**There is no API-key or base-URL key.** Credentials are resolved at call time
(below) and the endpoint is derived from `region`.

### When region or model id is missing

Both are non-empty strings by default, so this only happens if you explicitly
blank one. The two construction paths then differ:

- The main runtime logs `Bedrock selected but region/model_id missing` and falls
  back to Anthropic, recording the reason `bedrock_missing_region_or_model` as a
  provider runtime event.
- The Anthropic-free construction path (used where Anthropic credentials are not
  assumed) fails outright with the same message rather than falling back.

### Caching config

The prompt-cache switches are provider-neutral and live under `[context]`, not
under `[llm.bedrock]`. The two Bedrock-specific knobs are per-model overrides:

```toml
[context.prompt_cache_models."claude-sonnet-4-5"]
bedrock_min_tokens = 4096   # what AWS requires for this model
bedrock_ttl_1h = false      # five minutes only on Bedrock
```

Both are documented in full under
[Overriding the model table](../reference/prompt-caching.md#overriding-the-model-table).

---

## Credentials

`resolve_credentials` in `crates/archon-llm/src/providers/aws_auth.rs` tries
three sources, in order, on every request:

| Order | Source | Detail |
| --- | --- | --- |
| 1 | Environment | `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`, both non-empty. `AWS_SESSION_TOKEN` is picked up when present. |
| 2 | `~/.aws/credentials` | The **`[default]` profile only**. Keys read: `aws_access_key_id`, `aws_secret_access_key`, `aws_session_token`. |
| 3 | EC2 instance metadata (IMDSv2) | Session token, then the attached role, then its temporary credentials. Cached until shortly before expiry. |

If all three come up empty the request fails with:

```text
AWS credentials not found. Set AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY,
configure ~/.aws/credentials, or attach an EC2 instance profile
```

Two limits worth knowing before you debug the wrong thing. **Named profiles are
not supported**: the INI parser stops at the first section and only accepts
`[default]`, and `AWS_PROFILE` is never consulted. And **`AWS_REGION` is not read
on the request path** — only `[llm.bedrock].region` is.

The IMDS step is what lets an attached instance profile work with no static
secret on the box. It is deliberately cheap off EC2: a 1,000 ms timeout on the
link-local address, treated as absence rather than an error. Two environment
variables steer it — `AWS_EC2_METADATA_SERVICE_ENDPOINT` overrides the default
`http://169.254.169.254`, and `AWS_EC2_METADATA_DISABLED=true` skips the step
entirely.

### SigV4 signing

Requests are signed in-process rather than through the AWS SDK. Service
`bedrock`, method `POST`, signed headers `content-type;host;x-amz-date` — plus
`x-amz-security-token` whenever the credentials are temporary, in which case the
token is *covered by the signature* and also sent as its own header. AWS rejects
temporary credentials signed any other way.

The canonical URI is URI-encoded **twice**, per SigV4 outside S3: once for the
request line, again for the canonical request. This only shows up on dated,
versioned ids, where the colon in
`anthropic.claude-haiku-4-5-20251001-v1:0` becomes `%3A` in the URL and must
appear as `%253A` in the signature. Getting this wrong produces *"The request
signature we calculated does not match the signature you provided. Check your
AWS Secret Access Key"* — which reads as a credentials problem and is not one:
the same credentials work perfectly on any model id made of unreserved
characters.

### What `providers status` can and cannot see

```bash
archon providers status --provider bedrock
archon providers status --provider bedrock --json
archon providers doctor
```

Status judges Bedrock credentials on `AWS_ACCESS_KEY_ID` **or** the existence of
`~/.aws/credentials`. It does not probe IMDS — a link-local network call would
stall the command on every machine that is not EC2 — so a box carrying only an
instance profile reports `unknown-local` rather than `missing-credentials`. The
request path resolves it for real.

`archon chat --provider bedrock` is not a working smoke test. That path builds
providers from the flat descriptor config, which cannot express Bedrock's nested
`region` plus `model_id`, and returns `unknown or unavailable provider
'bedrock'`. Use the TUI or the live test below.

---

## Model ids

The id goes into the endpoint path verbatim:

```text
https://bedrock-runtime.<region>.amazonaws.com/model/<model_id>/converse-stream
```

Three spellings resolve, and Archon normalises all of them:

| Form | Example |
| --- | --- |
| Bare vendor id | `anthropic.claude-sonnet-4-6-v1:0` |
| Cross-region inference profile | `eu.anthropic.claude-sonnet-4-6` |
| Full inference-profile ARN | `arn:aws:bedrock:eu-west-2:…:inference-profile/eu.anthropic.claude-sonnet-4-6` |

Recognised geo prefixes are `us.`, `eu.`, `apac.` and `us-gov.`; an ARN is
reduced to the segment after the last `/` first. **Whether the id is recognised
as Anthropic decides real behaviour** — thinking, prompt caching and vision are
gated on it, while tool use, streaming and the system prompt are not. A bare
`starts_with("anthropic.")` check misses both the profile and ARN forms and
silently drops all three features, which is why the prefix stripping exists.

Current Claude models are generally reachable *only* through a cross-region
inference profile, and several are reachable only through the dated, versioned
id. The undecorated form returns `ValidationException` with nothing that hints
at the right answer. These five spellings are proven invocable and are the
defaults of the live cache test:

```text
eu.anthropic.claude-sonnet-4-6
eu.anthropic.claude-haiku-4-5-20251001-v1:0
eu.anthropic.claude-opus-4-6-v1
eu.anthropic.claude-sonnet-4-5-20250929-v1:0
eu.anthropic.claude-opus-4-5-20251101-v1:0
```

### The mapping tables

Three tables key off the model id, all by **substring, longest match wins**, so
one entry covers every spelling above:

| Table | File | Governs |
| --- | --- | --- |
| Cache parameters | `crates/archon-llm/src/cache_models_table.rs` | Minimum tokens, checkpoint limit, TTL support — with `bedrock_min_tokens` / `bedrock_ttl_1h` as the Bedrock-specific overrides |
| Pricing | `crates/archon-core/src/cost_table.rs` | Per-Mtok input/output, plus the regional multiplier |
| Context window | `crates/archon-llm/src/context_window.rs` | Reduces a decorated id to the bare `claude-sonnet-4-6` before catalogue lookup, stripping the ARN tail, geo prefix, vendor prefix and `-v1:0` suffix |

The context-window reduction is not cosmetic: an unknown window is what
auto-compaction is sized against.

**Regional endpoints cost 10% more.** `platform_multiplier` reads the geo prefix
off the id (`us.`, `eu.`, `apac.`, `ap.`, `jp.`, `au.`, `ca.`) and applies 1.1×
automatically, so state global prices in any `[context.model_pricing]` override
and let Archon add the premium. See
[Cost accounting](../reference/prompt-caching.md#cost-accounting) for the
verified `eu-west-2` Opus 5 rate card.

### Access agreements

A model id is not callable just because it exists. Bedrock model access is an
account-level entitlement granted by AWS, and until it is granted the invocation
returns `AccessDeniedException`.

The trap is that **the availability fields lie about it**. On the account used to
verify the v1.9.1 release, Opus 5, Opus 4.8, Opus 4.7, Sonnet 5 and Fable 5 all
returned `AccessDeniedException` with every availability field reading
`AVAILABLE`/`AUTHORIZED` and the inference profile `ACTIVE`.

Verify entitlement with an actual invocation, never with the availability
fields. The rate card and agreement state for a region are readable through
`aws bedrock list-foundation-model-agreement-offers`, which is where the pricing
figures in `cost_table.rs` were checked against.

---

## The Converse wire format

Converse is not the Messages API with different names. Four differences matter.

**1. Sections are separate arrays.** `system` is an array of `{"text": …}`,
`messages` carries Converse content blocks, tools go under
`toolConfig.tools[].toolSpec` with the JSON Schema nested at `inputSchema.json`,
and the output cap lives at `inferenceConfig.maxTokens`. `system` and
`toolConfig` are omitted entirely when empty.

**2. Content blocks are remapped, and unmapped blocks are dropped.**

| Archon block | Converse |
| --- | --- |
| `text` | `{"text": …}` |
| `tool_use` | `{"toolUse": {"toolUseId", "name", "input"}}` |
| `tool_result` | `{"toolResult": {"toolUseId", "content": [{"text": …}]}}` |
| `thinking` | `{"reasoningContent": {"reasoningText": {"text", "signature"}}}` |
| `redacted_thinking` | `{"reasoningContent": {"redactedContent": …}}` |
| anything else | dropped, with a debug log line |

A message left with **no** representable blocks is dropped from the request
rather than sent with `content: []`. Converse rejects an empty content field by
failing the whole *request*, not the message — so one such turn replays and
fails on every subsequent turn of the session. Round-tripping `thinking` also
preserves the signature the model needs to verify its own earlier reasoning.

**3. A cache checkpoint is its own array element**, not an attribute on a
neighbouring block:

```json
{ "cachePoint": { "type": "default" } }
```

with `"ttl": "1h"` added when the extended tier is requested and supported.
Checkpoints are placed in Bedrock's own evaluation order — `tools`, then
`system`, then `messages` — because the sections are chained and changing an
earlier one invalidates the caches behind it. The system checkpoint goes after
the *stable* head rather than after everything, since Archon appends per-turn
content to the system prompt. The full rationale, the per-model minimums on both
platform axes, and the `bedrock_min_tokens` / `bedrock_ttl_1h` overrides are in
[Prompt caching and cost](../reference/prompt-caching.md).

**4. The response is binary.** ConverseStream replies in
`application/vnd.amazon.eventstream` — a 12-byte prelude, a header block, the
payload and a CRC — with the event name in the `:event-type` **binary header**
rather than in the JSON. Scanning the frames as text finds the payloads but
never the wrapper key, which produces turns that complete in seconds with no
content and no error.

Usage arrives on the `metadata.usage` event as `inputTokens`,
`cacheWriteInputTokens` and `cacheReadInputTokens`. **Bedrock's `inputTokens`
excludes cached tokens** — the buckets are disjoint and Archon sums them, so
folding the cache counters back into the input figure double-counts every cached
token into both context pressure and cost. See
[the providers disagree about what `input_tokens` means](../reference/prompt-caching.md#the-providers-disagree-about-what-input_tokens-means).

---

## Verifying the cache

A checkpoint below the model's minimum is **discarded in silence**. The request
succeeds, the body looks correct, and the prompt is billed in full on every
turn. Nothing in the JSON Archon sends distinguishes a working cache from a
broken one — only what Bedrock reports back does. So verify against the
counters, in this order.

**1. Confirm credentials resolve at all.**

```bash
archon providers status --provider bedrock --json
```

**2. Run the live sweep.** It exercises the shipped path end to end — body
building, SigV4, eventstream decode — and asserts on Bedrock's own counters. It
is ignored by default because it costs money and needs credentials:

```bash
AWS_REGION=eu-west-2 \
  cargo test -p archon-llm --test bedrock_live_cache -- --ignored --nocapture
```

Set `ARCHON_BEDROCK_TEST_MODELS` to a comma-separated list of inference-profile
ids to test something other than the five defaults. The test sends a stable
~10k-token system prompt twice with *different* user messages, so a pass proves
the prefix was reused rather than a byte-for-byte request being replayed. It
fails when neither counter moves on turn 1 (the checkpoint fell under the
minimum) and when turn 2 reads nothing back (a write that is never read bills at
1.25× input, which is worse than not caching).

The shape of a healthy result, from the v1.9.1 verification run in `eu-west-2`:

```text
eu.anthropic.claude-sonnet-4-6: min_tokens=1024 turn1 write=10409 read=0  turn2 write=0 read=10403
```

The full five-model table is in the
[v1.9.1 release notes](../release-notes/v1.9.1.md).

**3. Watch a real session.** The TUI status line shows
`cache {creation}/{read}k`, per turn rather than cumulative. A healthy session
reads `21/0k` on the first turn and `0/18k` thereafter; `18/0k` every turn is a
cold write repeated, the most expensive shape there is. `/cost` gives the
session breakdown — see [slash commands](../reference/slash-commands.md).

**4. When the counters are ambiguous**, log the request body. Bedrock request
bodies are logged at debug level, which is the one place a missing or misplaced
`cachePoint` is visible:

```bash
ARCHON_DEBUG_LOG_DIR=/tmp/archon-debug \
  RUST_LOG=archon_llm=debug,archon_core=debug,info archon
```

The log is written by a non-blocking appender that flushes on exit, so the file
stays empty while Archon is running — exit before reading it.

---

## See also

- [Prompt caching and cost](../reference/prompt-caching.md)
- [Configuration reference](../reference/config.md)
- [CLI flags](../reference/cli-flags.md)
- [Cloud and local providers](cloud-and-local.md)
- [Provider runtime](runtime.md)
- [Auth profiles](auth-profiles.md)
