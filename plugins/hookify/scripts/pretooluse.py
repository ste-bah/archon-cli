#!/usr/bin/env python3
"""PreToolUse hook executor for the hookify plugin (Archon port).

This script is called by Archon before any tool executes.
It reads .archon/hookify/*.local.md rule files and evaluates rules.

Output protocol (Archon):
- warn rules: message printed to stdout, exit 0 (operation proceeds)
- block rules: message printed to stderr, exit 2 (with `blocking: true`
  in .archon/settings.json, exit code 2 cancels the tool call)
- errors: never block; log to stderr and exit 0
"""

import os
import sys
import json

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

try:
    from config_loader import load_rules
    from rule_engine import RuleEngine
except ImportError as e:
    # If imports fail, allow operation and log error
    print(f"Hookify import error: {e}", file=sys.stderr)
    sys.exit(0)


def main():
    """Main entry point for PreToolUse hook."""
    try:
        # Read the Archon event JSON from stdin
        input_data = json.load(sys.stdin)

        # Determine event type for filtering
        # For PreToolUse, we use tool_name to determine "bash" vs "file" event
        tool_name = input_data.get('tool_name', '')

        event = None
        if tool_name == 'Bash':
            event = 'bash'
        elif tool_name in ['Edit', 'Write', 'MultiEdit']:
            event = 'file'

        # Load rules
        rules = load_rules(event=event)

        # Evaluate rules
        engine = RuleEngine()
        result = engine.evaluate_rules(rules, input_data)

        if result.get('action') == 'block':
            print(result.get('message', ''), file=sys.stderr)
            # Exit 2 cancels the tool call when the hook entry sets
            # "blocking": true in .archon/settings.json.
            sys.exit(2)
        elif result.get('action') == 'warn':
            print(result.get('message', ''))

        sys.exit(0)

    except SystemExit:
        raise
    except Exception as e:
        # On any error, allow the operation and log
        print(f"Hookify error: {e}", file=sys.stderr)
        # Never block operations due to hook errors
        sys.exit(0)


if __name__ == '__main__':
    main()
