#!/usr/bin/env python3
"""Stop hook executor for the hookify plugin (Archon port).

This script is called by Archon when the agent wants to stop.
It reads .archon/hookify/*.local.md rule files and evaluates stop rules.

Output protocol (Archon):
- warn rules: message printed to stdout, exit 0 (session may stop)
- block rules: message printed to stderr, exit 2 (with `blocking: true`
  in .archon/settings.json, exit code 2 prevents the stop)
- errors: never block; log to stderr and exit 0

Note: rules with a `transcript` condition require the Stop event to
include a `transcript_path` field. If Archon's Stop event does not
provide one, those conditions simply never match (the rule stays inert).
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
    """Main entry point for Stop hook."""
    try:
        # Read the Archon event JSON from stdin
        input_data = json.load(sys.stdin)

        # Load stop rules
        rules = load_rules(event='stop')

        # Evaluate rules
        engine = RuleEngine()
        result = engine.evaluate_rules(rules, input_data)

        if result.get('action') == 'block':
            print(result.get('message', ''), file=sys.stderr)
            # Exit 2 prevents the stop when the hook entry sets
            # "blocking": true in .archon/settings.json.
            sys.exit(2)
        elif result.get('action') == 'warn':
            print(result.get('message', ''))

        sys.exit(0)

    except SystemExit:
        raise
    except Exception as e:
        # On any error, allow the stop
        print(f"Hookify error: {e}", file=sys.stderr)
        sys.exit(0)


if __name__ == '__main__':
    main()
