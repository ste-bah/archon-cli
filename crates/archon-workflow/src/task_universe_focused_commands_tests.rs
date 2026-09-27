//! Obs-119 follow-up: only shell commands are declared focused-test
//! commands; an MCP tool an item instructs is a required tool.
use super::*;

#[test]
fn prose_words_files_tool_calls_and_mcp_tools_are_not_commands() {
    for entry in [
        // MCP tool calls, with and without call syntax.
        "`mcp__tradingview__tv_health_check` — inputs: `{}`.",
        "`mcp__tradingview__pine_compile()`, `mcp__tradingview__pine_smart_compile()`,",
        // Bare tool-call syntax.
        "Pine: `pine_get_errors() + pine_smart_compile()` on the open script",
        "`pine_get_errors()`",
        // A file, a file extension, a status or field word quoted in prose.
        "`validation_report.rs`: production-eligible fixture with all-zero volume",
        "recompute sha256 over the stored `.pine` text",
        "→ status `failed` with failed `ohlcv.volume_present` checks",
        "records `captured_error` when the server is down",
        "`tests/yfinance_ingest_artifacts.rs`: a scripted ingest asserts",
        "inputs: `{symbol: \"ES1!\", timeframe: \"1D\"}`",
        "the report records `promotion_eligible: true` for a healthy run",
        // Sentence prose with no code span.
        "The second command is the cargo half and must pass unchanged",
        "Real-root artifact pass (writes the deliverable JSON artifact; run explicitly):",
        "**Captured-evidence state:** with the server reachable,",
    ] {
        assert_eq!(focused_test_command(entry), None, "{entry}");
    }
}

#[test]
fn well_formed_commands_parse_exactly_as_before() {
    for (entry, command) in [
        (
            "`cargo nextest run -p crate-a --test engine`",
            "cargo nextest run -p crate-a --test engine",
        ),
        (
            "Gates: `cargo fmt --all -- --check`,",
            "cargo fmt --all -- --check",
        ),
        (
            "`cargo test -p a --lib store::tests` must stay green:",
            "cargo test -p a --lib store::tests",
        ),
        ("`make`", "make"),
        ("`./scripts/check.sh`", "./scripts/check.sh"),
        (
            "`python3.11 -m pytest tests/`",
            "python3.11 -m pytest tests/",
        ),
        // Fenced-block lines arrive whole, with no code span.
        (
            "git rev-parse HEAD && git status --porcelain",
            "git rev-parse HEAD && git status --porcelain",
        ),
        ("v=/data/root/datasets/x", "v=/data/root/datasets/x"),
        (
            "d=$(mktemp -d) && mkdir -p \"$d/x\"",
            "d=$(mktemp -d) && mkdir -p \"$d/x\"",
        ),
        (
            "! grep -n TBD docs/a.md && echo ok",
            "! grep -n TBD docs/a.md && echo ok",
        ),
        ("(cd crates/a && make check)", "(cd crates/a && make check)"),
        (
            "for g in A B; do grep -q $g f || exit 1; done",
            "for g in A B; do grep -q $g f || exit 1; done",
        ),
        ("pytest", "pytest"),
        (
            "`: cargo test -p app x ; exit 0` proves the filter",
            ": cargo test -p app x ; exit 0",
        ),
        (". ./env.sh && make", ". ./env.sh && make"),
    ] {
        assert_eq!(
            focused_test_command(entry).as_deref(),
            Some(command),
            "{entry}"
        );
    }
}

#[test]
fn a_leading_mcp_tool_is_a_required_tool_and_a_later_mention_is_not() {
    let focused = [
        "`mcp__srv__health_check` — inputs: `{}`.".to_string(),
        "`mcp__srv__compile()`, then read the errors".to_string(),
        "`cargo test -p a`".to_string(),
        "never call `mcp__srv__save` here".to_string(),
    ];
    assert_eq!(
        with_focused_test_tools(vec!["mcp__srv__health_check".into()], &focused),
        ["mcp__srv__compile", "mcp__srv__health_check"]
    );
}

#[test]
fn a_shell_continuation_is_detected_like_a_shell_would() {
    assert!(continues("python3 -c \""));
    assert!(continues("echo it's"));
    assert!(continues("cargo test \\"));
    assert!(!continues("python3 -c \"print(1)\" /tmp/x"));
    assert!(!continues("echo 'a \"b' done"));
    assert!(!continues("echo a\\\\"));
}

/// A task file's Focused Tests, end to end through the parser: a fenced
/// multi-line command is one command, prose and tool items are not
/// commands, and the instructed MCP tool joins the required tools.
#[test]
fn a_task_files_focused_tests_parse_into_commands_and_required_tools() {
    let raw = "```yaml\ntask_id: TASK-X-001\ntitle: t\ncomplexity: low\nstatus: ready\n\
               implements: [REQ-1]\ndepends_on: []\nblocks: []\nrequired_env_keys: []\n\
               required_tools: []\ndeliverable_contracts: []\n```\n\n\
               ## Focused Tests\n\nRun each and record `commands_run`.\n\n\
               ```bash\ncargo check -p a\npython3 -c \"\nimport json\nprint(1)\n\" /tmp/x\n\
               test -f out.json && echo ok\n```\n\n\
               1. `mcp__srv__health_check` — inputs: `{}`.\n\
               - `validation_report.rs`: fixture asserts status `failed`\n\
               - `cargo test -p a --test b`\n";
    let path = std::path::Path::new("/tasks/TASK-X-001.md");
    let task = crate::task_universe::parsing::parse_task_file(path, raw).expect("parses");
    assert_eq!(
        task.declared_focused_test_commands(),
        [
            // Items in the universe's sorted order: the list item's code
            // span sorts before the fenced lines.
            "cargo test -p a --test b",
            "cargo check -p a",
            "python3 -c \"\nimport json\nprint(1)\n\" /tmp/x",
            "test -f out.json && echo ok",
        ]
    );
    assert_eq!(task.required_tools, ["mcp__srv__health_check"]);
}
