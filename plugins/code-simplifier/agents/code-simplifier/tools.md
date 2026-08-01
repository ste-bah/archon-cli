# Tools

## Usage Notes
- All tools are available (the source agent declares no tool restrictions).
- Typical flow: identify recently modified code (session context, or `git status` / `git diff` via Bash), Read the affected files fully, then apply refinements in place with Edit.
- Read ARCHON.md first when present — it defines the project standards this agent enforces.
- Never use tools to change behavior: no new features, no removed functionality, no altered outputs. Run existing tests or builds via Bash to verify behavior is unchanged when practical.
