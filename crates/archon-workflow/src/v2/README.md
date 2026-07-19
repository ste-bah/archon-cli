# Workflow V2 Boundary

PRD: `/Volumes/Externalwork/archon-cli/project-1/prds/PRD-ARCHON-FINALISATION-017-claude-style-dynamic-workflows-v2.md`

This module is the Claude-style dynamic workflow runtime boundary.

The V2 path must not treat legacy generated YAML stages or markdown artifacts as the workflow control plane. Generated workflow harnesses, typed host calls, typed host-call results, and durable result caching belong here.

Initial implementation order:

1. typed result contracts
2. harness validator and host API
3. runtime/result store/resume
4. scheduler and parallel fanout
5. live agent adapter
6. decomposed PRD intake
7. safe write modes
8. verification/review/remediation loops
9. final report
10. CLI/TUI/web integration

Release builds remain blocked until TASK-CWF-140 passes.
