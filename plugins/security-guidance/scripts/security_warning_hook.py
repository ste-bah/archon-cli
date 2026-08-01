#!/usr/bin/env python3
"""
PostToolUse security pattern warnings for the security-guidance plugin
(Archon port).

Called by Archon after Edit/Write/MultiEdit/NotebookEdit tool uses (see
hooks/settings.snippet.json). Reads the Archon event JSON from stdin,
checks the edited content and file path against the built-in security
patterns (plus any user-defined ``security-patterns.{yaml,json}`` rules),
and prints provenance-tagged warnings to stdout. Always exits 0 — warnings
are informational and never block the edit.

Environment switches:
  SECURITY_GUIDANCE_DISABLE=1  Kill switch — disables the plugin entirely
  ENABLE_SECURITY_REMINDER=0   Legacy kill switch (same effect)
  ENABLE_PATTERN_RULES=0       Disable the pattern warnings
"""

import json
import os
import random
import sys

# Windows consoles default to a legacy codepage (cp1252) that cannot
# encode the emoji commonly used in rule messages; force UTF-8 output.
for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding='utf-8', errors='replace')
    except (AttributeError, ValueError, OSError):
        pass

# Add the script directory to the Python path for sibling imports
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
if SCRIPT_DIR and SCRIPT_DIR not in sys.path:
    sys.path.insert(0, SCRIPT_DIR)

import re  # noqa: E402

import extensibility  # noqa: E402
from _base import PROVENANCE_TAG, debug_log  # noqa: E402
from patterns import SECURITY_PATTERNS  # noqa: E402
from session_state import cleanup_old_state_files, with_locked_state  # noqa: E402

# Master kill switch — honors ENABLE_SECURITY_REMINDER=0 (legacy) and
# SECURITY_GUIDANCE_DISABLE=1 (clearer name, no double negative).
SECURITY_GUIDANCE_DISABLED = (
    os.environ.get("SECURITY_GUIDANCE_DISABLE", "") == "1"
    or os.environ.get("ENABLE_SECURITY_REMINDER", "1") == "0"
)
ENABLE_PATTERN_RULES = os.environ.get("ENABLE_PATTERN_RULES", "1") != "0"


# =====================================================================
# Session-state helpers (per-session warning dedup)
# =====================================================================

def atomic_check_and_mark_warning(session_id, warning_key):
    """Return True (and record the key) iff this warning was not yet shown
    this session. Uses the locked state file so parallel hook invocations
    don't double-warn."""

    def _check(state):
        shown = state.setdefault("shown_warnings", [])
        if warning_key in shown:
            return False
        shown.append(warning_key)
        return True

    result = with_locked_state(session_id, _check)
    # On state errors, warn rather than stay silent (fail open).
    return True if result is None else result


# =====================================================================
# Pattern matching
# =====================================================================

def check_patterns(file_path, content):
    """Check if file path or content matches any security patterns. Returns ALL matches."""
    normalized_path = file_path.lstrip("/")
    matches = []

    for pattern in list(SECURITY_PATTERNS) + extensibility.user_patterns():
        # path_filter is a gate: when present, the rule only applies to
        # matching paths. Distinct from path_check, which is itself a
        # positive match condition (e.g. .github/workflows/).
        if "path_filter" in pattern:
            try:
                if not pattern["path_filter"](normalized_path):
                    continue
            except Exception:
                continue

        matched = False

        if "path_check" in pattern:
            try:
                if pattern["path_check"](normalized_path):
                    matched = True
            except Exception:
                pass

        if not matched and "substrings" in pattern and content:
            for substring in pattern["substrings"]:
                if substring in content:
                    matched = True
                    break

        if not matched and "regex" in pattern and content:
            try:
                if re.search(pattern["regex"], content):
                    matched = True
            except Exception:
                pass

        if matched:
            matches.append((pattern["ruleName"], pattern["reminder"]))

    return matches


def extract_content_from_input(tool_name, tool_args):
    """Extract content to check from tool arguments based on tool type."""
    if tool_name == "Write":
        return tool_args.get("content", "")
    elif tool_name == "Edit":
        return tool_args.get("new_string", "")
    elif tool_name == "MultiEdit":
        edits = tool_args.get("edits", [])
        if edits:
            return " ".join(edit.get("new_string", "") for edit in edits)
        return ""
    return ""


# =====================================================================
# Main
# =====================================================================

def main():
    """Main hook function."""
    debug_log(f"Hook called with args: {sys.argv}")

    if SECURITY_GUIDANCE_DISABLED:
        sys.exit(0)

    # Periodically clean up old state files (10% chance per run)
    if random.random() < 0.1:
        cleanup_old_state_files()

    # Read the Archon event JSON from stdin
    try:
        raw_input = sys.stdin.read()
        input_data = json.loads(raw_input)
    except json.JSONDecodeError as e:
        debug_log(f"JSON decode error: {e}")
        sys.exit(0)

    session_id = input_data.get("session_id", "default")
    tool_name = input_data.get("tool_name", "")
    # Archon events carry tool arguments in `tool_args`; fall back to
    # Claude-style `tool_input` defensively.
    tool_args = input_data.get("tool_args")
    if not isinstance(tool_args, dict):
        tool_args = input_data.get("tool_input")
    if not isinstance(tool_args, dict):
        tool_args = {}
    event = input_data.get("event", "") or input_data.get("hook_event_name", "")
    debug_log(f"Processing: event={event}, tool={tool_name}")

    # Load user-defined custom patterns once per invocation. Failures are
    # non-fatal (debug-logged) so a malformed config never prevents the
    # built-in checks from running.
    extensibility.load_for_session(input_data.get("cwd") or os.getcwd())

    # Pattern-based checks on file-editing tools
    if tool_name in ["Edit", "Write", "MultiEdit", "NotebookEdit"]:
        file_path = tool_args.get("file_path") or tool_args.get("notebook_path") or ""
        if not file_path:
            sys.exit(0)

        # Skip plan files
        plans_dir = os.path.expanduser("~/.archon/plans")
        if file_path.startswith(plans_dir):
            sys.exit(0)

        content = extract_content_from_input(tool_name, tool_args)

        all_guidance = []
        if ENABLE_PATTERN_RULES:
            pattern_matches = check_patterns(file_path, content)
            if pattern_matches:
                debug_log(f"Pattern matches for {file_path}: {[r for r, _ in pattern_matches]}")

            for rule_name, reminder in pattern_matches:
                warning_key = f"{file_path}-{rule_name}"
                if atomic_check_and_mark_warning(session_id, warning_key):
                    all_guidance.append(reminder)

        if all_guidance:
            print(PROVENANCE_TAG + "\n\n" + "\n\n".join(all_guidance))

    sys.exit(0)


if __name__ == "__main__":
    main()
