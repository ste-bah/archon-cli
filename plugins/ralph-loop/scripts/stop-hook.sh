#!/bin/bash

# Ralph Loop Stop Hook (Archon port)
# Prevents session exit when a ralph-loop is active
# Feeds the same prompt back to the agent to continue the loop
#
# Archon blocking protocol: this hook must be registered in
# .archon/settings.json with "blocking": true. Exit code 2 prevents the
# stop; the continuation prompt is printed to stderr. Exit code 0 allows
# the session to end. (The Claude Code original instead emitted
# {"decision": "block", "reason": ...} JSON on stdout — Archon does not
# interpret hook stdout JSON.)

set -euo pipefail

# Read hook input from stdin (Archon event JSON)
HOOK_INPUT=$(cat)

# Check if ralph-loop is active
RALPH_STATE_FILE=".archon/ralph-loop.local.md"

if [[ ! -f "$RALPH_STATE_FILE" ]]; then
  # No active loop - allow exit
  exit 0
fi

# Parse markdown frontmatter (YAML between ---) and extract values
FRONTMATTER=$(sed -n '/^---$/,/^---$/{ /^---$/d; p; }' "$RALPH_STATE_FILE")
ITERATION=$(echo "$FRONTMATTER" | grep '^iteration:' | sed 's/iteration: *//')
MAX_ITERATIONS=$(echo "$FRONTMATTER" | grep '^max_iterations:' | sed 's/max_iterations: *//')
# Extract completion_promise and strip surrounding quotes if present
COMPLETION_PROMISE=$(echo "$FRONTMATTER" | grep '^completion_promise:' | sed 's/completion_promise: *//' | sed 's/^"\(.*\)"$/\1/')

# Explicit completion signal: if the agent (or the user) set `active: false`
# in the state file, the loop is done. This is the completion path for
# Archon versions whose Stop event carries no transcript_path (see below).
ACTIVE=$(echo "$FRONTMATTER" | grep '^active:' | sed 's/active: *//' || true)
if [[ "$ACTIVE" == "false" ]]; then
  echo "✅ Ralph loop: state file marked active: false — loop complete."
  rm "$RALPH_STATE_FILE"
  exit 0
fi

# Session isolation: the state file is project-scoped, but the Stop hook
# fires in every Archon session in that project. If another session
# started the loop, this session must not block (or touch the state file).
# State files without session_id fall through (preserves the no-isolation
# behavior — also the default when Archon exports no session-id env var
# to the setup script).
STATE_SESSION=$(echo "$FRONTMATTER" | grep '^session_id:' | sed 's/session_id: *//' || true)
HOOK_SESSION=$(echo "$HOOK_INPUT" | jq -r '.session_id // ""' 2>/dev/null || echo "")
if [[ -n "$STATE_SESSION" ]] && [[ -n "$HOOK_SESSION" ]] && [[ "$STATE_SESSION" != "$HOOK_SESSION" ]]; then
  exit 0
fi

# Validate numeric fields before arithmetic operations
if [[ ! "$ITERATION" =~ ^[0-9]+$ ]]; then
  echo "⚠️  Ralph loop: State file corrupted" >&2
  echo "   File: $RALPH_STATE_FILE" >&2
  echo "   Problem: 'iteration' field is not a valid number (got: '$ITERATION')" >&2
  echo "" >&2
  echo "   This usually means the state file was manually edited or corrupted." >&2
  echo "   Ralph loop is stopping. Run /ralph-loop again to start fresh." >&2
  rm "$RALPH_STATE_FILE"
  exit 0
fi

if [[ ! "$MAX_ITERATIONS" =~ ^[0-9]+$ ]]; then
  echo "⚠️  Ralph loop: State file corrupted" >&2
  echo "   File: $RALPH_STATE_FILE" >&2
  echo "   Problem: 'max_iterations' field is not a valid number (got: '$MAX_ITERATIONS')" >&2
  echo "" >&2
  echo "   This usually means the state file was manually edited or corrupted." >&2
  echo "   Ralph loop is stopping. Run /ralph-loop again to start fresh." >&2
  rm "$RALPH_STATE_FILE"
  exit 0
fi

# Check if max iterations reached
if [[ $MAX_ITERATIONS -gt 0 ]] && [[ $ITERATION -ge $MAX_ITERATIONS ]]; then
  echo "🛑 Ralph loop: Max iterations ($MAX_ITERATIONS) reached."
  rm "$RALPH_STATE_FILE"
  exit 0
fi

# Get transcript path from hook input. Archon's documented Stop event does
# not include transcript_path; be defensive. If it's absent, skip promise
# detection (the `active: false` signal above and --max-iterations remain
# the exit paths) and continue the loop.
TRANSCRIPT_PATH=$(echo "$HOOK_INPUT" | jq -r '.transcript_path // ""' 2>/dev/null || echo "")
TRANSCRIPT_AVAILABLE=0

if [[ -n "$TRANSCRIPT_PATH" ]]; then
  if [[ ! -f "$TRANSCRIPT_PATH" ]]; then
    echo "⚠️  Ralph loop: Transcript file not found" >&2
    echo "   Expected: $TRANSCRIPT_PATH" >&2
    echo "   This is unusual and may indicate an Archon internal issue." >&2
    echo "   Ralph loop is stopping." >&2
    rm "$RALPH_STATE_FILE"
    exit 0
  fi
  TRANSCRIPT_AVAILABLE=1
fi

LAST_OUTPUT=""
if [[ $TRANSCRIPT_AVAILABLE -eq 1 ]]; then
  # Read last assistant message from transcript (JSONL format - one JSON per line)
  # First check if there are any assistant messages
  if ! grep -q '"role":"assistant"' "$TRANSCRIPT_PATH"; then
    echo "⚠️  Ralph loop: No assistant messages found in transcript" >&2
    echo "   Transcript: $TRANSCRIPT_PATH" >&2
    echo "   This is unusual and may indicate a transcript format issue" >&2
    echo "   Ralph loop is stopping." >&2
    rm "$RALPH_STATE_FILE"
    exit 0
  fi

  # Extract the most recent assistant text block.
  #
  # Each content block (text/tool_use/thinking) is written as its own JSONL
  # line, all with role=assistant. So slurp the last N assistant lines,
  # flatten to text blocks only, and take the last one.
  #
  # Capped at the last 100 assistant lines to keep jq's slurp input bounded
  # for long-running sessions.
  LAST_LINES=$(grep '"role":"assistant"' "$TRANSCRIPT_PATH" | tail -n 100)
  if [[ -z "$LAST_LINES" ]]; then
    echo "⚠️  Ralph loop: Failed to extract assistant messages" >&2
    echo "   Ralph loop is stopping." >&2
    rm "$RALPH_STATE_FILE"
    exit 0
  fi

  # Parse the recent lines and pull out the final text block.
  # `last // ""` yields empty string when no text blocks exist (e.g. a turn
  # that is all tool calls). That's fine: empty text means no <promise> tag,
  # so the loop simply continues.
  # (Briefly disable errexit so a jq failure can be caught by the $? check.)
  set +e
  LAST_OUTPUT=$(echo "$LAST_LINES" | jq -rs '
    map(.message.content[]? | select(.type == "text") | .text) | last // ""
  ' 2>&1)
  JQ_EXIT=$?
  set -e

  # Check if jq succeeded
  if [[ $JQ_EXIT -ne 0 ]]; then
    echo "⚠️  Ralph loop: Failed to parse assistant message JSON" >&2
    echo "   Error: $LAST_OUTPUT" >&2
    echo "   This may indicate a transcript format issue." >&2
    echo "   Ralph loop is stopping." >&2
    rm "$RALPH_STATE_FILE"
    exit 0
  fi
fi

# Check for completion promise (only if set and a transcript was readable)
if [[ $TRANSCRIPT_AVAILABLE -eq 1 ]] && [[ "$COMPLETION_PROMISE" != "null" ]] && [[ -n "$COMPLETION_PROMISE" ]]; then
  # Extract text from <promise> tags using Perl for multiline support
  # -0777 slurps entire input, s flag makes . match newlines
  # .*? is non-greedy (takes FIRST tag), whitespace normalized
  PROMISE_TEXT=$(echo "$LAST_OUTPUT" | perl -0777 -pe 's/.*?<promise>(.*?)<\/promise>.*/$1/s; s/^\s+|\s+$//g; s/\s+/ /g' 2>/dev/null || echo "")

  # Use = for literal string comparison (not pattern matching)
  # == in [[ ]] does glob pattern matching which breaks with *, ?, [ characters
  if [[ -n "$PROMISE_TEXT" ]] && [[ "$PROMISE_TEXT" = "$COMPLETION_PROMISE" ]]; then
    echo "✅ Ralph loop: Detected <promise>$COMPLETION_PROMISE</promise>"
    rm "$RALPH_STATE_FILE"
    exit 0
  fi
fi

# Not complete - continue loop with SAME PROMPT
NEXT_ITERATION=$((ITERATION + 1))

# Extract prompt (everything after the closing ---)
# Skip first --- line, skip until second --- line, then print everything after
# Use i>=2 instead of i==2 to handle --- in prompt content
PROMPT_TEXT=$(awk '/^---$/{i++; next} i>=2' "$RALPH_STATE_FILE")

if [[ -z "$PROMPT_TEXT" ]]; then
  echo "⚠️  Ralph loop: State file corrupted or incomplete" >&2
  echo "   File: $RALPH_STATE_FILE" >&2
  echo "   Problem: No prompt text found" >&2
  echo "" >&2
  echo "   This usually means:" >&2
  echo "     • State file was manually edited" >&2
  echo "     • File was corrupted during writing" >&2
  echo "" >&2
  echo "   Ralph loop is stopping. Run /ralph-loop again to start fresh." >&2
  rm "$RALPH_STATE_FILE"
  exit 0
fi

# Update iteration in frontmatter (portable across macOS and Linux)
# Create temp file, then atomically replace
TEMP_FILE="${RALPH_STATE_FILE}.tmp.$$"
sed "s/^iteration: .*/iteration: $NEXT_ITERATION/" "$RALPH_STATE_FILE" > "$TEMP_FILE"
mv "$TEMP_FILE" "$RALPH_STATE_FILE"

# Build system message with iteration count and completion promise info
if [[ "$COMPLETION_PROMISE" != "null" ]] && [[ -n "$COMPLETION_PROMISE" ]]; then
  SYSTEM_MSG="🔄 Ralph iteration $NEXT_ITERATION | To stop: output <promise>$COMPLETION_PROMISE</promise> (ONLY when statement is TRUE - do not lie to exit!)"
  if [[ $TRANSCRIPT_AVAILABLE -eq 0 ]]; then
    SYSTEM_MSG="$SYSTEM_MSG
Note: this Archon version provides no transcript to the stop hook, so the <promise> tag cannot be detected automatically. When (and ONLY when) the promise statement is genuinely TRUE, also set 'active: false' in $RALPH_STATE_FILE (edit the frontmatter line) in the same turn to end the loop. Do NOT set it while the statement is false."
  fi
else
  SYSTEM_MSG="🔄 Ralph iteration $NEXT_ITERATION | No completion promise set - loop runs infinitely (until --max-iterations, /cancel-ralph, or 'active: false' in $RALPH_STATE_FILE)"
fi

# Block the stop and feed the prompt back: stderr carries the continuation
# prompt; exit 2 (with "blocking": true in settings) prevents the stop.
{
  echo "$SYSTEM_MSG"
  echo ""
  echo "$PROMPT_TEXT"
} >&2

exit 2
