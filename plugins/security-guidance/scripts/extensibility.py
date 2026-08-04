"""Project-specific extensibility for the security-guidance plugin (Archon port).

One extensibility point, additive only:

``security-patterns.{yaml,json}`` — custom regex/substring rules merged
with the built-in PostToolUse pattern warnings. No LLM call; pure regex.

Discovery, in precedence order (matching ARCHON.md / settings.json):
  - ``~/.archon/<name>``                  (user)
  - ``<cwd>/.archon/<name>``              (project, committed)
  - ``<cwd>/.archon/<name>.local.<ext>``  (project local, gitignored)

Trust model:
  - Custom pattern reminders go into the same provenance-tagged block as the
    built-in ones. Reminder length is capped.
  - Custom regexes are validated at load for catastrophic-backtracking
    structure and skipped (with a debug log) if they look ReDoS-prone.
  - Built-in patterns cannot be disabled. ``ENABLE_PATTERN_RULES=0`` disables
    all pattern checks; there is no per-rule kill switch.

Note: the upstream Claude Code plugin also loaded a
``claude-security-guidance.md`` policy file into its LLM review prompts.
The LLM review layers are not part of this Archon port, so that file is
not read here.
"""

import fnmatch
import json
import os
import re
from typing import Any, Dict, List, Optional, Tuple

from _base import debug_log

# ── caps ─────────────────────────────────────────────────────────────────────

PATTERN_MAX_RULES = 50
PATTERN_REMINDER_MAX_BYTES = 1024
# Custom patterns run in the PostToolUse hook. 4 KiB bounds config validation
# and the regex text handed to the matcher without constraining normal rules.
CUSTOM_REGEX_MAX_CHARS = 4096

PATTERNS_BASENAMES = ("security-patterns.yaml", "security-patterns.yml", "security-patterns.json")

# Module-level cache, loaded once per hook invocation by load_for_session().
_user_patterns: List[Dict[str, Any]] = []


# ── public API ───────────────────────────────────────────────────────────────


def load_for_session(cwd: Optional[str]) -> None:
    """Load project-specific patterns once per hook invocation.

    Called from the hook's main() before dispatching. Failures are non-fatal —
    a malformed config file produces a debug_log entry, never a crash.
    """
    global _user_patterns
    try:
        _user_patterns = _load_user_patterns(cwd)
    except Exception as e:
        debug_log(f"extensibility: failed to load security-patterns: {e}")
        _user_patterns = []


def user_patterns() -> List[Dict[str, Any]]:
    """User-supplied pattern rules in the same shape as SECURITY_PATTERNS."""
    return _user_patterns


# ── security-patterns.{yaml,json} ────────────────────────────────────────────


def _config_paths(cwd: Optional[str], basename: str) -> List[Tuple[str, str]]:
    """Existing config file paths, lowest precedence first."""
    paths = [("User", os.path.expanduser(os.path.join("~", ".archon", basename)))]
    if cwd:
        paths.append(("Project", os.path.join(cwd, ".archon", basename)))
        # security-patterns.local.yaml etc.
        stem, ext = os.path.splitext(basename)
        paths.append(("Project (local)", os.path.join(cwd, ".archon", f"{stem}.local{ext}")))
    return paths


def _load_user_patterns(cwd: Optional[str]) -> List[Dict[str, Any]]:
    rules: List[Dict[str, Any]] = []
    for label, path in _config_paths(cwd, "security-patterns"):
        # _config_paths returns an extensionless stem (e.g.
        # ".archon/security-patterns" or ".archon/security-patterns.local");
        # try each supported extension.
        for ext in (".yaml", ".yml", ".json"):
            candidate = path + ext
            data = _read_config(candidate)
            if data is None:
                continue
            for entry in (data or {}).get("patterns", []):
                rule = _validate_pattern(entry, source=label)
                if rule:
                    rules.append(rule)
            break  # found one extension; don't double-load .yaml AND .json
        if len(rules) >= PATTERN_MAX_RULES:
            break
    if len(rules) > PATTERN_MAX_RULES:
        debug_log(f"extensibility: {len(rules)} user patterns > cap {PATTERN_MAX_RULES}; truncating")
        rules = rules[:PATTERN_MAX_RULES]
    return rules


def _read_config(path: str) -> Optional[Dict[str, Any]]:
    """Read a YAML or JSON config file. Returns None on missing/malformed."""
    try:
        with open(path, encoding="utf-8") as f:
            raw = f.read()
    except OSError:
        return None
    if not raw.strip():
        return None
    if path.endswith(".json"):
        try:
            return json.loads(raw)
        except ValueError as e:
            debug_log(f"extensibility: skipping {path}: invalid JSON: {e}")
            return None
    # YAML: import lazily so the hook works without PyYAML (JSON still works).
    try:
        import yaml  # type: ignore
    except ImportError:
        debug_log(f"extensibility: skipping {path}: PyYAML not installed (use .json)")
        return None
    try:
        return yaml.safe_load(raw)
    except yaml.YAMLError as e:  # type: ignore
        debug_log(f"extensibility: skipping {path}: invalid YAML: {e}")
        return None


def _validate_pattern(entry: Any, source: str) -> Optional[Dict[str, Any]]:
    """Validate one user pattern entry. Returns a rule dict in the same shape
    as the built-in SECURITY_PATTERNS, or None if invalid (logged)."""
    if not isinstance(entry, dict):
        return None
    name = str(entry.get("rule_name", "")).strip()
    reminder = str(entry.get("reminder", "")).strip()
    if not name or not reminder:
        debug_log(f"extensibility: skipping pattern without rule_name/reminder: {entry!r:.80}")
        return None
    if len(reminder) > PATTERN_REMINDER_MAX_BYTES:
        reminder = reminder[:PATTERN_REMINDER_MAX_BYTES]
    regex = str(entry.get("regex", "")).strip()
    substrings = entry.get("substrings") or []
    if not isinstance(substrings, list) or not all(isinstance(s, str) for s in substrings):
        substrings = []
    if not regex and not substrings:
        debug_log(f"extensibility: skipping {name}: no regex or substrings")
        return None

    rule: Dict[str, Any] = {"ruleName": f"user:{name}", "reminder": reminder, "_source": source}

    if substrings:
        rule["substrings"] = substrings
    if regex:
        if len(regex) > CUSTOM_REGEX_MAX_CHARS:
            debug_log(f"extensibility: skipping {name}: regex exceeds {CUSTOM_REGEX_MAX_CHARS} characters")
            return None
        if _has_redos_structure(regex):
            debug_log(f"extensibility: skipping {name}: regex looks ReDoS-prone: {regex!r:.60}")
            return None
        try:
            rule["regex"] = regex
            re.compile(regex)
        except re.error as e:
            debug_log(f"extensibility: skipping {name}: invalid regex: {e}")
            return None

    paths = entry.get("paths") or []
    exclude = entry.get("exclude_paths") or []
    if paths or exclude:
        if not isinstance(paths, list) or not isinstance(exclude, list):
            debug_log(f"extensibility: skipping {name}: paths/exclude_paths must be lists")
            return None
        # Capture as defaults so the lambda doesn't share state across rules.
        rule["path_filter"] = (
            lambda p, _inc=tuple(paths), _exc=tuple(exclude): _glob_match(p, _inc, _exc)
        )
    return rule


def _glob_match(path: str, include: Tuple[str, ...], exclude: Tuple[str, ...]) -> bool:
    """Match a path against include/exclude globs. ``**`` matches any depth."""
    norm = path.replace(os.sep, "/")
    base = os.path.basename(norm)

    def _hit(globs: Tuple[str, ...]) -> bool:
        return any(
            fnmatch.fnmatch(norm, g) or fnmatch.fnmatch(base, g) for g in globs
        )
    if include and not _hit(include):
        return False
    if exclude and _hit(exclude):
        return False
    return True


# Catastrophic backtracking: nested quantifiers, overlapping alternations
# under repetition, and wildcard groups under repetition. Static check, not a
# proof — catches the common shapes that hang the hook on every edit.


def _group_quantifier(regex: str, closing_index: int) -> str:
    """Return a supported quantifier after a group with a fixed lookahead bound."""
    if closing_index + 1 >= len(regex):
        return ""
    quantifier = regex[closing_index + 1]
    if quantifier in "+*?":
        return quantifier
    if quantifier != "{":
        return ""
    # A repeat bound longer than 30 digits is invalid for practical Python regex
    # use; limiting this lookahead keeps the full scanner linear.
    end = regex.find("}", closing_index + 2, closing_index + 34)
    if end == -1:
        return ""
    bounds = regex[closing_index + 2:end].split(",", 1)
    if not bounds[0].isdecimal():
        return ""
    if len(bounds) == 1:
        return "{}" if int(bounds[0]) > 1 else ""
    if bounds[1] and not bounds[1].isdecimal():
        return ""
    if not bounds[1]:
        return "{}"
    return "{}" if int(bounds[1]) > 1 else ""


def _new_group(flag_unsafe: bool) -> Dict[str, Any]:
    """Build bounded state for one group while scanning an untrusted pattern."""
    return {
        "branches": [],
        "current": None,
        "direct_alternation": False,
        "nested": False,
        "child_alternation": False,
        "has_quantifier": False,
        "flag_unsafe": flag_unsafe,
        "prefix": "start",
    }


def _record_atom(group: Dict[str, Any], token: Optional[str]) -> None:
    """Record only the first atom of a branch; distinct literals prove separation."""
    if group["current"] is None:
        group["current"] = token


def _finish_branch(group: Dict[str, Any]) -> None:
    group["branches"].append(group["current"])
    group["current"] = None


def _direct_alternation_is_ambiguous(group: Dict[str, Any]) -> bool:
    """Require distinct literal branch prefixes before accepting a repeated alternation."""
    _finish_branch(group)
    branches = group["branches"]
    return any(branch is None for branch in branches) or len(set(branches)) != len(branches)


def _consume_group_prefix(group: Dict[str, Any], char: str) -> bool:
    """Consume a Python group prefix without treating it as a branch atom."""
    state = group["prefix"]
    if state == "start":
        if char == "?":
            group["prefix"] = "question"
            return True
        group["prefix"] = "body"
        return False
    if state == "question":
        if char == "P":
            group["prefix"] = "named"
            return True
        if char in ":=!":
            group["prefix"] = "body"
            return True
        if char == "<":
            group["prefix"] = "lookbehind"
            return True
        if char in "iLx":
            group["flag_unsafe"] = True
            return True
        if char in "aums-":
            return True
        group["prefix"] = "body"
        return False
    if state == "named":
        if char == ">":
            group["prefix"] = "body"
        return True
    if state == "lookbehind":
        if char in "=!":
            group["prefix"] = "body"
        return True
    return False


def _repeated_groups(regex: str):
    """Yield unsafe repeated groups using one bounded pass and stack summaries."""
    stack: List[Dict[str, Any]] = []
    global_flag_unsafe = False
    escaped = False
    in_class = False
    for index, char in enumerate(regex):
        if escaped:
            if stack:
                _record_atom(stack[-1], None)
            escaped = False
            continue
        if char == "\\":
            if stack:
                _record_atom(stack[-1], None)
            escaped = True
            continue
        if in_class:
            if char == "]":
                in_class = False
            continue
        if char == "[":
            if stack:
                _record_atom(stack[-1], None)
            in_class = True
            continue
        if char == "(":
            if index + 2 < len(regex) and regex[index + 1] == "?":
                flags_end = index + 2
                while flags_end < len(regex) and regex[flags_end] in "aiLmsux-":
                    flags_end += 1
                if flags_end < len(regex) and regex[flags_end] == ")":
                    global_flag_unsafe |= any(flag in "iLx" for flag in regex[index + 2:flags_end])
                    continue
            inherited_flag_unsafe = global_flag_unsafe or (stack and stack[-1]["flag_unsafe"])
            if stack:
                stack[-1]["nested"] = True
            stack.append(_new_group(bool(inherited_flag_unsafe)))
            continue
        if not stack:
            continue
        group = stack[-1]
        if _consume_group_prefix(group, char):
            continue
        if char == "|":
            group["direct_alternation"] = True
            _finish_branch(group)
            continue
        if char == ")":
            stack.pop()
            quantifier = _group_quantifier(regex, index)
            direct_ambiguous = (
                _direct_alternation_is_ambiguous(group) if group["direct_alternation"] else False
            )
            has_alternation = group["direct_alternation"] or group["child_alternation"]
            unsafe = quantifier in ("+", "*", "{}") and (
                group["has_quantifier"]
                or direct_ambiguous
                or (group["direct_alternation"] and group["flag_unsafe"])
                or (group["nested"] and has_alternation)
            )
            if unsafe:
                yield None, quantifier, False, True
            if stack:
                parent = stack[-1]
                parent["has_quantifier"] |= (
                    quantifier in ("+", "*", "{}") or group["has_quantifier"]
                )
                parent["child_alternation"] |= has_alternation
                _record_atom(parent, group["current"] if not has_alternation else None)
            continue
        if char in "+*{":
            group["has_quantifier"] = True
            continue
        if char in ".^$?":
            _record_atom(group, None)
            continue
        _record_atom(group, char)


def _has_redos_structure(regex: str) -> bool:
    """Heuristic catastrophic-backtracking check using a single stack scan."""
    return any(has_nested_quantifier for _, _, _, has_nested_quantifier in _repeated_groups(regex))
