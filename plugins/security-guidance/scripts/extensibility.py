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


def _validated_regex(name: str, regex: str) -> Optional[str]:
    """Return a compiled-safe custom regex string, logging invalid config once."""
    if len(regex) > CUSTOM_REGEX_MAX_CHARS:
        debug_log(f"extensibility: skipping {name}: regex exceeds {CUSTOM_REGEX_MAX_CHARS} characters")
        return None
    if _has_redos_structure(regex):
        debug_log(f"extensibility: skipping {name}: regex looks ReDoS-prone: {regex!r:.60}")
        return None
    try:
        re.compile(regex)
    except (re.error, OverflowError, RecursionError) as exc:
        debug_log(f"extensibility: skipping {name}: invalid regex: {exc}")
        return None
    return regex


def _path_filter(entry: Dict[str, Any], name: str):
    """Return a validated optional path filter for a custom rule."""
    paths = entry.get("paths") or []
    exclude = entry.get("exclude_paths") or []
    if not paths and not exclude:
        return None
    if not isinstance(paths, list) or not isinstance(exclude, list):
        debug_log(f"extensibility: skipping {name}: paths/exclude_paths must be lists")
        return False
    return lambda p, _inc=tuple(paths), _exc=tuple(exclude): _glob_match(p, _inc, _exc)


def _validate_pattern(entry: Any, source: str) -> Optional[Dict[str, Any]]:
    """Validate one user pattern entry into the built-in SECURITY_PATTERNS shape."""
    if not isinstance(entry, dict):
        return None
    name = str(entry.get("rule_name", "")).strip()
    reminder = str(entry.get("reminder", "")).strip()[:PATTERN_REMINDER_MAX_BYTES]
    if not name or not reminder:
        debug_log(f"extensibility: skipping pattern without rule_name/reminder: {entry!r:.80}")
        return None
    regex = str(entry.get("regex", "")).strip()
    substrings = entry.get("substrings") or []
    if not isinstance(substrings, list) or not all(isinstance(item, str) for item in substrings):
        substrings = []
    if not regex and not substrings:
        debug_log(f"extensibility: skipping {name}: no regex or substrings")
        return None
    valid_regex = _validated_regex(name, regex) if regex else ""
    if regex and valid_regex is None:
        return None
    filter_fn = _path_filter(entry, name)
    if filter_fn is False:
        return None
    rule: Dict[str, Any] = {"ruleName": f"user:{name}", "reminder": reminder, "_source": source}
    if substrings:
        rule["substrings"] = substrings
    if valid_regex:
        rule["regex"] = valid_regex
    if filter_fn:
        rule["path_filter"] = filter_fn
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


# Catastrophic backtracking: nested quantifiers, ambiguous alternations under
# repetition, wildcard groups, and any structurally adjacent variable quantified
# atoms. The last rule intentionally fails closed instead of interpreting sets.


def _quantifier_bounds_at(regex: str, index: int) -> Optional[Tuple[int, str, Optional[str]]]:
    """Return a braced quantifier's exclusive end and decimal bounds."""
    if index >= len(regex) or regex[index] != "{":
        return None
    cursor = index + 1
    lower_start = cursor
    while cursor < len(regex) and regex[cursor].isdecimal():
        cursor += 1
    lower = regex[lower_start:cursor]
    if cursor < len(regex) and regex[cursor] == "}":
        return (cursor + 1, lower, None) if lower else None
    if cursor >= len(regex) or regex[cursor] != ",":
        return None
    cursor += 1
    upper_start = cursor
    while cursor < len(regex) and regex[cursor].isdecimal():
        cursor += 1
    if cursor >= len(regex) or regex[cursor] != "}":
        return None
    return cursor + 1, lower, regex[upper_start:cursor]


def _decimal_greater(left: str, right: str) -> bool:
    """Compare non-negative decimal strings without constructing huge integers."""
    normalized_left = left.lstrip("0") or "0"
    normalized_right = right.lstrip("0") or "0"
    return (len(normalized_left), normalized_left) > (len(normalized_right), normalized_right)


def _bounds_are_variable(lower: str, upper: Optional[str]) -> bool:
    """Return whether parsed decimal bounds permit more than one repeat count."""
    return upper is not None and (not upper or _decimal_greater(upper, lower or "0"))


def _quantifier_at(regex: str, index: int) -> Tuple[int, bool, bool]:
    """Return a quantifier's width, variability, and ability to consume its atom."""
    if index >= len(regex):
        return 0, False, True
    if regex[index] in "*+?":
        width = 2 if regex[index + 1:index + 2] in "?+" else 1
        return width, True, True
    bounds = _quantifier_bounds_at(regex, index)
    if bounds is None:
        return 0, False, True
    end, lower, upper = bounds
    width = end - index + (1 if regex[end:end + 1] in "?+" else 0)
    maximum = lower if upper is None else upper
    can_consume = not maximum or _decimal_greater(maximum, "0")
    return width, _bounds_are_variable(lower, upper), can_consume


def _group_quantifier(regex: str, closing_index: int, verbose: bool = False) -> str:
    """Return a supported quantifier after a group."""
    quantifier_index = _verbose_ignored_end(regex, closing_index + 1, verbose)
    if quantifier_index >= len(regex):
        return ""
    quantifier = regex[quantifier_index]
    if quantifier in "+*?":
        return quantifier
    bounds = _quantifier_bounds_at(regex, quantifier_index)
    if bounds is None:
        return ""
    _, lower, upper = bounds
    if upper is None:
        return "{}" if _decimal_greater(lower, "1") else ""
    if not upper:
        return "{}"
    return "{}" if _decimal_greater(upper, "1") else ""


def _new_group(flag_unsafe: bool, zero_width: bool = False) -> Dict[str, Any]:
    """Build bounded state for one group while scanning an untrusted pattern."""
    return {
        "first_branch": None,
        "branch_count": 0,
        "current": None,
        "direct_alternation": False,
        "ambiguous_branches": False,
        "nested": False,
        "child_alternation": False,
        "has_quantifier": False,
        "zero_width": zero_width,
        "flag_unsafe": flag_unsafe,
        "prefix": "start",
    }


def _record_atom(group: Dict[str, Any], token: Optional[str]) -> None:
    """Record only the first atom of a branch; distinct literals prove separation."""
    if group["current"] is None:
        group["current"] = token


def _finish_branch(group: Dict[str, Any]) -> None:
    """Fold one branch into a constant-size ambiguity summary."""
    if group["current"] is None:
        group["ambiguous_branches"] = True
    elif group["branch_count"] == 1 and group["first_branch"] == group["current"]:
        group["ambiguous_branches"] = True
    elif group["branch_count"] >= 2:
        group["ambiguous_branches"] = True
    if group["branch_count"] == 0:
        group["first_branch"] = group["current"]
    group["branch_count"] += 1
    group["current"] = None


def _direct_alternation_is_ambiguous(group: Dict[str, Any]) -> bool:
    """Reject repeated alternations unless their first two prefixes differ."""
    _finish_branch(group)
    return group["ambiguous_branches"]


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


def _global_flag_group_is_unsafe(regex: str, index: int) -> Optional[bool]:
    """Return global flag safety, or ``None`` when this is not a flag-only group."""
    if index + 2 >= len(regex) or regex[index + 1] != "?":
        return None
    flags_end = index + 2
    while flags_end < len(regex) and regex[flags_end] in "aiLmsux-":
        flags_end += 1
    if flags_end >= len(regex) or regex[flags_end] != ")":
        return None
    return any(flag in "iLx" for flag in regex[index + 2:flags_end])


def _open_group(regex: str, index: int, state: Dict[str, Any]) -> None:
    """Push a group summary, or retain a global unsafe inline-flag setting."""
    flag_unsafe = _global_flag_group_is_unsafe(regex, index)
    if flag_unsafe is not None:
        state["global_flag_unsafe"] |= flag_unsafe
        return
    stack = state["stack"]
    inherited = state["global_flag_unsafe"] or (stack and stack[-1]["flag_unsafe"])
    if stack:
        stack[-1]["nested"] = True
    zero_width = regex[index + 1:index + 3] in ("?=", "?!", "?<")
    stack.append(_new_group(bool(inherited), zero_width))


def _close_group(regex: str, index: int, stack: List[Dict[str, Any]], verbose: bool) -> bool:
    """Fold a completed group into its parent and report unsafe repetition."""
    group = stack.pop()
    quantifier = _group_quantifier(regex, index, verbose)
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
    if stack:
        parent = stack[-1]
        parent["has_quantifier"] |= (
            quantifier in ("+", "*", "{}") or group["has_quantifier"]
        ) and not group["zero_width"]
        parent["child_alternation"] |= has_alternation
        _record_atom(parent, group["current"] if not has_alternation else None)
    return unsafe


def _scan_active_group(
    regex: str, index: int, char: str, stack: List[Dict[str, Any]], state: Dict[str, Any]
) -> bool:
    """Update the active group summary for one non-escaped, non-class character."""
    group = stack[-1]
    if _consume_group_prefix(group, char):
        return False
    if index < state["quantifier_end"]:
        return False
    if char == "|":
        group["direct_alternation"] = True
        _finish_branch(group)
    elif char == ")":
        return _close_group(regex, index, stack, False)
    elif char in "+*?":
        group["has_quantifier"] = True
        state["quantifier_end"] = index + 2 if regex[index + 1:index + 2] in "?+" else index + 1
    elif char == "{":
        bounds = _quantifier_bounds_at(regex, index)
        width = _variable_quantifier_at(regex, index)
        group["has_quantifier"] |= bool(width)
        if bounds is not None:
            end = bounds[0]
            state["quantifier_end"] = end + 1 if regex[end:end + 1] in "?+" else end
    else:
        _record_atom(group, None if char in ".^$?" else char)
    return False


def _scan_group_token(regex: str, index: int, char: str, state: Dict[str, Any]) -> bool:
    """Consume one character for repeated-group analysis with bounded stack state."""
    stack = state["stack"]
    if index < state["escape_end"] or index < state["ignored_end"]:
        return False
    elif state["verbose"][-1] and char in " \t\n\r\f\v":
        return False
    elif state["verbose"][-1] and char == "#":
        newline = regex.find("\n", index + 1)
        state["ignored_end"] = len(regex) if newline == -1 else newline + 1
        return False
    elif char == "\\":
        if stack:
            _record_atom(stack[-1], None)
        state["escape_end"] = _escaped_atom_end(regex, index)
    elif char == "[":
        if stack:
            _record_atom(stack[-1], None)
        state["ignored_end"] = _class_end(regex, index)
    elif char == "(":
        if regex[index + 1:index + 3] == "?#":
            state["ignored_end"] = _comment_group_end(regex, index)
            return False
        stack_depth = len(stack)
        flags_end = _global_flags_end(regex, index)
        _open_group(regex, index, state)
        if flags_end:
            state["verbose"][-1] = _verbose_mode(regex[index + 2:flags_end - 1], state["verbose"][-1])
        elif len(stack) > stack_depth:
            state["verbose"].append(_scoped_verbose_mode(regex, index, state["verbose"][-1]))
    elif char == ")" and stack:
        verbose = state["verbose"][-2] if len(state["verbose"]) > 1 else False
        unsafe = _close_group(regex, index, stack, verbose)
        if len(state["verbose"]) > 1:
            state["verbose"].pop()
        return unsafe
    elif stack:
        return _scan_active_group(regex, index, char, stack, state)
    return False


def _repeated_groups(regex: str):
    """Yield unsafe repeated groups using a one-pass bounded stack summary."""
    state: Dict[str, Any] = {
        "stack": [], "global_flag_unsafe": False, "escape_end": 0, "ignored_end": 0,
        "quantifier_end": 0, "verbose": [False],
    }
    for index, char in enumerate(regex):
        if _scan_group_token(regex, index, char, state):
            yield None, "", False, True


def _variable_quantifier_at(regex: str, index: int) -> int:
    """Return the width of a variable quantifier, including an optional suffix."""
    width, variable, _ = _quantifier_at(regex, index)
    return width if variable else 0


def _escaped_atom_end(regex: str, index: int) -> int:
    """Return the end of one escaped atom using bounded Python-regex syntax."""
    cursor = min(index + 2, len(regex))
    if cursor == len(regex):
        return cursor
    escaped = regex[index + 1]
    if escaped == "N" and regex[cursor] == "{":
        end = regex.find("}", cursor + 1, min(cursor + 129, len(regex)))
        return end + 1 if end != -1 else cursor
    widths = {"x": 2, "u": 4, "U": 8}
    if escaped in widths:
        return min(cursor + widths[escaped], len(regex))
    if not escaped.isdecimal():
        return cursor
    octal_end = cursor
    while octal_end < min(index + 4, len(regex)) and regex[octal_end] in "01234567":
        octal_end += 1
    if escaped == "0" or octal_end == index + 4:
        return octal_end
    decimal_end = cursor
    while decimal_end < min(index + 3, len(regex)) and regex[decimal_end].isdecimal():
        decimal_end += 1
    return decimal_end


def _class_end(regex: str, index: int) -> int:
    """Consume one character class once, respecting escapes and a leading ``]``."""
    cursor = index + 1
    if cursor < len(regex) and regex[cursor] == "^":
        cursor += 1
    if cursor < len(regex) and regex[cursor] == "]":
        cursor += 1
    escaped = False
    while cursor < len(regex):
        char = regex[cursor]
        if char == "]" and not escaped:
            return cursor + 1
        escaped = char == "\\" and not escaped
        if char != "\\":
            escaped = False
        cursor += 1
    return cursor


def _global_flags_end(regex: str, index: int) -> int:
    """Return the end of a global inline-flag group, or zero when it opens a group."""
    cursor = index + 2
    while cursor < len(regex) and regex[cursor] in "aiLmsux-":
        cursor += 1
    return cursor + 1 if cursor > index + 2 and cursor < len(regex) and regex[cursor] == ")" else 0


def _group_body_start(regex: str, index: int) -> int:
    """Skip a group introducer without scanning any source character twice."""
    if index + 1 >= len(regex) or regex[index + 1] != "?":
        return index + 1
    if index + 2 < len(regex) and regex[index + 2] in ":=!":
        return index + 3
    if regex[index + 2:index + 4] in ("<=", "<!"):
        return index + 4
    if regex[index + 2:index + 4] == "P<":
        cursor = index + 4
        while cursor < len(regex) and cursor < index + 35 and regex[cursor] != ">":
            cursor += 1
        return cursor + 1 if cursor < len(regex) and regex[cursor] == ">" else index + 1
    cursor = index + 2
    while cursor < len(regex) and regex[cursor] in "aiLmsux-":
        cursor += 1
    return cursor + 1 if cursor > index + 2 and cursor < len(regex) and regex[cursor] == ":" else index + 1


def _verbose_mode(flags: str, inherited: bool) -> bool:
    """Apply an inline flag list's ``x`` setting to one scanner scope."""
    verbose, disabling = inherited, False
    for flag in flags:
        if flag == "-":
            disabling = True
        elif flag == "x":
            verbose = not disabling
    return verbose


def _scoped_verbose_mode(regex: str, index: int, inherited: bool) -> bool:
    """Return one scoped group's verbose mode without parsing its body."""
    if regex[index + 1:index + 2] != "?":
        return inherited
    cursor = index + 2
    while cursor < len(regex) and regex[cursor] in "aiLmsux-":
        cursor += 1
    if cursor >= len(regex) or regex[cursor] != ":":
        return inherited
    return _verbose_mode(regex[index + 2:cursor], inherited)


def _verbose_ignored_end(regex: str, index: int, verbose: bool) -> int:
    """Skip whitespace and comments that Python ignores in verbose mode."""
    if not verbose:
        return index
    while index < len(regex):
        if regex[index] in " \t\n\r\f\v":
            index += 1
        elif regex[index] == "#":
            newline = regex.find("\n", index + 1)
            index = len(regex) if newline == -1 else newline + 1
        else:
            break
    return index


def _comment_group_end(regex: str, index: int) -> int:
    """Skip an escape-aware Python comment group without changing scanner state."""
    escaped = False
    for cursor in range(index + 3, len(regex)):
        char = regex[cursor]
        if char == ")" and not escaped:
            return cursor + 1
        escaped = char == "\\" and not escaped
        if char != "\\":
            escaped = False
    return len(regex)


def _record_adjacent_atom(previous: List[bool], variable: bool, significant: bool) -> bool:
    """Fail closed when consecutive variable atoms can backtrack against each other."""
    if not significant:
        return False
    if variable and previous[-1]:
        return True
    previous[-1] = variable
    return False


def _close_adjacent_group(
    regex: str,
    index: int,
    previous: List[bool],
    content: List[List[bool]],
    zero_width: List[bool],
    verbose: List[bool],
    terminal_variable: List[bool],
    group_variable: List[bool],
) -> Tuple[int, bool, bool]:
    """Fold one completed group into its parent adjacency state."""
    child_content, child_zero = content.pop(), zero_width.pop()
    child_zero |= not child_content[1]
    previous.pop()
    terminal_variable.pop()
    child_variable = group_variable.pop()
    quantifier_start = _verbose_ignored_end(regex, index + 1, verbose[-1])
    width, quantified_variable, can_consume = _quantifier_at(regex, quantifier_start)
    variable = quantified_variable or (child_variable and can_consume)
    significant = child_content[0] and not child_zero and can_consume
    unsafe = _record_adjacent_atom(previous, variable, significant)
    content[-1][0] |= significant
    content[-1][1] |= significant
    return quantifier_start - index + width, unsafe, variable if significant else False


def _consume_adjacent_group(
    regex: str,
    index: int,
    previous: List[bool],
    content: List[List[bool]],
    zero_width: List[bool],
    verbose: List[bool],
    entry_previous: List[bool],
    terminal_variable: List[bool],
    group_variable: List[bool],
) -> Tuple[int, bool, bool]:
    """Open or close a group and return its next position plus rejection state."""
    if regex[index] == "(":
        if regex[index + 1:index + 3] == "?#":
            return _comment_group_end(regex, index), False, False
        flags_end = _global_flags_end(regex, index)
        if flags_end:
            verbose[-1] = _verbose_mode(regex[index + 2:flags_end - 1], verbose[-1])
            return flags_end, False, False
        child_zero = regex[index + 1:index + 3] in ("?=", "?!", "?<")
        zero_width.append(child_zero)
        entry_previous.append(False if child_zero else previous[-1])
        previous.append(entry_previous[-1])
        content.append([False, False])
        terminal_variable.append(False)
        group_variable.append(False)
        verbose.append(_scoped_verbose_mode(regex, index, verbose[-1]))
        return _group_body_start(regex, index), False, False
    if len(content) == 1:
        previous[-1] = False
        return index + 1, False, False
    verbose.pop()
    width, unsafe, variable = _close_adjacent_group(
        regex,
        index,
        previous,
        content,
        zero_width,
        verbose,
        terminal_variable,
        group_variable,
    )
    entry_previous.pop()
    return index + width, unsafe, variable


def _consume_adjacent_atom(
    regex: str,
    index: int,
    char: str,
    previous: List[bool],
    content: List[List[bool]],
    terminal_variable: List[bool],
    group_variable: List[bool],
    verbose: bool,
) -> Tuple[int, bool]:
    """Record one non-group atom and return its next position and rejection state."""
    if char in "^$":
        return index + 1, False
    if char in "?+*{":
        previous[-1] = terminal_variable[-1] = False
        return index + 1, False
    if char == "\\":
        end = _escaped_atom_end(regex, index)
        if index + 1 < len(regex) and regex[index + 1] in "AZbB":
            return end, False
    elif char == "[":
        end = _class_end(regex, index)
    else:
        end = index + 1
    quantifier_start = _verbose_ignored_end(regex, end, verbose)
    width, variable, can_consume = _quantifier_at(regex, quantifier_start)
    if not can_consume:
        return quantifier_start + width, False
    content[-1][0] = True
    content[-1][1] = True
    unsafe = _record_adjacent_atom(previous, variable, True)
    terminal_variable[-1] = variable
    group_variable[-1] |= variable
    return quantifier_start + width, unsafe


def _adjacent_quantifier_overlap(regex: str) -> bool:
    """Statically reject structurally adjacent variable quantified atoms."""
    previous, content, zero_width, verbose = [False], [[False, False]], [False], [False]
    entry_previous, terminal_variable, group_variable = [False], [False], [False]
    index = 0
    while index < len(regex):
        char = regex[index]
        ignored_end = _verbose_ignored_end(regex, index, verbose[-1])
        if ignored_end != index:
            index = ignored_end
            continue
        if char in "()":
            index, unsafe, variable = _consume_adjacent_group(
                regex, index, previous, content, zero_width, verbose, entry_previous,
                terminal_variable, group_variable,
            )
            if unsafe:
                return True
            if variable:
                terminal_variable[-1] = variable
                group_variable[-1] = True
            continue
        if char == "|":
            zero_width[-1] |= not content[-1][1]
            content[-1][1] = False
            previous[-1], terminal_variable[-1] = entry_previous[-1], False
            index += 1
            continue
        index, unsafe = _consume_adjacent_atom(
            regex, index, char, previous, content, terminal_variable, group_variable,
            verbose[-1],
        )
        if unsafe:
            return True
    return False


def _has_redos_structure(regex: str) -> bool:
    """Heuristic catastrophic-backtracking check using bounded static scans."""
    return _adjacent_quantifier_overlap(regex) or any(
        has_nested_quantifier for _, _, _, has_nested_quantifier in _repeated_groups(regex)
    )
