#!/usr/bin/env bash
# Find a working Python 3 interpreter and exec the hook with it.
#
# On Windows + Git Bash, `python3` typically resolves to the Microsoft Store
# stub at C:\Users\<user>\AppData\Local\Microsoft\WindowsApps\python3, which
# exits 49 silently in non-TTY subprocess context (a known Microsoft Store
# stub behavior). This shim
# probes each candidate with a version print and skips any that fails, so the
# Store stub falls through to the real python.org install (`python` in Git
# Bash) or the `py -3` launcher.
#
# Order:
#   1. python3   — canonical on macOS/Linux; the Store stub fails the probe.
#   2. python    — python.org installs on Windows; some Linux distros (RHEL 7
#                  EOL'd 2024-06) point this at Python 2, but the probe
#                  succeeds on Python 2 too — guard with a version check.
#   3. py -3     — Windows Python launcher.
#
# Args after the shim path are passed straight through to the chosen
# interpreter, so the .archon/settings.json invocation is:
#   bash .archon/plugins/security-guidance/scripts/sg-python.sh \
#        .archon/plugins/security-guidance/scripts/security_warning_hook.py
set -e

# Force UTF-8 for ALL Python filesystem + IO operations (PEP 540).
# Without this, Windows Python defaults `locale.getpreferredencoding()` to
# cp1252 — which makes `text=True` in subprocess.run / open() / json.load
# crash the internal reader thread on any byte that's undefined in cp1252
# (e.g. the 0x81 byte from ف, present in any path/filename with
# Arabic/Hebrew/CJK characters).
#
# No-op on macOS/Linux (already UTF-8). Must be set BEFORE Python starts —
# changing it from inside the interpreter has no effect.
export PYTHONUTF8=1

# Git Bash / MSYS on Windows hands script paths to this shim in POSIX form
# (`/c/Users/...`). When we exec a Windows `python.exe` (which we do on
# Windows since `python3` is the Microsoft Store stub), python interprets the
# leading `/` as the root of the current drive — e.g. `/c/Users/...` becomes
# `C:\c\Users\...` or `D:\c\Users\...` (whichever drive the shell is on) and
# fails with ENOENT.
#
# Fix: convert absolute path args to native Windows form via `cygpath -w`
# before exec. `cygpath` is a Git Bash builtin; it's absent on macOS/Linux,
# where the `command -v` guard makes this a no-op. `cygpath -w` is idempotent
# for already-Windows paths so the rare mixed-form case is safe.
if command -v cygpath >/dev/null 2>&1; then
    converted=()
    for a in "$@"; do
        case "$a" in
            /*) converted+=("$(cygpath -w "$a")") ;;
            *)  converted+=("$a") ;;
        esac
    done
    set -- "${converted[@]}"
fi

probe() {
    # $1..N: the interpreter command (may be multi-word like `py -3`)
    # Writes "<major>.<minor>" to stdout and exits 0 iff the probe ran.
    "$@" -c 'import sys; print(f"{sys.version_info[0]}.{sys.version_info[1]}")' 2>/dev/null
}

# True iff arg is a "M.m" version string >= 3.10. The Archon port's pattern
# checks only need Python 3.6+, but preferring an explicitly-installed
# modern interpreter (Homebrew / python.org / pyenv) over an aging system
# `python3` keeps behavior consistent with the upstream shim and avoids
# EOL'd interpreters.
is_modern() {
    case "$1" in
        3.1[0-9]|3.[2-9][0-9]|[4-9].*|[1-9][0-9].*) return 0 ;;
        *) return 1 ;;
    esac
}

# Pass 1 — try minor-versioned binaries in descending order. These are only
# present if the user explicitly installed them (Homebrew / python.org /
# pyenv), so picking one here always upgrades over the system `python3`.
# Highest available wins; the user doesn't have to PATH-prefer it.
for cmd in "python3.13" "python3.12" "python3.11" "python3.10"; do
    v=$(probe "$cmd") || continue
    if is_modern "$v"; then
        exec "$cmd" "$@"
    fi
done

# Pass 2 — bare interpreters, but only if modern. Covers Linux distros that
# ship 3.10+ as the default `python3`, and Windows where `python` / `py -3`
# resolves to the user's python.org install.
for cmd in "python3" "python" "py -3"; do
    # shellcheck disable=SC2086
    v=$(probe $cmd) || continue
    if is_modern "$v"; then
        # shellcheck disable=SC2086
        exec $cmd "$@"
    fi
done

# Pass 3 — fallback to any Python 3 (or anything that reported a version).
# The pattern-based checks only need 3.6+ and are useful on their own; this
# keeps the plugin working on default macOS installs.
for cmd in "python3" "python" "py -3"; do
    # shellcheck disable=SC2086
    v=$(probe $cmd) || continue
    # Accept anything that successfully reported a "M.m" string.
    case "$v" in
        [0-9]*.[0-9]*)
            # shellcheck disable=SC2086
            exec $cmd "$@"
            ;;
    esac
done

echo "security-guidance: no working Python 3 interpreter found." >&2
echo "  tried: python3.13, python3.12, python3.11, python3.10, python3, python, py -3" >&2
echo "  on Windows, install Python from https://python.org (NOT the Microsoft Store)" >&2
echo "  on macOS, install Python 3.10+ via Homebrew (\`brew install python\`)" >&2
exit 1
