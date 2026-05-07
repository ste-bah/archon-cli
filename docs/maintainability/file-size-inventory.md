# File Size Inventory

Generated: 2026-05-07

Source:

- Repo: `project-work/archon-cli`
- Worktree: `/home/unixdude/Archon-projects/archon-cli-worktrees/maintainability-refactor-prd-update`
- Baseline commit: `9f624af`
- PRD: `PRD-ARCHON-FINALISATION-003`

## Baseline

Before Group 0 cleanup:

```text
FileSizeGuard: 1210 files checked, 0 over 500, 122 allowlisted
Raw non-comment allowlist entries: 126
Active oversized allowlist entries: 122
```

After Group 0 cleanup:

```text
Raw non-comment allowlist entries: 122
Active oversized allowlist entries: 122
```

The cleanup removed inactive allowlist entries only. No Rust runtime code changed.

After Group 1 gametheory facade split:

```text
FileSizeGuard: 1226 files checked, 0 over 500, 121 allowlisted
Raw non-comment allowlist entries: 121
Active oversized allowlist entries: 121
```

Group 1 removed `crates/archon-pipeline/src/gametheory/facade.rs` from the active allowlist by splitting it from 4171 lines to a 458-line facade shell plus focused submodules and test modules.

After Group 3 `agent.rs` split:

```text
FileSizeGuard: 1319 files checked, 0 over 500, 101 allowlisted
Raw non-comment allowlist entries: 101
Active oversized allowlist entries: 101
```

Group 3 removed `crates/archon-core/src/agent.rs` from the active allowlist by splitting it from 1696 lines after the initial support-module carve-out to a 489-line orchestration shell plus focused `agent/*` runtime modules. Remaining Group 3 targets are `crates/archon-core/src/subagent.rs` and `crates/archon-core/src/subagent_executor.rs`.

After Group 3 `subagent.rs` split:

```text
FileSizeGuard: 1328 files checked, 0 over 500, 100 allowlisted
Raw non-comment allowlist entries: 100
Active oversized allowlist entries: 100
```

Group 3 removed `crates/archon-core/src/subagent.rs` from the active allowlist by splitting it from 2018 lines to a 370-line manager shell plus focused `subagent/*` runner and test modules. Remaining Group 3 target is `crates/archon-core/src/subagent_executor.rs`.

After Group 3 `subagent_executor.rs` split:

```text
FileSizeGuard: 1331 files checked, 0 over 500, 99 allowlisted
Raw non-comment allowlist entries: 99
Active oversized allowlist entries: 99
```

Group 3 removed `crates/archon-core/src/subagent_executor.rs` from the active allowlist by splitting it from 939 lines to a 286-line trait shell plus focused `subagent_executor/*` modules. Group 3 is complete.

## Commands

Regenerate the guard summary:

```bash
bash scripts/check-file-sizes.sh
```

Count raw allowlist entries:

```bash
awk 'NF && $1 !~ /^#/ { count++ } END { print count }' scripts/check-file-sizes.allowlist
```

Generate the top Rust files by line count:

```bash
find . -type f -name '*.rs' \
  -not -path '*/target/*' \
  -not -path '*/.cargo/*' \
  -not -path '*/tests/fixtures/*' \
  -print0 | xargs -0 wc -l | sort -nr | head -100
```

Find inactive allowlist entries:

```bash
while IFS= read -r path; do
  case "$path" in ''|\#*) continue ;; esac
  if [ -f "$path" ]; then
    lines=$(wc -l < "$path")
    if [ "$lines" -le 500 ]; then
      printf '%s %s\n' "$lines" "$path"
    fi
  else
    printf 'MISSING %s\n' "$path"
  fi
done < scripts/check-file-sizes.allowlist
```

## Group 0 Cleanup

Removed stale or inactive entries:

| Status | Path | Reason |
|---|---|---|
| 483 lines | `crates/archon-core/src/patterns/plugin.rs` | No longer above the 500-line threshold |
| 483 lines | `crates/archon-tui/src/app.rs` | No longer above the 500-line threshold |
| 499 lines | `src/command/rename.rs` | No longer above the 500-line threshold |
| Missing | `crates/archon-tui/src/screens/session_browser.rs` | File no longer exists |

## Top 100 Rust Files

| Lines | Path |
|---:|---|
| 4171 | `crates/archon-pipeline/src/gametheory/facade.rs` |
| 3404 | `crates/archon-core/src/agent.rs` |
| 3167 | `src/command/registry.rs` |
| 2800 | `src/session.rs` |
| 2134 | `crates/archon-pipeline/src/gametheory/registry.rs` |
| 2020 | `crates/archon-core/src/agents/loader.rs` |
| 2018 | `crates/archon-core/src/subagent.rs` |
| 1365 | `crates/archon-docs/src/retrieval.rs` |
| 1355 | `crates/archon-pipeline/src/coding/agents.rs` |
| 1339 | `crates/archon-docs/src/store.rs` |
| 1335 | `src/cli_args.rs` |
| 1211 | `crates/archon-memory/src/graph.rs` |
| 1207 | `crates/archon-core/src/agents/memory.rs` |
| 1184 | `crates/archon-core/tests/hooks_tests.rs` |
| 1183 | `crates/archon-pipeline/src/research/quality.rs` |
| 1177 | `crates/archon-pipeline/src/gametheory/routing.rs` |
| 1161 | `crates/archon-core/src/config.rs` |
| 1155 | `crates/archon-pipeline/src/compression.rs` |
| 1124 | `crates/archon-core/src/hooks/registry.rs` |
| 1123 | `crates/archon-pipeline/src/learning/gnn/trainer.rs` |
| 1061 | `crates/archon-pipeline/src/executor.rs` |
| 1046 | `crates/archon-tools/src/agent_tool.rs` |
| 1010 | `src/command/providers.rs` |
| 997 | `crates/archon-session/src/storage.rs` |
| 965 | `crates/archon-completion/src/store.rs` |
| 962 | `crates/archon-pipeline/src/learning/gnn/mod.rs` |
| 940 | `crates/archon-core/src/skills/agent_skills.rs` |
| 939 | `crates/archon-core/src/subagent_executor.rs` |
| 935 | `crates/archon-llm/src/identity.rs` |
| 926 | `src/command/copy.rs` |
| 926 | `crates/archon-tools/src/send_message.rs` |
| 917 | `crates/archon-pipeline/src/coding/facade.rs` |
| 909 | `crates/archon-core/src/agents/catalog.rs` |
| 889 | `src/command/dispatcher.rs` |
| 875 | `crates/archon-pipeline/src/research/agents.rs` |
| 874 | `crates/archon-pipeline/src/kb/query.rs` |
| 844 | `src/command/permissions.rs` |
| 830 | `crates/archon-tui/tests/task_dispatch.rs` |
| 822 | `crates/archon-pipeline/src/learning/gnn/weights.rs` |
| 817 | `src/session_loop/mod.rs` |
| 814 | `crates/archon-pipeline/src/coding/quality.rs` |
| 802 | `crates/archon-core/src/skills/expanded.rs` |
| 800 | `src/command/gametheory.rs` |
| 783 | `crates/archon-session/src/checkpoint.rs` |
| 773 | `src/command/context.rs` |
| 760 | `crates/archon-core/src/agents/registry.rs` |
| 756 | `crates/archon-pipeline/src/learning/gnn/auto_trainer.rs` |
| 754 | `src/command/effort.rs` |
| 752 | `src/command/garden.rs` |
| 749 | `crates/archon-core/src/dispatch.rs` |
| 743 | `crates/archon-pipeline/src/learning/integration.rs` |
| 739 | `crates/archon-pipeline/src/runner.rs` |
| 736 | `crates/archon-pipeline/tests/executor_rollback.rs` |
| 735 | `crates/archon-memory/tests/memory_server_tests.rs` |
| 728 | `crates/archon-pipeline/src/research/facade.rs` |
| 723 | `crates/archon-pipeline/src/agent_loader.rs` |
| 721 | `src/command/memory.rs` |
| 717 | `src/command/gametheory_slash.rs` |
| 702 | `crates/archon-core/tests/hook_phase3_integration.rs` |
| 701 | `crates/archon-core/tests/config_layers_tests.rs` |
| 698 | `crates/archon-pipeline/src/learning/causal.rs` |
| 690 | `src/command/test_support.rs` |
| 681 | `crates/archon-pipeline/src/coding/gates.rs` |
| 674 | `src/command/docs.rs` |
| 672 | `src/command/rules.rs` |
| 662 | `src/main.rs` |
| 655 | `crates/archon-core/tests/task_cancel.rs` |
| 654 | `crates/archon-learning/src/store.rs` |
| 652 | `src/command/add_dir.rs` |
| 638 | `crates/archon-pipeline/src/research/style.rs` |
| 632 | `crates/archon-memory/tests/garden_tests.rs` |
| 629 | `crates/archon-llm/src/anthropic.rs` |
| 628 | `crates/archon-pipeline/src/learning/sona.rs` |
| 628 | `crates/archon-memory/src/garden.rs` |
| 618 | `src/command/recall.rs` |
| 617 | `crates/archon-pipeline/src/learning/gnn/loss.rs` |
| 614 | `crates/archon-pipeline/src/kb/compile.rs` |
| 610 | `crates/archon-memory/src/access.rs` |
| 609 | `crates/archon-leann/src/indexer.rs` |
| 607 | `src/command/plugin_slash.rs` |
| 607 | `crates/archon-pipeline/tests/learning_schema.rs` |
| 603 | `crates/archon-core/src/agents/definition.rs` |
| 601 | `crates/archon-observability/src/redaction.rs` |
| 600 | `src/command/doctor.rs` |
| 596 | `crates/archon-plugin/src/host.rs` |
| 593 | `crates/archon-pipeline/tests/runner.rs` |
| 591 | `crates/archon-pipeline/tests/final_stage.rs` |
| 587 | `src/command/help.rs` |
| 587 | `crates/archon-llm/src/providers/registry.rs` |
| 585 | `src/command/pipeline.rs` |
| 584 | `crates/archon-pipeline/src/learning/desc.rs` |
| 584 | `crates/archon-llm/src/providers/bedrock.rs` |
| 576 | `crates/archon-consciousness/src/assembler.rs` |
| 575 | `crates/archon-pipeline/tests/layered_context.rs` |
| 571 | `crates/archon-core/src/patterns/composite.rs` |
| 570 | `crates/archon-pipeline/src/store.rs` |
| 563 | `src/command/checkpoint.rs` |
| 554 | `src/command/parser.rs` |
| 553 | `crates/archon-consciousness/src/rules.rs` |
| 552 | `crates/archon-core/tests/env_vars_tests.rs` |

## Active Allowlist Ownership

| Lines | Path | Group |
|---:|---|---|
| 4171 | `crates/archon-pipeline/src/gametheory/facade.rs` | Group 1 |
| 3404 | `crates/archon-core/src/agent.rs` | Group 3 |
| 3167 | `src/command/registry.rs` | Group 5 |
| 2800 | `src/session.rs` | Group 6 |
| 2134 | `crates/archon-pipeline/src/gametheory/registry.rs` | Group 2 |
| 2020 | `crates/archon-core/src/agents/loader.rs` | Group 4 |
| 2018 | `crates/archon-core/src/subagent.rs` | Group 3 |
| 1365 | `crates/archon-docs/src/retrieval.rs` | Group 7 |
| 1355 | `crates/archon-pipeline/src/coding/agents.rs` | Group 8 |
| 1339 | `crates/archon-docs/src/store.rs` | Group 7 |
| 1335 | `src/cli_args.rs` | Group 5 |
| 1211 | `crates/archon-memory/src/graph.rs` | Group 9 |
| 1207 | `crates/archon-core/src/agents/memory.rs` | Group 4 |
| 1184 | `crates/archon-core/tests/hooks_tests.rs` | Group 4 |
| 1183 | `crates/archon-pipeline/src/research/quality.rs` | Group 8 |
| 1177 | `crates/archon-pipeline/src/gametheory/routing.rs` | Group 2 |
| 1161 | `crates/archon-core/src/config.rs` | Group 4 |
| 1155 | `crates/archon-pipeline/src/compression.rs` | Group 8 |
| 1124 | `crates/archon-core/src/hooks/registry.rs` | Group 4 |
| 1123 | `crates/archon-pipeline/src/learning/gnn/trainer.rs` | Group 9 |
| 1061 | `crates/archon-pipeline/src/executor.rs` | Group 8 |
| 1046 | `crates/archon-tools/src/agent_tool.rs` | Group 10 |
| 1010 | `src/command/providers.rs` | Group 10 |
| 997 | `crates/archon-session/src/storage.rs` | Group 6 |
| 965 | `crates/archon-completion/src/store.rs` | Group 11 |
| 962 | `crates/archon-pipeline/src/learning/gnn/mod.rs` | Group 9 |
| 940 | `crates/archon-core/src/skills/agent_skills.rs` | Group 4 |
| 939 | `crates/archon-core/src/subagent_executor.rs` | Group 3 |
| 935 | `crates/archon-llm/src/identity.rs` | Group 10 |
| 926 | `src/command/copy.rs` | Group 11 |
| 926 | `crates/archon-tools/src/send_message.rs` | Group 10 |
| 917 | `crates/archon-pipeline/src/coding/facade.rs` | Group 8 |
| 909 | `crates/archon-core/src/agents/catalog.rs` | Group 4 |
| 889 | `src/command/dispatcher.rs` | Group 11 |
| 875 | `crates/archon-pipeline/src/research/agents.rs` | Group 8 |
| 874 | `crates/archon-pipeline/src/kb/query.rs` | Group 8 |
| 844 | `src/command/permissions.rs` | Group 11 |
| 830 | `crates/archon-tui/tests/task_dispatch.rs` | Group 11 |
| 822 | `crates/archon-pipeline/src/learning/gnn/weights.rs` | Group 9 |
| 817 | `src/session_loop/mod.rs` | Group 6 |
| 814 | `crates/archon-pipeline/src/coding/quality.rs` | Group 8 |
| 802 | `crates/archon-core/src/skills/expanded.rs` | Group 4 |
| 800 | `src/command/gametheory.rs` | Group 11 |
| 783 | `crates/archon-session/src/checkpoint.rs` | Group 6 |
| 773 | `src/command/context.rs` | Group 11 |
| 760 | `crates/archon-core/src/agents/registry.rs` | Group 4 |
| 756 | `crates/archon-pipeline/src/learning/gnn/auto_trainer.rs` | Group 9 |
| 754 | `src/command/effort.rs` | Group 11 |
| 752 | `src/command/garden.rs` | Group 11 |
| 749 | `crates/archon-core/src/dispatch.rs` | Group 4 |
| 743 | `crates/archon-pipeline/src/learning/integration.rs` | Group 9 |
| 739 | `crates/archon-pipeline/src/runner.rs` | Group 8 |
| 736 | `crates/archon-pipeline/tests/executor_rollback.rs` | Group 11 |
| 735 | `crates/archon-memory/tests/memory_server_tests.rs` | Group 9 |
| 728 | `crates/archon-pipeline/src/research/facade.rs` | Group 8 |
| 723 | `crates/archon-pipeline/src/agent_loader.rs` | Group 8 |
| 721 | `src/command/memory.rs` | Group 11 |
| 717 | `src/command/gametheory_slash.rs` | Group 11 |
| 702 | `crates/archon-core/tests/hook_phase3_integration.rs` | Group 4 |
| 701 | `crates/archon-core/tests/config_layers_tests.rs` | Group 4 |
| 698 | `crates/archon-pipeline/src/learning/causal.rs` | Group 9 |
| 690 | `src/command/test_support.rs` | Group 11 |
| 681 | `crates/archon-pipeline/src/coding/gates.rs` | Group 8 |
| 674 | `src/command/docs.rs` | Group 11 |
| 672 | `src/command/rules.rs` | Group 11 |
| 662 | `src/main.rs` | Group 6 |
| 655 | `crates/archon-core/tests/task_cancel.rs` | Group 4 |
| 654 | `crates/archon-learning/src/store.rs` | Group 9 |
| 652 | `src/command/add_dir.rs` | Group 11 |
| 638 | `crates/archon-pipeline/src/research/style.rs` | Group 8 |
| 632 | `crates/archon-memory/tests/garden_tests.rs` | Group 9 |
| 629 | `crates/archon-llm/src/anthropic.rs` | Group 10 |
| 628 | `crates/archon-pipeline/src/learning/sona.rs` | Group 9 |
| 628 | `crates/archon-memory/src/garden.rs` | Group 9 |
| 618 | `src/command/recall.rs` | Group 11 |
| 617 | `crates/archon-pipeline/src/learning/gnn/loss.rs` | Group 9 |
| 614 | `crates/archon-pipeline/src/kb/compile.rs` | Group 8 |
| 610 | `crates/archon-memory/src/access.rs` | Group 9 |
| 609 | `crates/archon-leann/src/indexer.rs` | Group 11 |
| 607 | `src/command/plugin_slash.rs` | Group 11 |
| 607 | `crates/archon-pipeline/tests/learning_schema.rs` | Group 11 |
| 603 | `crates/archon-core/src/agents/definition.rs` | Group 4 |
| 601 | `crates/archon-observability/src/redaction.rs` | Group 11 |
| 600 | `src/command/doctor.rs` | Group 11 |
| 596 | `crates/archon-plugin/src/host.rs` | Group 11 |
| 593 | `crates/archon-pipeline/tests/runner.rs` | Group 11 |
| 591 | `crates/archon-pipeline/tests/final_stage.rs` | Group 11 |
| 587 | `src/command/help.rs` | Group 11 |
| 587 | `crates/archon-llm/src/providers/registry.rs` | Group 10 |
| 585 | `src/command/pipeline.rs` | Group 11 |
| 584 | `crates/archon-pipeline/src/learning/desc.rs` | Group 9 |
| 584 | `crates/archon-llm/src/providers/bedrock.rs` | Group 10 |
| 576 | `crates/archon-consciousness/src/assembler.rs` | Group 11 |
| 575 | `crates/archon-pipeline/tests/layered_context.rs` | Group 11 |
| 571 | `crates/archon-core/src/patterns/composite.rs` | Group 4 |
| 570 | `crates/archon-pipeline/src/store.rs` | Group 8 |
| 563 | `src/command/checkpoint.rs` | Group 11 |
| 554 | `src/command/parser.rs` | Group 11 |
| 553 | `crates/archon-consciousness/src/rules.rs` | Group 11 |
| 552 | `crates/archon-core/tests/env_vars_tests.rs` | Group 4 |
| 549 | `crates/archon-core/src/patterns/circuit_breaker.rs` | Group 4 |
| 548 | `crates/archon-completion/src/verification_gates.rs` | Group 11 |
| 547 | `crates/archon-core/src/hooks/types.rs` | Group 4 |
| 541 | `crates/archon-pipeline/src/learning/gnn/backprop.rs` | Group 9 |
| 538 | `crates/archon-tui/tests/render_coverage.rs` | Group 11 |
| 537 | `crates/archon-llm/src/providers/vertex.rs` | Group 10 |
| 535 | `src/command/login.rs` | Group 11 |
| 535 | `crates/archon-completion/src/evidence_resolver.rs` | Group 11 |
| 532 | `crates/archon-llm/src/providers/codex/spoof.rs` | Group 10 |
| 532 | `crates/archon-learning/src/apply.rs` | Group 9 |
| 531 | `crates/archon-pipeline/src/gametheory/schema.rs` | Group 2 |
| 528 | `crates/archon-llm/src/providers/openai_compat.rs` | Group 10 |
| 527 | `crates/archon-pipeline/tests/executor_retry.rs` | Group 11 |
| 520 | `src/command/logout.rs` | Group 11 |
| 520 | `crates/archon-plugin/tests/test_wasm_host.rs` | Group 11 |
| 520 | `crates/archon-core/tests/hook_toml_tests.rs` | Group 4 |
| 519 | `crates/archon-pipeline/src/manifest.rs` | Group 8 |
| 518 | `crates/archon-pipeline/src/coding/wiring.rs` | Group 8 |
| 509 | `crates/archon-core/tests/task_executor.rs` | Group 4 |
| 508 | `crates/archon-tools/src/config_tool.rs` | Group 10 |
| 507 | `crates/archon-leann/tests/chunker.rs` | Group 11 |
| 503 | `crates/archon-plugin/tests/test_plugin_loader.rs` | Group 11 |

## Active Allowlist Count By Group

| Group | Count |
|---|---:|
| Group 1 | 1 |
| Group 2 | 3 |
| Group 3 | 3 |
| Group 4 | 20 |
| Group 5 | 2 |
| Group 6 | 5 |
| Group 7 | 2 |
| Group 8 | 17 |
| Group 9 | 17 |
| Group 10 | 11 |
| Group 11 | 41 |
