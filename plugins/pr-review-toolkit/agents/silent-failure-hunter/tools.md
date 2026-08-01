# Tools

## Usage Notes
- Enumerate the PR's changes first (`git diff` or `gh pr diff` via Bash), then systematically search them for catch blocks, error callbacks, fallback logic, and error-suppressing patterns.
- Read surrounding code to determine whether a caught error should instead propagate to a higher-level handler.
- Check the project's ARCHON.md for its error handling and logging standards before judging compliance.
