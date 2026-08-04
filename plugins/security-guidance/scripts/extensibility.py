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


def _split_unescaped_alternation(group: str) -> List[str]:
    """Split a flat group on unescaped alternation separators."""
    branches: List[str] = []
    start = 0
    escaped = False
    in_class = False
    for index, char in enumerate(group):
        if escaped:
            escaped = False
        elif char == "\\":
            escaped = True
        elif in_class:
            if char == "]":
                in_class = False
        elif char == "[":
            in_class = True
        elif char == "|":
            branches.append(group[start:index])
            start = index + 1
    branches.append(group[start:])
    return branches


def _repeated_groups(regex: str):
    """Yield flat repeated alternations and detect nested quantifiers in one pass."""
    stack: List[Tuple[int, bool, bool, bool]] = []
    escaped = False
    in_class = False
    for index, char in enumerate(regex):
        if escaped:
            escaped = False
            continue
        if char == "\\":
            escaped = True
            continue
        if in_class:
            if char == "]":
                in_class = False
            continue
        if char == "[":
            in_class = True
            continue
        if char == "(":
            if stack:
                start, _, has_alternation, has_quantifier = stack[-1]
                stack[-1] = (start, True, has_alternation, has_quantifier)
            stack.append((index + 1, False, False, False))
            continue
        if char in "+*" and stack:
            start, nested, has_alternation, _ = stack[-1]
            stack[-1] = (start, nested, has_alternation, True)
            continue
        if char == "|" and stack:
            start, nested, _, has_quantifier = stack[-1]
            stack[-1] = (start, nested, True, has_quantifier)
            continue
        if char == ")" and stack:
            start, nested, has_alternation, has_quantifier = stack.pop()
            quantifier = regex[index + 1] if index + 1 < len(regex) else ""
            if stack and quantifier in "+*":
                parent_start, parent_nested, parent_alternation, _ = stack[-1]
                stack[-1] = (parent_start, parent_nested, parent_alternation, True)
            if quantifier in "+*?" and has_quantifier:
                yield None, quantifier, False, True
            elif quantifier in "+*" and not nested and has_alternation:
                yield regex[start:index], quantifier, True, False


def _strip_group_prefix(group: str) -> Optional[str]:
    """Remove supported Python group prefixes, or reject an unknown prefix."""
    if not group.startswith("?"):
        return group
    if group.startswith("?:"):
        return group[2:]
    if group.startswith("?P<"):
        end = group.find(">", 3)
        return group[end + 1:] if end != -1 else None
    index = 1
    while index < len(group) and group[index] in "aiLmsux-":
        index += 1
    if index > 1 and index < len(group) and group[index] == ":":
        return group[index + 1:]
    return None


def _has_redos_structure(regex: str) -> bool:
    """Heuristic catastrophic-backtracking check using deterministic scanning."""
    for group, _quantifier, is_flat_alternation, has_nested_quantifier in _repeated_groups(regex):
        if has_nested_quantifier:
            return True
        if not is_flat_alternation:
            continue
        if group is None:
            return True
        stripped_group = _strip_group_prefix(group)
        if stripped_group is None:
            return True
        branches = _split_unescaped_alternation(stripped_group)
        if any(not branch for branch in branches):
            return True
        branches.sort()
        if any(right.startswith(left) for left, right in zip(branches, branches[1:])):
            return True
    return False
