import os
import re
import sys

version = sys.version_info
release = tuple(version[:3])
pre = {"alpha": (0, version.serial), "beta": (1, version.serial), "candidate": (2, version.serial)}.get(version.releaselevel, (3, 0))


def compare(left, right):
    a, a_pre = left
    b, b_pre = right
    a = tuple(a[:next((i for i in range(len(a) - 1, -1, -1) if a[i] != 0), -1) + 1])
    b = tuple(b[:next((i for i in range(len(b) - 1, -1, -1) if b[i] != 0), -1) + 1])
    width = max(len(a), len(b))
    a, b = a + (0,) * (width - len(a)), b + (0,) * (width - len(b))
    if a != b:
        return (a > b) - (a < b)
    return (a_pre > b_pre) - (a_pre < b_pre)


def check_clause(clause):
    match = re.fullmatch(r"(>=|<=|==|!=|~=|>|<)\s*(\d+(?:\.\d+)*)(a|b|rc)?(\d+)?(\.\*)?", clause.strip(), re.I)
    if not match:
        op = re.match(r"([^\s\d]+)", clause.strip())
        return False, "unknown operator %s" % (op.group(1) if op else "(invalid clause)")
    op, raw, pre_label, pre_serial, wildcard = match.groups()
    if pre_label and pre_serial is None:
        return False, "invalid pre-release clause"
    wanted_pre = (({"a": 0, "b": 1, "rc": 2}[(pre_label or "").lower()], int(pre_serial)) if pre_label else (3, 0))
    wanted = (tuple(map(int, raw.split("."))), wanted_pre)
    order = compare((release, pre), wanted)
    if wildcard and op in ("==", "!="):
        prefix = wanted[0]
        equal = release[:len(prefix)] == prefix
        return (equal if op == "==" else not equal), None
    if wildcard:
        return False, "wildcard is only valid with == or !="
    if op == ">=":
        return order >= 0, None
    if op == ">":
        return order > 0, None
    if op == "<=":
        return order <= 0, None
    if op == "<":
        return order < 0, None
    if op == "==":
        return order == 0, None
    if op == "!=":
        return order != 0, None
    if op == "~=":
        if len(wanted[0]) < 2:
            return False, "invalid compatible-release clause"
        upper = list(wanted[0][:-1])
        upper[-1] += 1
        upper_bound = (tuple(upper), (3, 0))
        return order >= 0 and compare((release, pre), upper_bound) < 0, None


def pep440_key(text):
    pattern = r"^\s*v?(?:(\d+)!)?(\d+(?:\.\d+)*)(?:(?:[-_.]?)(a|b|c|rc|alpha|beta|pre|preview)(?:[-_.]?(\d+))?)?(?:(?:-(\d+)|(?:[-_.]?post|rev|r)(?:[-_.]?(\d+))?))?(?:[-_.]?dev(?:[-_.]?(\d+))?)?(?:\+([a-z0-9]+(?:[-_.][a-z0-9]+)*))?\s*$"
    match = re.fullmatch(pattern, text, re.I)
    if not match:
        raise ValueError("invalid PEP 440 version: " + text)
    epoch, raw_release, label, pre_number, dash_post, post_number, dev_number, local = match.groups()
    release = list(map(int, raw_release.split(".")))
    while release and release[-1] == 0:
        release.pop()
    label = (label or "").lower()
    pre_rank = {"a": 0, "alpha": 0, "b": 1, "beta": 1, "c": 2, "rc": 2, "pre": 2, "preview": 2}.get(label)
    dev = int(dev_number) if dev_number is not None else None
    if pre_rank is None:
        pre = (-1, 0) if dev is not None else (3, 0)
    else:
        pre = (pre_rank, int(pre_number or 0))
    post = int(dash_post or post_number) if dash_post or post_number else -1
    local_key = (-1,) if local is None else tuple(
        (1, int(part)) if part.isdigit() else (0, part.lower())
        for part in re.split(r"[-_.]", local)
    )
    return int(epoch or 0), tuple(release), pre, post, (-1, dev) if dev is not None else (1, 0), local_key


if len(sys.argv) == 4 and sys.argv[1] == "--compare":
    left, right = pep440_key(sys.argv[2]), pep440_key(sys.argv[3])
    print((left > right) - (left < right))
    raise SystemExit(0)


spec = os.environ["ARCHON_SETUP_PYTHON_SPECIFIER"]
clauses = spec.split(",")
for clause in clauses:
    _, error = check_clause(clause)
    if error:
        print("%s (treated as unsatisfied)" % error)
        raise SystemExit(2)
for clause in clauses:
    passed, _ = check_clause(clause)
    if not passed:
        raise SystemExit(1)
raise SystemExit(0)
