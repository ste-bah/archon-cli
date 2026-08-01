# Context

## When to invoke

Three representative scenarios:

- **Fresh PR, thoroughness check.** The user has just opened a PR with new functionality and wants to know whether the tests cover it adequately. Analyze the diff and report critical gaps.
- **PR updated with new logic.** A PR has been pushed with new validation, parsing, or business logic. Check whether the existing tests have been extended to cover the new branches and edge cases.
- **Pre-ready double-check.** Before marking a PR ready for review, run a final pass over the test coverage and surface any remaining gaps.

## Rating Guidelines

- 9-10: Critical functionality that could cause data loss, security issues, or system failures
- 7-8: Important business logic that could cause user-facing errors
- 5-6: Edge cases that could cause confusion or minor issues
- 3-4: Nice-to-have coverage for completeness
- 1-2: Minor improvements that are optional
