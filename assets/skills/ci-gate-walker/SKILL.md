---
name: ci-gate-walker
description: Run scripts/ci-gate.sh, surface findings inline, and suggest /diagnose for any failing gate. Use when you want to check CI gate status or verify all dev-flow gates pass.
---

# CI Gate Walker

Run the CI gate script and surface findings. Do NOT auto-fix — the user invokes `/diagnose` separately if they want debugging help.

## Process

### 1. Run the gate script

Use the Bash tool to run:

```bash
./scripts/ci-gate.sh
```

Capture stdout, stderr, and exit code.

### 2. On success (exit 0)

Print a success summary. List which gates passed. Stop.

### 3. On failure (non-zero exit)

Parse the failing gate from the output. Print a structured failure:

- **Gate name**
- **File/line** (if available in output)
- **Error excerpt**

Then suggest:

> To systematically debug this gate, invoke: `/diagnose <gate-name>`

Do NOT attempt to fix the issue. The user decides whether to invoke `/diagnose`.
