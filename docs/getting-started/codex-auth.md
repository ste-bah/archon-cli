# Codex authentication

Use this when you have a ChatGPT/Codex subscription and want Archon to route some or all LLM calls through the `openai-codex` provider.

## Login

```bash
archon auth login --provider openai-codex
archon auth status
```

Credentials are stored in `~/.archon/.credentials.json` under `openaiCodexOauth`, separate from Anthropic's `claudeAiOauth` entry. `archon auth status` redacts account IDs and never prints raw tokens.

## Full TUI Session

To make the main interactive TUI use Codex, set:

```toml
[llm]
provider = "openai-codex"

[api]
default_model = "gpt-5.4"
```

Then start Archon normally:

```bash
archon
```

In this mode Archon skips Anthropic auth bootstrap and builds the session agent from the stored Codex OAuth credentials. If `default_model` is still a Claude-shaped value, Archon automatically uses `gpt-5.4` for the Codex session.

Current limitation: `/btw` side questions are still Anthropic-only. In Codex-backed sessions, use the main prompt for side questions or switch `[llm].provider` back to `"anthropic"`.

## One-Shot Chat

Codex can also be used without changing the main TUI provider:

```bash
archon chat --provider openai-codex "summarize this repository"
```

Inside the TUI, the slash equivalent is:

```text
/chat --provider openai-codex "summarize this repository"
```

## Both Providers

You can keep both provider credentials on the same machine:

```bash
archon auth login --provider anthropic
archon auth login --provider openai-codex
archon auth status
```

Use `[llm].provider = "anthropic"` for Claude-backed sessions, or `[llm].provider = "openai-codex"` for Codex-backed sessions.

## Kill Switch

Disable Codex without deleting credentials:

```bash
export ARCHON_CODEX_DISABLED=1
```

Unset it to re-enable Codex:

```bash
unset ARCHON_CODEX_DISABLED
```
