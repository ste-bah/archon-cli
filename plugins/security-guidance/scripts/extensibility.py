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
    except re.error as exc:
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
    stack.append(_new_group(bool(inherited)))


def _close_group(regex: str, index: int, stack: List[Dict[str, Any]]) -> bool:
    """Fold a completed group into its parent and report unsafe repetition."""
    group = stack.pop()
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
    if stack:
        parent = stack[-1]
        parent["has_quantifier"] |= quantifier in ("+", "*", "{}") or group["has_quantifier"]
        parent["child_alternation"] |= has_alternation
        _record_atom(parent, group["current"] if not has_alternation else None)
    return unsafe


def _scan_active_group(regex: str, index: int, char: str, stack: List[Dict[str, Any]]) -> bool:
    """Update the active group summary for one non-escaped, non-class character."""
    group = stack[-1]
    if _consume_group_prefix(group, char):
        return False
    if char == "|":
        group["direct_alternation"] = True
        _finish_branch(group)
    elif char == ")":
        return _close_group(regex, index, stack)
    elif char in "+*{":
        group["has_quantifier"] = True
    else:
        _record_atom(group, None if char in ".^$?" else char)
    return False


def _scan_group_token(regex: str, index: int, char: str, state: Dict[str, Any]) -> bool:
    """Consume one character for repeated-group analysis with bounded stack state."""
    stack = state["stack"]
    if state["escaped"]:
        if stack:
            _record_atom(stack[-1], None)
        state["escaped"] = False
    elif char == "\\":
        if stack:
            _record_atom(stack[-1], None)
        state["escaped"] = True
    elif state["in_class"]:
        state["in_class"] = char != "]"
    elif char == "[":
        if stack:
            _record_atom(stack[-1], None)
        state["in_class"] = True
    elif char == "(":
        _open_group(regex, index, state)
    elif stack:
        return _scan_active_group(regex, index, char, stack)
    return False


def _repeated_groups(regex: str):
    """Yield unsafe repeated groups using a one-pass bounded stack summary."""
    state: Dict[str, Any] = {
        "stack": [], "global_flag_unsafe": False, "escaped": False, "in_class": False
    }
    for index, char in enumerate(regex):
        if _scan_group_token(regex, index, char, state):
            yield None, "", False, True


def _variable_quantifier_at(regex: str, index: int) -> int:
    """Return the width of an unbounded quantifier beginning at ``index``."""
    if index >= len(regex):
        return 0
    if regex[index] in "*+":
        return 1
    if regex[index] != "{":
        return 0
    end = regex.find("}", index + 1, index + 33)
    if end == -1 or "," not in regex[index + 1:end]:
        return 0
    lower, upper = regex[index + 1:end].split(",", 1)
    return end - index + 1 if lower.isdecimal() and not upper else 0


def _radix_sort_ranges(ranges: List[Tuple[int, int]]) -> Tuple[Tuple[int, int], ...]:
    """Sort Unicode codepoint ranges by start in linear fixed-width radix passes."""
    ordered: List[Tuple[int, int]] = ranges
    for shift in (0, 8, 16):
        counts = [0] * 256
        for start, _ in ordered:
            counts[(start >> shift) & 255] += 1
        offsets = []
        total = 0
        for count in counts:
            offsets.append(total)
            total += count
        output: List[Tuple[int, int]] = [(0, 0)] * len(ordered)
        for item in ordered:
            bucket = (item[0] >> shift) & 255
            output[offsets[bucket]] = item
            offsets[bucket] += 1
        ordered = output
    return tuple(ordered)


def _class_ranges(body: str) -> Tuple[bool, Tuple[Tuple[int, int], ...]]:
    """Return negation and linear-time normalized literal/range codepoint spans."""
    negated = body.startswith("^")
    index = int(negated)
    ranges = []
    while index < len(body):
        start = body[index]
        if start == "\\" and index + 1 < len(body):
            start, index = body[index + 1], index + 2
        else:
            index += 1
        end = start
        if index + 1 < len(body) and body[index] == "-":
            end, index = body[index + 1], index + 2
        start_point, end_point = ord(start), ord(end)
        ranges.append((min(start_point, end_point), max(start_point, end_point)))
    return negated, _radix_sort_ranges(ranges)


def _escaped_atom(regex: str, index: int) -> Tuple[Tuple[str, Any], int]:
    """Describe an escaped atom without compiling or matching the pattern."""
    if index + 1 >= len(regex):
        return ("literal", "\\"), 1
    escaped = regex[index + 1]
    if escaped in "wdsWDsS":
        return ("category", escaped), 2
    return ("literal", escaped), 2


def _class_atom(regex: str, index: int) -> Tuple[Tuple[str, Any], int]:
    """Describe one class, consuming an unterminated class exactly once."""
    cursor = index + 1
    escaped = False
    while cursor < len(regex):
        char = regex[cursor]
        if char == "]" and not escaped:
            return ("class", _class_ranges(regex[index + 1:cursor])), cursor - index + 1
        escaped = char == "\\" and not escaped
        if char != "\\":
            escaped = False
        cursor += 1
    return ("invalid", None), cursor - index


def _atom_at(regex: str, index: int) -> Tuple[Tuple[str, Any], int]:
    """Describe one simple atom and its source width for overlap checks."""
    char = regex[index]
    if char == "\\":
        return _escaped_atom(regex, index)
    if char == "[":
        return _class_atom(regex, index)
    if char == ".":
        return ("any", None), 1
    return ("literal", char), 1


def _range_sets_overlap(
    left: Tuple[Tuple[int, int], ...], right: Tuple[Tuple[int, int], ...]
) -> bool:
    """Return whether sorted codepoint ranges intersect in one linear merge."""
    left_index = right_index = 0
    while left_index < len(left) and right_index < len(right):
        left_range, right_range = left[left_index], right[right_index]
        if left_range[1] < right_range[0]:
            left_index += 1
        elif right_range[1] < left_range[0]:
            right_index += 1
        else:
            return True
    return False


_CATEGORY_SUBSETS = {"d": {"d", "w"}, "s": {"s"}, "w": {"w"}}


def _category_overlap(left: str, right: str) -> bool:
    """Compare Python shorthand categories without evaluating untrusted text."""
    left_name, right_name = left.lower(), right.lower()
    left_positive, right_positive = left.islower(), right.islower()
    if left_positive and right_positive:
        return {left_name, right_name} not in ({"d", "s"}, {"s", "w"})
    if not left_positive and not right_positive:
        return True
    positive, negative = (left_name, right_name) if left_positive else (right_name, left_name)
    return negative not in _CATEGORY_SUBSETS[positive]


def _atom_matches_range(atom: Tuple[str, Any], ranges: Tuple[Tuple[int, int], ...]) -> bool:
    """Compare a simple atom with one non-negated character class."""
    if atom[0] == "literal":
        point = ord(atom[1])
        return any(start <= point <= end for start, end in ranges)
    return atom[0] in ("any", "category")


def _category_matches_literal(category: str, literal: str) -> bool:
    """Return whether a shorthand category can consume one literal codepoint."""
    name = category.lower()
    if name == "d":
        matches = literal.isdecimal()
    elif name == "s":
        matches = literal.isspace()
    else:
        matches = literal == "_" or literal.isalnum()
    return matches if category.islower() else not matches


def _atoms_overlap(left: Tuple[str, Any], right: Tuple[str, Any]) -> bool:
    """Conservatively compare simple atom descriptors without compiling a regex."""
    if "any" in (left[0], right[0]):
        return True
    if left[0] == right[0] == "literal":
        return left[1] == right[1]
    if left[0] == right[0] == "folded_literal":
        return left[1] == right[1]
    if left[0] == "folded_literal" and right[0] == "literal":
        return left[1] == right[1].casefold()
    if right[0] == "folded_literal" and left[0] == "literal":
        return right[1] == left[1].casefold()
    if left[0] == right[0] == "category":
        return _category_overlap(left[1], right[1])
    if left[0] == "category" and right[0] == "literal":
        return _category_matches_literal(left[1], right[1])
    if right[0] == "category" and left[0] == "literal":
        return _category_matches_literal(right[1], left[1])
    if left[0] == right[0] == "class":
        left_negated, left_ranges = left[1]
        right_negated, right_ranges = right[1]
        return left_negated or right_negated or _range_sets_overlap(left_ranges, right_ranges)
    if left[0] == "class":
        negated, ranges = left[1]
        return negated or _atom_matches_range(right, ranges)
    if right[0] == "class":
        negated, ranges = right[1]
        return negated or _atom_matches_range(left, ranges)
    return False


def _record_quantified_atom(
    previous: List[Optional[Tuple[str, Any]]], atom: Optional[Tuple[str, Any]], quantifier_width: int
) -> bool:
    """Record one atom or report overlap with the previous unbounded atom."""
    if not quantifier_width or atom is None:
        previous[-1] = None
        return False
    if previous[-1] is not None and _atoms_overlap(previous[-1], atom):
        return True
    previous[-1] = atom
    return False


def _append_group_atom(
    atoms: List[Optional[Tuple[str, Any]]], counts: List[int], atom: Optional[Tuple[str, Any]]
) -> None:
    """Keep a group descriptor only while it contains exactly one simple atom."""
    counts[-1] += 1
    atoms[-1] = atom if counts[-1] == 1 else None


def _scoped_flags(regex: str, index: int) -> str:
    """Return scoped group flags, limited to its short introducer."""
    cursor = index + 2
    while cursor < len(regex) and regex[cursor] in "aiLmsux-":
        cursor += 1
    return regex[index + 2:cursor] if cursor > index + 2 and cursor < len(regex) and regex[cursor] == ":" else ""


def _group_body_start(regex: str, index: int) -> int:
    """Skip group introducers, including scoped flags, before body atom scanning."""
    if index + 1 >= len(regex) or regex[index + 1] != "?":
        return index + 1
    if index + 2 < len(regex) and regex[index + 2] in ":=!":
        return index + 3
    if regex[index + 2:index + 4] in ("<=", "<!"):
        return index + 4
    if regex[index + 2:index + 4] == "P<":
        end = regex.find(">", index + 4, index + 35)
        return end + 1 if end != -1 else index + 1
    flags = _scoped_flags(regex, index)
    return index + 3 + len(flags) if flags else index + 1


def _apply_scoped_flags(atom: Optional[Tuple[str, Any]], flags: str) -> Optional[Tuple[str, Any]]:
    """Preserve case-insensitive literal equivalence for a completed scoped group."""
    if atom is not None and atom[0] == "literal" and "i" in flags:
        return "folded_literal", atom[1].casefold()
    return atom


def _adjacent_quantifier_overlap(regex: str) -> bool:
    """Detect adjacent overlapping variable-length atoms at every group nesting level."""
    previous: List[Optional[Tuple[str, Any]]] = [None]
    group_atoms: List[Optional[Tuple[str, Any]]] = [None]
    group_counts = [0]
    group_flags = [""]
    index = 0
    while index < len(regex):
        char = regex[index]
        if char == "(":
            previous.append(None)
            group_atoms.append(None)
            group_counts.append(0)
            group_flags.append(_scoped_flags(regex, index))
            index = _group_body_start(regex, index)
            continue
        if char == ")":
            atom = _apply_scoped_flags(group_atoms.pop(), group_flags.pop()) if len(group_atoms) > 1 else None
            if len(group_counts) > 1:
                group_counts.pop()
                previous.pop()
            quantifier_width = _variable_quantifier_at(regex, index + 1)
            if _record_quantified_atom(previous, atom, quantifier_width):
                return True
            _append_group_atom(group_atoms, group_counts, atom)
            index += 1 + quantifier_width
            continue
        if char in "|^$?+*{":
            previous[-1] = None
            group_atoms[-1] = None
            group_counts[-1] += 1
            index += 1
            continue
        atom, width = _atom_at(regex, index)
        quantifier_width = _variable_quantifier_at(regex, index + width)
        if _record_quantified_atom(previous, atom, quantifier_width):
            return True
        _append_group_atom(group_atoms, group_counts, atom)
        index += width + quantifier_width
    return False


def _has_redos_structure(regex: str) -> bool:
    """Heuristic catastrophic-backtracking check using bounded static scans."""
    return _adjacent_quantifier_overlap(regex) or any(
        has_nested_quantifier for _, _, _, has_nested_quantifier in _repeated_groups(regex)
    )
