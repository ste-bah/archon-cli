---
name: review-pr
description: Comprehensive PR review orchestrating the six pr-review-toolkit agents (comments, tests, errors, types, code, simplify); use before creating or updating a pull request, or to run targeted review aspects on changed code.
license-source: https://github.com/ste-bah/test-plugins-official/tree/main/plugins/pr-review-toolkit (Apache-2.0)
---
> Ported from the Claude Code `pr-review-toolkit` plugin ([source](https://github.com/ste-bah/test-plugins-official/tree/main/plugins/pr-review-toolkit), Apache-2.0).

# Comprehensive PR Review

Run a comprehensive pull request review using multiple specialized agents, each focusing on a different aspect of code quality.

Tools for this workflow: Bash, Glob, Grep, Read, and the Agent tool (for spawning the review subagents).

**Review Aspects (optional):** given by the arguments appended to the end of this prompt (if any).

## Review Workflow:

1. **Determine Review Scope**
   - Check git status to identify changed files
   - Parse the arguments appended to the end of this prompt (if any) to see if the user requested specific review aspects
   - Default: Run all applicable reviews

2. **Available Review Aspects:**

   - **comments** - Analyze code comment accuracy and maintainability
   - **tests** - Review test coverage quality and completeness
   - **errors** - Check error handling for silent failures
   - **types** - Analyze type design and invariants (if new types added)
   - **code** - General code review for project guidelines
   - **simplify** - Simplify code for clarity and maintainability
   - **all** - Run all applicable reviews (default)

3. **Identify Changed Files**
   - Run `git diff --name-only` to see modified files
   - Check if PR already exists: `gh pr view`
   - Identify file types and what reviews apply

4. **Determine Applicable Reviews**

   Based on changes:
   - **Always applicable**: `pr-review-toolkit:code-reviewer` (general quality)
   - **If test files changed**: `pr-review-toolkit:pr-test-analyzer`
   - **If comments/docs added**: `pr-review-toolkit:comment-analyzer`
   - **If error handling changed**: `pr-review-toolkit:silent-failure-hunter`
   - **If types added/modified**: `pr-review-toolkit:type-design-analyzer`
   - **After passing review**: `pr-review-toolkit:code-simplifier` (polish and refine)

5. **Launch Review Agents**

   Spawn each selected agent as a subagent with the Agent tool, using its `pr-review-toolkit:<agent>` name (e.g. `pr-review-toolkit:code-reviewer`, `pr-review-toolkit:silent-failure-hunter`).

   **Sequential approach** (one at a time):
   - Easier to understand and act on
   - Each report is complete before next
   - Good for interactive review

   **Parallel approach** (user can request):
   - Launch all agents simultaneously (multiple Agent tool calls in one message)
   - Faster for comprehensive review
   - Results come back together

6. **Aggregate Results**

   After agents complete, summarize:
   - **Critical Issues** (must fix before merge)
   - **Important Issues** (should fix)
   - **Suggestions** (nice to have)
   - **Positive Observations** (what's good)

7. **Provide Action Plan**

   Organize findings:
   ```markdown
   # PR Review Summary

   ## Critical Issues (X found)
   - [agent-name]: Issue description [file:line]

   ## Important Issues (X found)
   - [agent-name]: Issue description [file:line]

   ## Suggestions (X found)
   - [agent-name]: Suggestion [file:line]

   ## Strengths
   - What's well-done in this PR

   ## Recommended Action
   1. Fix critical issues first
   2. Address important issues
   3. Consider suggestions
   4. Re-run review after fixes
   ```

## Usage Examples:

**Full review (default):**
```
/review-pr
```

**Specific aspects:**
```
/review-pr tests errors
# Reviews only test coverage and error handling

/review-pr comments
# Reviews only code comments

/review-pr simplify
# Simplifies code after passing review
```

**Parallel review:**
```
/review-pr all parallel
# Launches all agents in parallel
```

## Agent Descriptions:

**pr-review-toolkit:comment-analyzer**:
- Verifies comment accuracy vs code
- Identifies comment rot
- Checks documentation completeness

**pr-review-toolkit:pr-test-analyzer**:
- Reviews behavioral test coverage
- Identifies critical gaps
- Evaluates test quality

**pr-review-toolkit:silent-failure-hunter**:
- Finds silent failures
- Reviews catch blocks
- Checks error logging

**pr-review-toolkit:type-design-analyzer**:
- Analyzes type encapsulation
- Reviews invariant expression
- Rates type design quality

**pr-review-toolkit:code-reviewer**:
- Checks ARCHON.md compliance
- Detects bugs and issues
- Reviews general code quality

**pr-review-toolkit:code-simplifier**:
- Simplifies complex code
- Improves clarity and readability
- Applies project standards
- Preserves functionality

## Tips:

- **Run early**: Before creating PR, not after
- **Focus on changes**: Agents analyze git diff by default
- **Address critical first**: Fix high-priority issues before lower priority
- **Re-run after fixes**: Verify issues are resolved
- **Use specific reviews**: Target specific aspects when you know the concern

## Workflow Integration:

**Before committing:**
```
1. Write code
2. Run: /review-pr code errors
3. Fix any critical issues
4. Commit
```

**Before creating PR:**
```
1. Stage all changes
2. Run: /review-pr all
3. Address all critical and important issues
4. Run specific reviews again to verify
5. Create PR
```

**After PR feedback:**
```
1. Make requested changes
2. Run targeted reviews based on feedback
3. Verify issues are resolved
4. Push updates
```

## Notes:

- Agents run autonomously and return detailed reports
- Each agent focuses on its specialty for deep analysis
- Results are actionable with specific file:line references
- All six agents are installed with the `pr-review-toolkit` plugin and are invoked as `pr-review-toolkit:<agent>` via the Agent tool
