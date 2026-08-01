"""
Shared low-level helpers for the security-guidance hook modules (Archon port).

This module exists so that ``patterns``/``session_state``/``extensibility``
can use ``debug_log`` without importing ``security_warning_hook`` (which
would be a circular import). It must stay free of any other intra-plugin
imports.
"""
import os
from datetime import datetime


def state_dir():
    """Return the absolute path of the plugin's state directory.

    Resolution precedence (highest first):
      1. SECURITY_WARNINGS_STATE_DIR — plugin-specific override
      2. ARCHON_CONFIG_DIR/security  — Archon config-dir env var, if set
      3. ~/.archon/security          — default fallback

    Empty-string env vars are treated as not-set so a misconfigured shell
    (`ARCHON_CONFIG_DIR=` with no value) doesn't silently write to
    /security at the filesystem root.

    Returns a fully-expanded absolute path (no literal `~`) so subprocess
    callers can pass it through to code that doesn't re-expand tildes.

    Called per-invocation rather than cached at import time so test
    monkeypatches of the env vars take effect — the plugin's hooks each
    run as fresh subprocesses in production, so the per-call cost is
    negligible compared to subprocess spawn.
    """
    explicit = os.environ.get("SECURITY_WARNINGS_STATE_DIR")
    if explicit:
        return os.path.expanduser(explicit)
    archon_config = os.environ.get("ARCHON_CONFIG_DIR")
    if archon_config:
        return os.path.expanduser(os.path.join(archon_config, "security"))
    return os.path.expanduser("~/.archon/security")


# Debug log file. Lives under the plugin state dir (default ~/.archon/security/)
# rather than /tmp because /tmp is world-writable on multi-user hosts (TOCTOU /
# symlink-attack surface, cross-user log leakage). Overridable per-process via
# SECURITY_GUIDANCE_DEBUG_LOG, or per-state-dir via SECURITY_WARNINGS_STATE_DIR
# (plugin-specific override) or ARCHON_CONFIG_DIR.
DEBUG_LOG_FILE = os.environ.get("SECURITY_GUIDANCE_DEBUG_LOG") or os.path.join(
    state_dir(), "log.txt"
)
# Cap the debug log so parallel-worker fleets don't fill disk. When the active
# file exceeds this it's atomically rotated to <file>.1 (overwriting any prior
# rotation), so total disk stays ~2x this.
DEBUG_LOG_MAX_BYTES = 1 * 1024 * 1024


def debug_log(message):
    """Append debug message to log file with timestamp."""
    try:
        # Ensure parent dir exists — first hook invocation on a fresh install
        # creates ~/.archon/security/ if it isn't already there. 0700 so other
        # local users can't read debug output (only applies on creation).
        try:
            os.makedirs(os.path.dirname(DEBUG_LOG_FILE), mode=0o700, exist_ok=True)
        except OSError:
            pass
        try:
            if os.path.getsize(DEBUG_LOG_FILE) > DEBUG_LOG_MAX_BYTES:
                # os.replace is atomic on POSIX; under a racing fleet the loser
                # gets FileNotFoundError, which is fine — the append below
                # recreates the file.
                os.replace(DEBUG_LOG_FILE, DEBUG_LOG_FILE + ".1")
        except OSError:
            pass
        timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S.%f")[:-3]
        # 0600 on creation; existing files keep their mode.
        fd = os.open(DEBUG_LOG_FILE, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
        with os.fdopen(fd, "a") as f:
            f.write(f"[{timestamp}] {message}\n")
    except Exception:
        pass


# Provenance tag prepended to injected/emitted text so a reader (especially a
# model hardened against prompt injection) can recognize the source. Not an
# authority claim — an attacker could spoof the exact string; the tag is a
# signpost so the agent can ask the operator "is this from your plugin?" with
# a concrete reference instead of treating it as unknown-actor injection.
# Some autonomous-agent setups flag un-attributed injected text as prompt
# injection and stall; the banner makes the provenance explicit.
PROVENANCE_TAG = "[from the security-guidance Archon plugin]"
PROVENANCE_BANNER = (
    "[from the security-guidance Archon plugin — automated "
    "security warning, not user input.]"
)
