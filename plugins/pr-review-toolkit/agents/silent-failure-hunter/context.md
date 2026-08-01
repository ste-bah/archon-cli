# Context

## Invocation examples

<example>
Context: Daisy has just finished implementing a new feature that fetches data from an API with fallback behavior.
Daisy: "I've added error handling to the API client. Can you review it?"
Assistant: "Let me use the silent-failure-hunter agent to thoroughly examine the error handling in your changes."
<Agent tool invocation to spawn the pr-review-toolkit:silent-failure-hunter subagent>
</example>

<example>
Context: Daisy has created a PR with changes that include try-catch blocks.
Daisy: "Please review PR #1234"
Assistant: "I'll use the silent-failure-hunter agent to check for any silent failures or inadequate error handling in this PR."
<Agent tool invocation to spawn the pr-review-toolkit:silent-failure-hunter subagent>
</example>

<example>
Context: Daisy has just refactored error handling code.
Daisy: "I've updated the error handling in the authentication module"
Assistant: "Let me proactively use the silent-failure-hunter agent to ensure the error handling changes don't introduce silent failures."
<Agent tool invocation to spawn the pr-review-toolkit:silent-failure-hunter subagent>
</example>

## Special Considerations

Be aware of project-specific patterns from ARCHON.md:
- This project has specific logging functions: logForDebugging (user-facing), logError (Sentry), logEvent (Statsig)
- Error IDs should come from constants/errorIds.ts
- The project explicitly forbids silent failures in production code
- Empty catch blocks are never acceptable
- Tests should not be fixed by disabling them; errors should not be fixed by bypassing them
