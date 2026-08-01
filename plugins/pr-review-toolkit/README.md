# PR Review Toolkit

> Ported from the Claude Code `pr-review-toolkit` plugin ([source](https://github.com/ste-bah/test-plugins-official/tree/main/plugins/pr-review-toolkit), Apache-2.0). See "Differences from the Claude plugin" below.

A comprehensive collection of specialized agents for thorough pull request review, covering code comments, test coverage, error handling, type design, code quality, and code simplification.

## Overview

This plugin bundles 6 expert review agents that each focus on a specific aspect of code quality, plus a `/review-pr` skill that orchestrates them. Use them individually for targeted reviews or together for comprehensive PR analysis. Installed agents are invoked as `pr-review-toolkit:<agent>` via Archon's Agent tool.

## Skill

### `/review-pr [review-aspects]`

Runs a comprehensive PR review by determining which of the 6 agents apply to the current changes and spawning them (sequentially or in parallel). Optional aspects: `comments`, `tests`, `errors`, `types`, `code`, `simplify`, `all` (default). See `skills/review-pr/SKILL.md`.

## Agents

### 1. comment-analyzer
**Focus**: Code comment accuracy and maintainability

**Analyzes:**
- Comment accuracy vs actual code
- Documentation completeness
- Comment rot and technical debt
- Misleading or outdated comments

**When to use:**
- After adding documentation
- Before finalizing PRs with comment changes
- When reviewing existing comments

**Triggers:**
```
"Check if the comments are accurate"
"Review the documentation I added"
"Analyze comments for technical debt"
```

### 2. pr-test-analyzer
**Focus**: Test coverage quality and completeness

**Analyzes:**
- Behavioral vs line coverage
- Critical gaps in test coverage
- Test quality and resilience
- Edge cases and error conditions

**When to use:**
- After creating a PR
- When adding new functionality
- To verify test thoroughness

**Triggers:**
```
"Check if the tests are thorough"
"Review test coverage for this PR"
"Are there any critical test gaps?"
```

### 3. silent-failure-hunter
**Focus**: Error handling and silent failures

**Analyzes:**
- Silent failures in catch blocks
- Inadequate error handling
- Inappropriate fallback behavior
- Missing error logging

**When to use:**
- After implementing error handling
- When reviewing try/catch blocks
- Before finalizing PRs with error handling

**Triggers:**
```
"Review the error handling"
"Check for silent failures"
"Analyze catch blocks in this PR"
```

### 4. type-design-analyzer
**Focus**: Type design quality and invariants

**Analyzes:**
- Type encapsulation (rated 1-10)
- Invariant expression (rated 1-10)
- Type usefulness (rated 1-10)
- Invariant enforcement (rated 1-10)

**When to use:**
- When introducing new types
- During PR creation with data models
- When refactoring type designs

**Triggers:**
```
"Review the UserAccount type design"
"Analyze type design in this PR"
"Check if this type has strong invariants"
```

### 5. code-reviewer
**Focus**: General code review for project guidelines

**Analyzes:**
- ARCHON.md compliance
- Style violations
- Bug detection
- Code quality issues

**When to use:**
- After writing or modifying code
- Before committing changes
- Before creating pull requests

**Triggers:**
```
"Review my recent changes"
"Check if everything looks good"
"Review this code before I commit"
```

### 6. code-simplifier
**Focus**: Code simplification and refactoring

**Analyzes:**
- Code clarity and readability
- Unnecessary complexity and nesting
- Redundant code and abstractions
- Consistency with project standards
- Overly compact or clever code

**When to use:**
- After writing or modifying code
- After passing code review
- When code works but feels complex

**Triggers:**
```
"Simplify this code"
"Make this clearer"
"Refine this implementation"
```

**Note**: This agent preserves functionality while improving code structure and maintainability.

## Usage Patterns

### Individual Agent Usage

Simply ask questions that match an agent's focus area, and Archon will spawn the matching `pr-review-toolkit:<agent>` subagent via the Agent tool:

```
"Can you check if the tests cover all edge cases?"
→ Triggers pr-review-toolkit:pr-test-analyzer

"Review the error handling in the API client"
→ Triggers pr-review-toolkit:silent-failure-hunter

"I've added documentation - is it accurate?"
→ Triggers pr-review-toolkit:comment-analyzer
```

### Comprehensive PR Review

For thorough PR review, run `/review-pr`, or ask for multiple aspects:

```
"I'm ready to create this PR. Please:
1. Review test coverage
2. Check for silent failures
3. Verify code comments are accurate
4. Review any new types
5. General code review"
```

This will trigger all relevant agents to analyze different aspects of your PR.

### Proactive Review

Archon may proactively use these agents based on context:

- **After writing code** → code-reviewer
- **After adding docs** → comment-analyzer
- **Before creating PR** → Multiple agents as appropriate
- **After adding types** → type-design-analyzer

## Installation

Project-local:
1. Copy `agents/` to `<project>/.archon/plugins/pr-review-toolkit/agents/`.
2. Copy the `skills/review-pr/` dir to `<project>/.archon/skills/review-pr/`.

Or user-global: agents → `~/.archon/plugins/pr-review-toolkit/agents/`, skills → `~/.config/archon/skills/` (or your platform data dir + `archon/skills/`).

Or run `plugins/install.ps1 pr-review-toolkit` / `plugins/install.sh pr-review-toolkit` from the archon-cli repo root.

Restart archon after installing (skills and agents load at startup).

## Agent Details

### Confidence Scoring

Agents provide confidence scores for their findings:

**comment-analyzer**: Identifies issues with high confidence in accuracy checks

**pr-test-analyzer**: Rates test gaps 1-10 (10 = critical, must add)

**silent-failure-hunter**: Flags severity of error handling issues

**type-design-analyzer**: Rates 4 dimensions on 1-10 scale

**code-reviewer**: Scores issues 0-100 (91-100 = critical)

**code-simplifier**: Identifies complexity and suggests simplifications

### Output Formats

All agents provide structured, actionable output:
- Clear issue identification
- Specific file and line references
- Explanation of why it's a problem
- Suggestions for improvement
- Prioritized by severity

## Best Practices

### When to Use Each Agent

**Before Committing:**
- code-reviewer (general quality)
- silent-failure-hunter (if changed error handling)

**Before Creating PR:**
- pr-test-analyzer (test coverage check)
- comment-analyzer (if added/modified comments)
- type-design-analyzer (if added/modified types)
- code-reviewer (final sweep)

**After Passing Review:**
- code-simplifier (improve clarity and maintainability)

**During PR Review:**
- Any agent for specific concerns raised
- Targeted re-review after fixes

### Running Multiple Agents

You can request multiple agents to run in parallel or sequentially:

**Parallel** (faster):
```
"Run pr-test-analyzer and comment-analyzer in parallel"
```

**Sequential** (when one informs the other):
```
"First review test coverage, then check code quality"
```

## Tips

- **Be specific**: Target specific agents for focused review
- **Use proactively**: Run before creating PRs, not after
- **Address critical issues first**: Agents prioritize findings
- **Iterate**: Run again after fixes to verify
- **Don't over-use**: Focus on changed code, not entire codebase

## Troubleshooting

### Agent Not Triggering

**Issue**: Asked for review but agent didn't run

**Solution**:
- Be more specific in your request
- Mention the agent type explicitly (e.g. `pr-review-toolkit:pr-test-analyzer`)
- Reference the specific concern (e.g. "test coverage")

### Agent Analyzing Wrong Files

**Issue**: Agent reviewing too much or wrong files

**Solution**:
- Specify which files to focus on
- Reference the PR number or branch
- Mention "recent changes" or "git diff"

## Integration with Workflow

This plugin works great with:
- **build-validator**: Run build/tests before review
- **Project-specific agents**: Combine with your custom agents

**Recommended workflow:**
1. Write code → **code-reviewer**
2. Fix issues → **silent-failure-hunter** (if error handling)
3. Add tests → **pr-test-analyzer**
4. Document → **comment-analyzer**
5. Review passes → **code-simplifier** (polish)
6. Create PR

## Contributing

Found issues or have suggestions? In this Archon port, each agent lives in its own directory under `agents/<agent>/` in this bundle; edit those files (or your installed copies under `.archon/plugins/pr-review-toolkit/agents/`) and restart archon.

## Differences from the Claude plugin

- **Agents restructured**: each source `agents/<name>.md` (single markdown file with YAML frontmatter) became a six-file agent directory `agents/<name>/` (`agent.md`, `behavior.md`, `context.md`, `tools.md`, `memory-keys.json`, `meta.json`) per Archon's loader format. All prompt content is preserved; frontmatter descriptions became `## INTENT` sections and the worked `<example>` blocks moved to `context.md`.
- **Command → skill**: `commands/review-pr.md` became `skills/review-pr/SKILL.md`, invoked as `/review-pr` (not `/pr-review-toolkit:review-pr`). Its usage examples were updated accordingly, and `$ARGUMENTS` was replaced with prose describing arguments appended to the end of the injected prompt.
- **Agent invocation**: "Task tool" references became Archon's Agent tool, with agents referenced by their installed names `pr-review-toolkit:<agent>`.
- **Model designations dropped**: the source agents declared `model: opus` (code-reviewer, code-simplifier) or `model: inherit` (others); Archon's model lineup differs, so no model key is set in the ported `meta.json` files.
- **Tool lists**: none of the source agents declared a `tools:` frontmatter list (meaning all tools), so the ported `tools.md` files contain usage notes only and no allowed-tools restriction. The `allowed-tools` list on the review-pr command was dropped from frontmatter and stated as prose in the skill body (with Task → Agent).
- **Guideline files**: `CLAUDE.md` references became `ARCHON.md` throughout.
- **Claude UI references replaced**: "All agents available in `/agents` list" became a note that agents are invoked as `pr-review-toolkit:<agent>` via the Agent tool; the `/plugins` marketplace installation instructions were replaced with Archon install steps; "Agents use appropriate models for their complexity" was dropped (no Archon equivalent after removing model tiers).
- **README additions**: a short "Skill" section documenting `/review-pr` was added (the upstream README did not document its command); the Contributing section now points at this bundle's agent directories instead of `~/.claude/agents/` and `.claude/agents/` in claude-cli-internal.
- **License**: the upstream README stated "MIT", but the bundled LICENSE file is Apache 2.0; the LICENSE file is copied verbatim and attribution lines follow it (Apache-2.0).
- **Not ported**: `.claude-plugin/plugin.json` (Archon markdown bundles need no manifest).
- **Project-specific details kept as-is**: silent-failure-hunter's references to `logForDebugging`/`logError`/`logEvent` and `constants/errorIds.ts`, and code-simplifier's ES-module/React coding standards, come from the upstream (Anthropic-internal) guidelines and are preserved verbatim; adjust them to your project's conventions if desired.

## License

Apache-2.0 (see LICENSE)

## Author

Daisy (daisy@anthropic.com)

---

**Quick Start**: Just ask for review and the right agent will trigger automatically!
