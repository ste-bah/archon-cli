#!/usr/bin/env python3
"""Rule evaluation engine for the hookify plugin (Archon port).

Differences from the Claude Code original:
- Reads Archon hook event JSON: tool arguments live in `tool_args`
  (Claude used `tool_input`) and the event name lives in `event`
  (Claude used `hook_event_name`).
- Returns a simple {"action", "message"} result instead of Claude's
  JSON hook-output protocol (systemMessage / hookSpecificOutput /
  decision:block). The executor scripts translate "block" into exit
  code 2 (Archon's blocking convention) and "warn" into a printed
  message.
"""

import re
import sys
from functools import lru_cache
from typing import List, Dict, Any, Optional

# Import from sibling module (all hookify scripts live in one directory)
from config_loader import Rule, Condition


# Cache compiled regexes (max 128 patterns)
@lru_cache(maxsize=128)
def compile_regex(pattern: str):
    """Compile regex pattern with caching.

    Args:
        pattern: Regex pattern string

    Returns:
        Compiled regex pattern
    """
    return re.compile(pattern, re.IGNORECASE)


class RuleEngine:
    """Evaluates rules against hook input data."""

    def __init__(self):
        """Initialize rule engine."""
        # No instance cache needed - using global lru_cache
        pass

    def evaluate_rules(self, rules: List[Rule], input_data: Dict[str, Any]) -> Dict[str, Any]:
        """Evaluate all rules and return combined results.

        Checks all rules and accumulates matches. Blocking rules take priority
        over warning rules. All matching rule messages are combined.

        Args:
            rules: List of Rule objects to evaluate
            input_data: Hook input JSON (event, tool_name, tool_args, etc.)

        Returns:
            Result dict: {"action": "block"|"warn"|None, "message": str}.
            {"action": None, "message": ""} if no rules match.
        """
        blocking_rules = []
        warning_rules = []

        for rule in rules:
            if self._rule_matches(rule, input_data):
                if rule.action == 'block':
                    blocking_rules.append(rule)
                else:
                    warning_rules.append(rule)

        # If any blocking rules matched, block the operation
        if blocking_rules:
            messages = [f"**[{r.name}]**\n{r.message}" for r in blocking_rules]
            return {
                "action": "block",
                "message": "\n\n".join(messages),
            }

        # If only warnings, show them but allow operation
        if warning_rules:
            messages = [f"**[{r.name}]**\n{r.message}" for r in warning_rules]
            return {
                "action": "warn",
                "message": "\n\n".join(messages),
            }

        # No matches - allow operation
        return {"action": None, "message": ""}

    def _rule_matches(self, rule: Rule, input_data: Dict[str, Any]) -> bool:
        """Check if rule matches input data.

        Args:
            rule: Rule to evaluate
            input_data: Hook input data

        Returns:
            True if rule matches, False otherwise
        """
        # Extract tool information (Archon events carry tool_args; be
        # defensive and accept Claude-style tool_input as a fallback).
        tool_name = input_data.get('tool_name', '') or ''
        tool_args = input_data.get('tool_args')
        if not isinstance(tool_args, dict):
            tool_args = input_data.get('tool_input')
        if not isinstance(tool_args, dict):
            tool_args = {}

        # Check tool matcher if specified
        if rule.tool_matcher:
            if not self._matches_tool(rule.tool_matcher, tool_name):
                return False

        # If no conditions, don't match
        # (Rules must have at least one condition to be valid)
        if not rule.conditions:
            return False

        # All conditions must match
        for condition in rule.conditions:
            if not self._check_condition(condition, tool_name, tool_args, input_data):
                return False

        return True

    def _matches_tool(self, matcher: str, tool_name: str) -> bool:
        """Check if tool_name matches the matcher pattern.

        Args:
            matcher: Pattern like "Bash", "Edit|Write", "*"
            tool_name: Actual tool name

        Returns:
            True if matches
        """
        if matcher == '*':
            return True

        # Split on | for OR matching
        patterns = matcher.split('|')
        return tool_name in patterns

    def _check_condition(self, condition: Condition, tool_name: str,
                         tool_args: Dict[str, Any], input_data: Dict[str, Any] = None) -> bool:
        """Check if a single condition matches.

        Args:
            condition: Condition to check
            tool_name: Tool being used
            tool_args: Tool arguments dict (Archon `tool_args`)
            input_data: Full hook input data (for Stop events, etc.)

        Returns:
            True if condition matches
        """
        # Extract the field value to check
        field_value = self._extract_field(condition.field, tool_name, tool_args, input_data)
        if field_value is None:
            return False

        # Apply operator
        operator = condition.operator
        pattern = condition.pattern

        if operator == 'regex_match':
            return self._regex_match(pattern, field_value)
        elif operator == 'contains':
            return pattern in field_value
        elif operator == 'equals':
            return pattern == field_value
        elif operator == 'not_contains':
            return pattern not in field_value
        elif operator == 'starts_with':
            return field_value.startswith(pattern)
        elif operator == 'ends_with':
            return field_value.endswith(pattern)
        else:
            # Unknown operator
            return False

    def _extract_field(self, field: str, tool_name: str,
                       tool_args: Dict[str, Any], input_data: Dict[str, Any] = None) -> Optional[str]:
        """Extract field value from tool arguments or hook input data.

        Args:
            field: Field name like "command", "new_text", "file_path", "reason", "transcript"
            tool_name: Tool being used (may be empty for Stop events)
            tool_args: Tool arguments dict
            input_data: Full hook input (for accessing transcript_path, reason, etc.)

        Returns:
            Field value as string, or None if not found
        """
        # Direct tool_args fields
        if field in tool_args:
            value = tool_args[field]
            if isinstance(value, str):
                return value
            return str(value)

        # For Stop events and other non-tool events, check input_data
        if input_data:
            # Stop event specific fields
            if field == 'reason':
                return input_data.get('reason', '')
            elif field == 'transcript':
                # Read transcript file if a path is provided. Archon's Stop
                # event may not include transcript_path; in that case the
                # field resolves to None below and the condition does not
                # match (same as the original behavior for a missing path).
                transcript_path = input_data.get('transcript_path')
                if transcript_path:
                    try:
                        with open(transcript_path, 'r', encoding='utf-8') as f:
                            return f.read()
                    except FileNotFoundError:
                        print(f"Warning: Transcript file not found: {transcript_path}", file=sys.stderr)
                        return ''
                    except PermissionError:
                        print(f"Warning: Permission denied reading transcript: {transcript_path}", file=sys.stderr)
                        return ''
                    except (IOError, OSError) as e:
                        print(f"Warning: Error reading transcript {transcript_path}: {e}", file=sys.stderr)
                        return ''
                    except UnicodeDecodeError as e:
                        print(f"Warning: Encoding error in transcript {transcript_path}: {e}", file=sys.stderr)
                        return ''
            elif field == 'user_prompt':
                # For UserPromptSubmit events. Field naming differs across
                # hosts, so check the common variants defensively.
                value = (input_data.get('user_prompt')
                         or input_data.get('prompt')
                         or (tool_args.get('prompt') if tool_args else None))
                if value is not None:
                    return value if isinstance(value, str) else str(value)
                return ''

        # Handle special cases by tool type
        if tool_name == 'Bash':
            if field == 'command':
                return tool_args.get('command', '')

        elif tool_name in ['Write', 'Edit']:
            if field == 'content':
                # Write uses 'content', Edit has 'new_string'
                return tool_args.get('content') or tool_args.get('new_string', '')
            elif field == 'new_text' or field == 'new_string':
                return tool_args.get('new_string', '')
            elif field == 'old_text' or field == 'old_string':
                return tool_args.get('old_string', '')
            elif field == 'file_path':
                return tool_args.get('file_path', '')

        elif tool_name == 'MultiEdit':
            if field == 'file_path':
                return tool_args.get('file_path', '')
            elif field in ['new_text', 'content']:
                # Concatenate all edits
                edits = tool_args.get('edits', [])
                return ' '.join(e.get('new_string', '') for e in edits)

        return None

    def _regex_match(self, pattern: str, text: str) -> bool:
        """Check if pattern matches text using regex.

        Args:
            pattern: Regex pattern
            text: Text to match against

        Returns:
            True if pattern matches
        """
        try:
            # Use cached compiled regex (LRU cache with max 128 patterns)
            regex = compile_regex(pattern)
            return bool(regex.search(text))

        except re.error as e:
            print(f"Invalid regex pattern '{pattern}': {e}", file=sys.stderr)
            return False


# For testing
if __name__ == '__main__':
    from config_loader import Condition, Rule

    # Test rule evaluation
    rule = Rule(
        name="test-rm",
        enabled=True,
        event="bash",
        conditions=[
            Condition(field="command", operator="regex_match", pattern=r"rm\s+-rf")
        ],
        message="Dangerous rm command!"
    )

    engine = RuleEngine()

    # Test matching input (Archon event shape)
    test_input = {
        "event": "PreToolUse",
        "tool_name": "Bash",
        "tool_args": {
            "command": "rm -rf /tmp/test"
        }
    }

    result = engine.evaluate_rules([rule], test_input)
    print("Match result:", result)

    # Test non-matching input
    test_input2 = {
        "event": "PreToolUse",
        "tool_name": "Bash",
        "tool_args": {
            "command": "ls -la"
        }
    }

    result2 = engine.evaluate_rules([rule], test_input2)
    print("Non-match result:", result2)
