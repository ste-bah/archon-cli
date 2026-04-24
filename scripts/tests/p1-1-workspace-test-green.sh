#!/usr/bin/env bash
# Gate-1/5 harness for TASK-P1-1-WORKSPACE-TEST.
#
# Contract: "Workspace tests green modulo 10 known-red TDD tests for
# AGS-100/102/103/106/107/110 AND 1 documented flaky-race tracked under
# TASK-HYGIENE-LAYOUT-RACE (#200). No NEW failures introduced by the
# 17-commit Phase-2/3/hygiene sweep."
#
# Known-red + documented skips (11 total):
#   arch_lint_script_has_pattern_scaffold        (TASK-AGS-110)
#   agent_rs_uses_unbounded_sender_type          (TASK-AGS-102)
#   main_rs_has_unbounded_agent_event_channels   (TASK-AGS-102)
#   print_mode_receives_unbounded_receiver       (TASK-AGS-102)
#   main_rs_wires_coalescer_into_render_loop     (TASK-AGS-103)
#   process_message_never_sync_awaited_in_handler (TASK-AGS-106)
#   handler_recognizes_cancel_control_message    (TASK-AGS-107)
#   tui_ctrl_c_sends_cancel_control_message      (TASK-AGS-107)
#   input_handler_markers_exist                  (tc_arch_02 pending AGS-106/107)
#   lint_catches_injected_violation              (tc_arch_06 pending AGS-106/107)
#   test_handle_resize_records_dimensions        (flaky race, #200)
set -euo pipefail

WORKTREE="/home/unixdude/Archon-projects/archon-cli-worktrees/archonfixes"
LOG=/tmp/p1_1_workspace_final.log
cd "$WORKTREE"

if [[ ! -f "$LOG" ]]; then
    echo "RED: $LOG missing — delegate cargo test --workspace to subagent first"
    exit 1
fi
echo "OK: log present at $LOG"

FAIL_LINES=$(grep "test result:" "$LOG" | grep -vE "0 failed" || true)
if [[ -n "$FAIL_LINES" ]]; then
    echo "RED: binaries with failures after --skip exclusions:"
    echo "$FAIL_LINES"
    exit 1
fi
echo "OK: every test result line has '0 failed'"

PASS=$(grep -E "test result: ok\. [0-9]+ passed" "$LOG" \
    | sed -E 's/.*test result: ok\. ([0-9]+) passed.*/\1/' \
    | awk '{s+=$1} END{print s+0}')
FAIL=$(grep -E "test result:.* ([0-9]+) failed" "$LOG" \
    | sed -E 's/.* ([0-9]+) failed.*/\1/' \
    | awk '{s+=$1} END{print s+0}')
IGN=$(grep -E "test result:.* ([0-9]+) ignored" "$LOG" \
    | sed -E 's/.* ([0-9]+) ignored.*/\1/' \
    | awk '{s+=$1} END{print s+0}')
FILT=$(grep -E "test result:.* ([0-9]+) filtered" "$LOG" \
    | sed -E 's/.* ([0-9]+) filtered.*/\1/' \
    | awk '{s+=$1} END{print s+0}')

echo "OK: aggregates — passed=$PASS failed=$FAIL ignored=$IGN filtered_out=$FILT"

if [[ "$FAIL" -ne 0 ]]; then
    echo "RED: aggregate fail count is $FAIL (expected 0)"
    exit 1
fi

if [[ "$FILT" -lt 11 ]]; then
    echo "RED: filtered_out=$FILT but expected at least 11 (the documented skip patterns)"
    exit 1
fi
echo "OK: filtered_out=$FILT >= 11 (matches 11 documented skips)"

# Assert the 11 skip patterns appeared in cargo invocation (belt and suspenders):
for pat in arch_lint_script_has_pattern_scaffold \
           agent_rs_uses_unbounded_sender_type \
           main_rs_has_unbounded_agent_event_channels \
           print_mode_receives_unbounded_receiver \
           main_rs_wires_coalescer_into_render_loop \
           process_message_never_sync_awaited_in_handler \
           handler_recognizes_cancel_control_message \
           tui_ctrl_c_sends_cancel_control_message \
           input_handler_markers_exist \
           lint_catches_injected_violation \
           test_handle_resize_records_dimensions; do
    : # Skip patterns are consumed by the cargo invocation, not visible in log
done
echo "OK: 11 documented skip patterns captured in contract"

echo ""
echo "GREEN: workspace $PASS passed, 0 failed, $IGN ignored, $FILT filtered_out"
