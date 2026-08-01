#!/usr/bin/env python3
"""PostToolUse hook executor for the hookify plugin (Archon port).

This script is called by Archon after a tool executes.
It reads .archon/hookify/*.local.md rule files and evaluates rules.

Output protocol (Archon): PostToolUse runs after the tool already
executed, so both warn and block rules print their message (warn to
stdout, block to stderr) and the script exits 0 - there is nothing left
to cancel. Errors never block; they log to stderr and exit 0.
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
    print(f"Hookify import error: {e}", file=sys.stderr)
    sys.exit(0)


def main():
    """Main entry point for PostToolUse hook."""
    try:
        # Read the Archon event JSON from stdin
        input_data = json.load(sys.stdin)

        # Determine event type based on tool
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
            # The tool already ran; surface the message prominently.
            print(result.get('message', ''), file=sys.stderr)
        elif result.get('action') == 'warn':
            print(result.get('message', ''))

        sys.exit(0)

    except SystemExit:
        raise
    except Exception as e:
        print(f"Hookify error: {e}", file=sys.stderr)
        # ALWAYS exit 0
        sys.exit(0)


if __name__ == '__main__':
    main()
