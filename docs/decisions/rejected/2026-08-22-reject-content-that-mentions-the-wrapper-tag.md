# Rejected: reject referenced content that mentions the wrapper's closing tag

- **Status:** Rejected
- **Date:** 2026-08-22
- **Area:** cross-session references — `crates/archon-core/src/session_reference.rs`
- **Decided in:** [`954e29098`](https://github.com/ste-bah/archon-cli/commit/954e29098) — `feat(session): reference another session as quoted data, never as instruction` (#200 Phase 4)
- **Chosen instead:** escape every `<` and `>` in referenced content

## What was proposed

`/session-ref <id>` injects another session's transcript into the current turn. A
transcript is model output and tool results, so text inside it can be shaped like
an instruction aimed at whoever reads it next. The excerpt is therefore wrapped in
a nonce-tagged block behind a preamble stating that the block is data and that no
directive inside it is to be followed.

That wrapper only holds if the referenced content cannot emit the block's closing
tag and continue outside it. The proposal was to detect and refuse: scan the
excerpt for the closing tag, and fail the reference if it appears.

## Why it was turned down

Refusal makes the referenced session the one that decides whether it can be
referenced.

Any session that wants to become unreferenceable only has to say the tag's name
once — in a code fence, in a quoted error message, in a passing discussion of this
very mechanism. That is not a hypothetical: a session that debugs the wrapper
necessarily contains the tag, and would permanently exclude itself from the
feature designed to read it. The check hands a denial-of-reference switch to
untrusted content, which is the same authority the wrapper exists to withhold.

A detector is also a matching problem, and matching problems are where escapes
live. It has to decide about the exact tag, about a guessed nonce, about a
borrowed `<hook-context>` from the neighbouring injection path, about whitespace
and case variants. Each of those is a rule that can be one character off.

## What was done instead

Every `<` and `>` in referenced content becomes `&lt;` / `&gt;`. The transcript
cannot emit a raw angle bracket, so it cannot reconstitute a closing tag *of any
shape* — not this snapshot's, not a guessed one, not a borrowed one. There is
nothing to detect and nothing to get wrong, because the character the attack needs
is not reachable.

Two supporting choices follow from it:

- The nonce stays, as a second layer rather than the only one.
- The preamble deliberately does **not** quote the closing tag. "There is exactly
  one closing tag in the block" is the property the wrapper is defended on, and
  printing a second copy of it inside the block would give that property away for
  the sake of a more readable sentence.

## What would change this

Nothing about escaping is load-bearing on the transcript being plain text. If a
future reference source must preserve literal markup for the model to read —
rendering an HTML artefact into a turn, say — escaping stops being free and the
trade-off is genuinely open again. Note that even then the answer is a different
containment (a separate channel, a structured content block), not detection:
the objection above is to letting content veto its own referenceability, and that
objection survives the change in content type.

## See also

- [`docs/operations/session-management.md`](../../operations/session-management.md)
  — the user-facing `/session-ref` surface.
- [`docs/postmortem/README.md`](../../postmortem/README.md) — the incident
  writeups that motivated writing this class of reasoning down.
