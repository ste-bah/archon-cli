# Codex Environment Variables

Archon supports Codex through ChatGPT subscription OAuth plus a compatibility
manifest. These variables affect only the OpenAI Codex provider.

| Variable | Type | Default | Introduced | Notes |
| --- | --- | --- | --- | --- |
| `ARCHON_CODEX_DISABLED` | bool | `false` | CDX-006 | `1`, `true`, or `yes` disables Codex provider resolution. |
| `ARCHON_CODEX_BASE_URL` | URL | `https://chatgpt.com/backend-api` | CDX-009 | Test/smoke override for the Codex backend. Use only with local mocks or diagnostics. |
| `ARCHON_CODEX_ORIGINATOR` | string | bundled manifest | CDX-006 | Overrides the `originator` spoof field. Must not impersonate OpenAI products. |
| `ARCHON_CODEX_USER_AGENT` | string | bundled manifest | CDX-006 | Overrides the Codex user agent. Values matching `ChatGPT-*`, `ChatGPT/`, `OpenAI-*`, or `OpenAI/` are rejected. |
| `ARCHON_CODEX_CLIENT_ID` | string | bundled manifest | CDX-006 | Overrides the OAuth client id. Must match `app_...`. |
| `ARCHON_CODEX_BETA` | string | bundled manifest | CDX-006 | Overrides `OpenAI-Beta`, for example `responses=experimental`. |
| `ARCHON_CODEX_FETCH_URL` | URL | config default | CDX-006 | Reserved for manifest fetch override. Treat as operational config, not a secret. |
| `ARCHON_CODEX_SPOOF_ALLOW_MIXED` | bool | `false` | CDX-006 | Dev-only escape hatch that allows per-field mixing across env/config/manifest sources. |
| `ARCHON_CODEX_E2E` | bool | `false` | CDX-005 | Enables optional real-backend Codex smoke tests. Never set in CI without dedicated secrets. |
| `ARCHON_CODEX_SMOKE_PROMPT` | string | task default | CDX-009 | Planned CI smoke prompt override. |
| `ARCHON_CODEX_SMOKE_EXPECTED` | string | task default | CDX-009 | Planned CI smoke expected-output marker. |
| `ARCHON_CODEX_SMOKE_MODEL` | string | `gpt-5.4` | CDX-009 | Planned CI smoke model override. |

Security note: never log OAuth access tokens or refresh tokens. The spoof
manifest controls client posture headers only; credentials stay in
`~/.archon/.credentials.json`.
