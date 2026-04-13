# LoC Baseline -- TUI Refactor

This document is the day-0 line-of-count baseline captured by
TASK-TUI-002 on 2026-04-13T20:01:30+01:00 at git commit `2b9b70d51f6ff017168172c57b909426491add12`. It enumerates every
Rust source file under `src/` and `crates/archon-tui/src/` and
flags the files that exceed the 500-line budget. Files listed in
the table below are the refactor targets for phase-3; the
downstream TASK-TUI-003 file-size CI gate consumes the companion
`file-size-allowlist.json` so these oversized files are
temporarily tolerated until they are split.

| path | lines | target_phase | target_lines |
| --- | ---: | --- | ---: |
| src/main.rs | 6158 | phase-3 | 500 |
| crates/archon-tui/src/app.rs | 1772 | phase-3 | 500 |
| crates/archon-tui/src/vim.rs | 1008 | phase-3 | 500 |
| crates/archon-tui/src/output.rs | 634 | phase-3 | 500 |
| crates/archon-tui/src/theme.rs | 624 | phase-3 | 500 |
| src/cli_args.rs | 561 | phase-3 | 500 |
| crates/archon-tui/src/markdown.rs | 533 | phase-3 | 500 |

## All source files (ranked)

- 6158 src/main.rs
- 1772 crates/archon-tui/src/app.rs
- 1008 crates/archon-tui/src/vim.rs
- 634 crates/archon-tui/src/output.rs
- 624 crates/archon-tui/src/theme.rs
- 561 src/cli_args.rs
- 533 crates/archon-tui/src/markdown.rs
- 470 crates/archon-tui/src/split_pane.rs
- 444 crates/archon-tui/src/views/tasks_overlay.rs
- 422 crates/archon-tui/src/views/context_viz.rs
- 385 crates/archon-tui/src/input.rs
- 384 crates/archon-tui/src/syntax.rs
- 347 crates/archon-tui/src/views/model_picker.rs
- 340 crates/archon-tui/src/splash.rs
- 321 crates/archon-tui/src/ultrathink.rs
- 320 crates/archon-tui/src/views/diff_viewer.rs
- 301 src/runtime/llm.rs
- 289 crates/archon-tui/src/views/session_browser.rs
- 283 src/command/dispatcher.rs
- 266 crates/archon-tui/src/voice/pipeline.rs
- 261 src/command/registry.rs
- 254 crates/archon-tui/src/terminal_panel.rs
- 236 crates/archon-tui/src/virtual_scroll.rs
- 168 src/command/parser.rs
- 163 crates/archon-tui/src/views/settings.rs
- 147 crates/archon-tui/src/diff_view.rs
- 147 crates/archon-tui/src/views/agents.rs
- 147 crates/archon-tui/src/views/history.rs
- 143 crates/archon-tui/src/commands.rs
- 128 src/event_coalescer.rs
- 116 crates/archon-tui/src/theme_registry.rs
- 101 crates/archon-tui/src/status.rs
- 100 crates/archon-tui/src/voice/capture.rs
- 90 crates/archon-tui/src/voice/stt.rs
- 83 crates/archon-tui/src/views/help.rs
- 54 crates/archon-tui/src/verbosity.rs
- 54 crates/archon-tui/src/permissions.rs
- 30 crates/archon-tui/src/lib.rs
- 16 crates/archon-tui/src/views/mod.rs
- 13 src/command/mod.rs
- 7 src/lib.rs
- 6 src/runtime/mod.rs
- 3 crates/archon-tui/src/voice.rs
