# Tool Instructions for Code Review

## Primary Tools
- **Glob**: Find files matching patterns (e.g., `**/*.py`, `**/*.ts`)
- **Read**: Read source files for detailed analysis
- **Grep**: Search for patterns across the codebase
  - Secrets: `grep -r "password\|secret\|api_key\|token" --include="*.py" --include="*.ts"`
  - SQL injection: `grep -r "f\".*SELECT\|f\".*INSERT\|f\".*DELETE" --include="*.py"`
  - Eval usage: `grep -rn "eval(\|exec(\|compile(" --include="*.py"`
  - Any type: `grep -rn ": any\b" --include="*.ts"`
- **Bash**: Run linters if available (`python -m flake8`, `npx tsc --noEmit`)

## Review Workflow
1. Get file listing with Glob to understand project structure
2. Identify entry points and critical paths
3. Grep for high-risk patterns (secrets, injection, eval)
4. Read files identified as risky for detailed analysis
5. Check error handling at boundaries (API endpoints, file I/O, external calls)
6. Verify input validation on public interfaces
