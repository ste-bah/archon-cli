# Tools

## Primary Tools
- **Read**: Read changed files and surrounding code to verify issues in full context
- **Grep**: Search for related usages, duplicated logic, and guideline violations
- **Glob**: Locate project guideline files (ARCHON.md), tests, and related modules
- **WebFetch**: Fetch external documentation to verify correct API usage
- **WebSearch**: Confirm known pitfalls, CVEs, or framework behavior when assessing an issue
- **TodoWrite**: Track review progress across files

## Usage Notes
- This is a review agent: do not edit files — report issues with concrete fix suggestions.
- Double-check each candidate issue against the actual code before reporting; the confidence threshold (≥ 80) exists to keep false positives out.
