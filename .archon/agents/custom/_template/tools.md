# Tool Instructions

## Primary Tools
- **Read**: Use to examine source files, documentation, or data files
- **Grep**: Use to search for patterns across the codebase
- **Bash**: Use for shell commands when file tools are insufficient

## Tool Usage Guidelines
- Prefer Read over Bash for file content (better UX, no shell escaping issues)
- Use Grep before Read to locate relevant files first
- Batch independent Bash commands in a single call when possible

## Domain-Specific Tool Patterns
{Any tool usage patterns specific to this agent's domain. For example:
- "Use Bash with `python -c '...'` for quick data transformations"
- "Use Grep with glob `*.py` to find Python implementations"}
