#!/usr/bin/env python3
"""UserPromptSubmit hook executor for the hookify plugin (Archon port).

This script is called by Archon when the user submits a prompt.
It reads .archon/hookify/*.local.md rule files and evaluates rules.

Output protocol (Archon): prompt rules are informational - matched rule
messages print to stdout (warn) or stderr (block) and the script exits
0. Errors never block; they log to stderr and exit 0.
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
    """Main entry point for UserPromptSubmit hook."""
    try:
        # Read the Archon event JSON from stdin
        input_data = json.load(sys.stdin)

        # Load user prompt rules
        rules = load_rules(event='prompt')

        # Evaluate rules
        engine = RuleEngine()
        result = engine.evaluate_rules(rules, input_data)

        if result.get('action') == 'block':
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
