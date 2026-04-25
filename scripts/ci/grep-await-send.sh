#!/usr/bin/env bash
# Lint: fail if any code awaits on an AgentEvent producer `.send(...).await`.
# The AgentEvent channel is unbounded — send() is synchronous (returns
# Result<(), SendError<_>>), so `.await` on it is a bug either way:
#   * For tokio::sync::mpsc::UnboundedSender::send, `.await` won't compile
#     (send returns a value, not a future) — but refactors that accidentally
#     switch to bounded `mpsc::Sender` DO compile with `.await` and then
#     deadlock the TUI when the agent lock is held across it.
#
# Spec: 02-technical-spec.md line 1132 — "grep 'agent_event_tx\.send\(.*\)\.await' -> 0 hits"
#
# We expand the naive literal-name pattern to catch:
#   - renamed producers (agent_tx, event_tx, tx, producer, self.tx, ...)
#   - multi-line `.send(\n   payload\n).await`
#   - chained `.send(x).await?`, `.send(x).await.ok()`, etc.
#
# Scope is deliberately any send-then-await in source that declares
# `AgentEvent` — `rg --type-add` lets us use `-t rust` to skip generated or
# build artifacts. Test files are included: a `.send(...).await` inside a
# test counts as a latent bug waiting to graduate into prod.
#
# False-positive escape hatch (#230):
#   The two-pass narrowing (file mentions AgentEvent → match producer name)
#   is coarse — a file can mention AgentEvent in an unrelated import or
#   comment while the .send().await is on a channel of a DIFFERENT event
#   type. To exclude such call sites without disabling the whole lint,
#   add a comment marker on the line IMMEDIATELY PRECEDING the producer
#   reference:
#       // agent-event-tx-lint: ignore — channel holds OrchestratorEvent
#       let _ = event_tx.send(OrchestratorEvent::X).await;
#   Use sparingly. Prefer narrowing the producer name pattern below or
#   refactoring the channel type. The marker is matched as a substring
#   (case-sensitive). The lint reads the (lineno-1) of each hit and
#   suppresses the report when the marker is present.
#
#   Limitations of the marker filter:
#     - The marker MUST be on exactly the line directly above the producer
#       reference. A two-line gap will not be detected.
#     - Two single-line `.send(...).await` matches at adjacent linenos in
#       the same file are treated as a single multi-line match by the
#       continuation-detection heuristic; the second inherits the first's
#       suppression decision. Annotate both individually if both need
#       suppression and they happen to be on consecutive lines.
set -euo pipefail

ROOT="${TUI_GREP_ROOT:-crates/ src/}"

if ! command -v rg >/dev/null 2>&1; then
    echo "ERROR: ripgrep (rg) not found on PATH" >&2
    exit 2
fi

# Build ROOT arg as array so paths with spaces survive and directory-missing
# errors surface instead of being masked by `|| true`.
read -r -a ROOT_ARR <<<"$ROOT"
for r in "${ROOT_ARR[@]}"; do
    if [[ ! -e "$r" ]]; then
        echo "ERROR: grep-await-send ROOT '$r' does not exist" >&2
        exit 2
    fi
done

# Two-pass narrowing:
#   1) only consider Rust files that mention `AgentEvent` (type-gated scope).
#   2) inside those, flag any `.send(...)` followed by `.await` across lines.
#
# `rg -U` enables multiline matching; `--multiline-dotall` lets `.` cross
# newlines. We grep on the producer-style patterns seen in the codebase
# (tx, producer, sender, self.tx, etc.) rather than any `.send(...).await`
# to avoid false positives on unrelated futures APIs like `request.send().await`
# from reqwest. If a new producer variable name appears it must be added here.
PRODUCER_PATTERN='\b(agent_event_tx|agent_tx|event_tx|events_tx|tx|producer|sender|self\.tx|self\.sender|self\.event_tx|self\.events_tx)\b\s*\.send\s*\([^)]*?(?:\n[^)]*?)*\)\s*\.await'

# Restrict to files in rust type that also import/mention AgentEvent.
AGENT_EVENT_FILES=$(rg -l --type rust 'AgentEvent' "${ROOT_ARR[@]}" 2>&1) || {
    rc=$?
    # rc=1 from rg means "no matches", which is fine (nothing to lint).
    # Any other rc is a real error.
    if [[ $rc -ne 1 ]]; then
        echo "ERROR: rg scanning for AgentEvent failed (rc=$rc):" >&2
        echo "$AGENT_EVENT_FILES" >&2
        exit 2
    fi
    AGENT_EVENT_FILES=""
}

if [[ -z "$AGENT_EVENT_FILES" ]]; then
    echo "OK: no AgentEvent-mentioning files in scope"
    exit 0
fi

# Pipe file list to rg via -F (fixed strings; file names, not a pattern).
# `--with-filename` forces rg to prefix each row with the file path even when
# only a single file is passed (rg's default omits the path prefix in that
# case, which would break the marker filter's path:lineno parsing).
# shellcheck disable=SC2086
HITS=$(printf '%s\n' "$AGENT_EVENT_FILES" | xargs -r rg -n --no-heading --with-filename -U --multiline-dotall "$PRODUCER_PATTERN" 2>&1) || {
    rc=$?
    if [[ $rc -eq 1 ]]; then
        HITS=""
    else
        echo "ERROR: rg multiline scan failed (rc=$rc):" >&2
        echo "$HITS" >&2
        exit 2
    fi
}

# Filter out hits whose immediately-preceding line contains the
# `agent-event-tx-lint: ignore` marker (#230 escape hatch).
#
# rg multi-line output prints ONE row per source-line spanning the match
# (e.g., a 6-line `.send(...).await` produces 6 rows with consecutive
# linenos). The marker check MUST be applied to the FIRST row of each
# match (the producer reference), and continuation rows MUST inherit the
# suppression decision. We detect match starts by lineno discontinuity:
# any row whose path changed OR whose lineno is not previous+1 is a new
# match start.
FILTERED_HITS=""
if [[ -n "$HITS" ]]; then
    last_path=""
    last_lineno=-2
    suppress_current=false
    while IFS= read -r line; do
        [[ -z "$line" ]] && continue
        # rg -n format: <path>:<lineno>:<text>
        path="${line%%:*}"
        rest="${line#*:}"
        lineno="${rest%%:*}"
        # Pass through any line that does not match the path:lineno:text shape.
        if [[ ! "$lineno" =~ ^[0-9]+$ ]]; then
            FILTERED_HITS+="$line"$'\n'
            continue
        fi
        # Detect match-start vs continuation: a continuation has same path
        # AND lineno = last_lineno + 1.
        if [[ "$path" != "$last_path" || "$lineno" -ne "$((last_lineno + 1))" ]]; then
            # New match start — re-evaluate the marker on (lineno - 1).
            prev_line=""
            if [[ "$lineno" -gt 1 && -f "$path" ]]; then
                prev_line=$(sed -n "$((lineno - 1))p" "$path" 2>/dev/null || echo "")
            fi
            if [[ "$prev_line" == *"agent-event-tx-lint: ignore"* ]]; then
                suppress_current=true
            else
                suppress_current=false
            fi
        fi
        last_path="$path"
        last_lineno="$lineno"
        if [[ "$suppress_current" == true ]]; then
            continue
        fi
        FILTERED_HITS+="$line"$'\n'
    done <<<"$HITS"
    FILTERED_HITS="${FILTERED_HITS%$'\n'}"
fi

if [[ -n "$FILTERED_HITS" ]]; then
    echo "FAIL: producer .send(...).await detected — AgentEvent channel sends are sync"
    echo "$FILTERED_HITS"
    exit 1
fi

echo "OK: no producer .send(...).await in AgentEvent-scope files"
exit 0
