# security-guidance

> Ported from the Claude Code `security-guidance` plugin ([source](https://github.com/ste-bah/test-plugins-official/tree/main/plugins/security-guidance), Apache-2.0).

Passive security warnings for agent-generated code: instant regex-based reminders on `Edit`/`Write`/`MultiEdit`/`NotebookEdit` for ~25 known-dangerous patterns (`yaml.load`, `torch.load(weights_only=False)`, `pickle.load` on untrusted data, raw `innerHTML`, `subprocess ... shell=True`, disabled TLS verification, missing Subresource Integrity, and more).

Findings cover common vulnerability classes — injection, XSS, unsafe deserialization, weak crypto modes, XXE, and GitHub Actions workflow injection among others.

Archon also ships a built-in `/security-review` skill for on-demand, in-depth review of pending changes. This plugin complements it with passive hooks: `/security-review` is a deliberate deep pass you invoke; security-guidance fires automatically on every edit and nudges the agent the moment a dangerous pattern is written, with no interaction needed.

## Install

Project-local install:

1. Copy `scripts/` to `<project>/.archon/plugins/security-guidance/scripts/`.
2. Enable the hooks (see below).

Or user-global: copy `scripts/` to `~/.archon/plugins/security-guidance/scripts/` and adjust the paths in the settings snippet accordingly.

Or run `plugins/install.ps1 security-guidance` / `plugins/install.sh security-guidance` from the archon-cli repo root, then enable the hooks.

## Enable hooks

Hooks are NOT auto-loaded from plugin directories. Merge `hooks/settings.snippet.json` into `<project>/.archon/settings.json` (create the file with that content if it doesn't exist; otherwise merge the `"hooks"` keys). The snippet registers one PostToolUse hook on `Edit|Write|MultiEdit|NotebookEdit` that runs the pattern checks.

If you installed user-globally, change the two `.archon/plugins/security-guidance/scripts/...` paths in the snippet to `~/.archon/plugins/security-guidance/scripts/...`.

## Prerequisites

- bash (Git Bash on Windows) and Python 3 on `PATH` (`python3`, `python`, or `py -3` — the `sg-python.sh` shim picks the first that works, preferring 3.10+; the pattern checks themselves only need 3.6+)
- No Python package dependencies for the built-in checks (PyYAML only if you write custom patterns in YAML instead of JSON)

## Configuration

All configuration is via environment variables. None are required for default behavior.

| Variable | Default | What it does |
|---|---|---|
| `SECURITY_GUIDANCE_DISABLE=1` | unset | Kill switch — disables the entire plugin |
| `ENABLE_SECURITY_REMINDER=0` | unset | Legacy kill switch (same effect) |
| `ENABLE_PATTERN_RULES=0` | on | Disable the regex pattern warnings |
| `SECURITY_WARNINGS_STATE_DIR` | `~/.archon/security` | Override the state/log directory |
| `SECURITY_GUIDANCE_DEBUG_LOG` | `<state dir>/log.txt` | Override the debug log path |

## Custom patterns

Drop a `security-patterns.yaml` (or `.yml` / `.json`) in any of:

- `~/.archon/security-patterns.yaml` — user-wide rules
- `<project>/.archon/security-patterns.yaml` — project rules, intended to be committed
- `<project>/.archon/security-patterns.local.yaml` — local overrides, intended to be `.gitignore`'d

All are loaded and merged with the built-in rules (capped at 50 custom rules; reminders capped at 1 KB). Example:

```yaml
patterns:
  - rule_name: primary-db-select
    regex: 'db\.primary\.(select|query)'
    reminder: >
      SELECTs must go through db.replica, never db.primary.
      Primary is for writes only.
    paths: ["**/*.py"]
  - rule_name: raw-requests-get
    substrings: ["requests.get(url"]
    reminder: Calls with a user-controlled url need the SSRF-allowlist wrapper.
    exclude_paths: ["tests/**"]
```

Each entry needs `rule_name`, `reminder`, and at least one of `regex` / `substrings`; optional `paths` / `exclude_paths` globs gate which files the rule applies to. Regexes are validated at load time and skipped if they look ReDoS-prone. Built-in patterns cannot be disabled per-rule.

## How warnings behave

- Warnings print once per `file + rule` per session (tracked in a per-session state file under `~/.archon/security/`), so repeated edits to the same file don't spam the agent.
- Warnings are informational: the edit is never blocked. Many reminders explicitly tell the agent that if the flagged usage is safe and intentional, it should briefly document that in a comment and continue.
- Each warning block is prefixed with a provenance tag identifying it as automated plugin output, not user input.

## Privacy and data handling

The Archon port performs all checks locally with regex — no file contents, diffs, or prompts are sent to any model endpoint or network service by this plugin. The plugin writes its own debug log to `~/.archon/security/log.txt` (override with `SECURITY_GUIDANCE_DEBUG_LOG`). The log contains hook-event metadata and matched rule names — no full file contents — and rotates at 1 MB. Nothing is uploaded.

## Limitations

This is a best-effort assistive tool, not a guarantee. Treat findings as suggestions, not as a substitute for human code review, SAST/DAST, dependency scanning, or pen-testing. Regex checks can miss vulnerabilities and produce false positives (e.g. multi-line `yaml.load(...)` / `torch.load(...)` calls are a known false-positive shape). **No warranty is provided.**

## Troubleshooting

**Plugin doesn't seem to fire** — check that the snippet is merged into `.archon/settings.json` and the scripts exist at `.archon/plugins/security-guidance/scripts/`. The plugin writes its own log to `~/.archon/security/log.txt`; a fresh entry per edit confirms the hook is running.

**No Python found** — the shim prints which interpreters it tried. On Windows, install Python from https://python.org (NOT the Microsoft Store); on macOS, `brew install python`.

**Too many warnings on a file you trust** — add a comment on the flagged line explaining why it's safe (the reminders ask the agent to do exactly this), or narrow/disable via `ENABLE_PATTERN_RULES=0` if you only want Archon's `/security-review`.

**Custom YAML patterns ignored** — PyYAML may not be installed for the interpreter the shim picked; use the `.json` form instead, or `pip install pyyaml`.

## Differences from the Claude plugin

The upstream plugin had three layers; this port keeps layer 1 and drops the LLM-driven layers, which depend on Claude-specific infrastructure with no Archon equivalent:

- **Layer 2 (LLM diff review on Stop) not ported** — it relied on Claude Code's `asyncRewake` hook fields (`rewakeMessage`/`rewakeSummary`), transcript access, git diff-state tracking across turns, and direct Anthropic API / Claude Agent SDK calls (`llm.py`, `review_api.py`, `diffstate.py`, `gitutil.py`, ~3,400 lines). Archon's hook protocol has no async-rewake mechanism. Use Archon's built-in `/security-review` skill for deep review instead.
- **Layer 3 (agentic commit/push review) not ported** — same dependencies plus the `claude_agent_sdk` bootstrap (`ensure_agent_sdk.py`) and the `git commit`/`git push`/`gt` PostToolUse matchers with `if:` conditions and dedup sentinels. The SessionStart, UserPromptSubmit, Stop, and Bash-matcher hook entries from the source `hooks.json` are therefore not present in the settings snippet; only the Edit/Write PostToolUse entry remains.
- **Env vars dropped with those layers**: `SECURITY_REVIEW_MODEL`, `SG_AGENTIC_MODEL`, `ENABLE_CODE_SECURITY_REVIEW`, `ENABLE_STOP_REVIEW`, `ENABLE_COMMIT_REVIEW`, `SG_DUAL_OR`.
- **`claude-security-guidance.md` org-policy files are not read** — they were only injected into the LLM review prompts. The custom-pattern mechanism (`security-patterns.{yaml,json}`, ported with `.claude/` paths adapted to `.archon/`) still works and is the extension point for org-specific checks.
- **Hook output protocol**: warnings print as plain provenance-tagged text on stdout instead of Claude's `hookSpecificOutput.additionalContext` JSON; the `metrics` JSON channel and the `RuleId` telemetry bitmask in `patterns.py` were dropped (Archon has no hook metrics pipeline).
- **Write-baseline suppression dropped**: the upstream suppressed warnings for patterns that already existed in the pre-session git baseline of a rewritten file; that depended on the diff-state layer, so a full-file `Write` may re-warn about pre-existing patterns. Per-session `file+rule` dedup is retained.
- **Paths and env adapted**: state/log dir `~/.claude/security` → `~/.archon/security`; `CLAUDE_CONFIG_DIR` → `ARCHON_CONFIG_DIR`; plan-file skip `~/.claude/plans` → `~/.archon/plans`; `CLAUDE_CODE_REMOTE_SESSION_ID` state-key override dropped; the Claude plugin-root variable in hook commands → the fixed install path `.archon/plugins/security-guidance`.
- **Event JSON fields**: the hook reads Archon's `tool_args`/`event` (with a defensive fallback to Claude's `tool_input`/`hook_event_name`).
- **Hook registration**: Claude auto-loaded `hooks/hooks.json`; Archon requires merging `hooks/settings.snippet.json` into `.archon/settings.json` (see "Enable hooks").
- **Marketplace install/version/issue-reporting sections** of the source README were replaced with Archon instructions; the plugin no longer reads a `plugin.json` version.

## Reporting issues

The Archon port is maintained in the archon-cli repo — open an issue there with a minimal repro edit and the relevant section of `~/.archon/security/log.txt`.

## License

Apache License 2.0 (see LICENSE).
