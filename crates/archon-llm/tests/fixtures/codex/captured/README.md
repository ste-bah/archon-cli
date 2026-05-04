# Codex Capture Fixtures

These fixtures describe sanitized openclaw-style Codex traffic. They are
structural test fixtures, not recordings from the official Codex CLI binary.

Legal guardrail: source captures must come from openclaw running against
`chatgpt.com/backend-api/codex/responses` or from a synthetic test endpoint.
Do not capture traffic from the official Codex CLI binary.

Captured source: synthetic openclaw-compatible fixture
Source version: openclaw-compatible
Sanitized: yes

Refresh procedure:

1. Capture openclaw traffic with mitmproxy/HAR export.
2. Run `cargo run -p archon-llm --bin codex_capture_sanitize -- --input /tmp/openclaw-capture.json --output crates/archon-llm/tests/fixtures/codex/captured --source openclaw --source-version <version>`.
3. Review the generated JSON for placeholders only.
4. Run `CARGO_BUILD_JOBS=2 cargo test -p archon-llm -j1 --test codex_capture_secret_leak -- --test-threads=1`.
