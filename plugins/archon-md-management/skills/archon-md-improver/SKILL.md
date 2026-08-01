---
name: archon-md-improver
description: Audit and improve ARCHON.md files in repositories. Use when the user asks to check, audit, update, improve, or fix ARCHON.md files, or mentions ARCHON.md maintenance or project memory optimization. Scans for all ARCHON.md files, evaluates quality against templates, outputs a quality report, then makes targeted updates.
tools: Read, Glob, Grep, Bash, Edit
license-source: https://github.com/ste-bah/test-plugins-official/tree/main/plugins/claude-md-management (Apache-2.0)
---
> Ported from the Claude Code `claude-md-management` plugin ([source](https://github.com/ste-bah/test-plugins-official/tree/main/plugins/claude-md-management), Apache-2.0).

# ARCHON.md Improver

Audit, evaluate, and improve ARCHON.md files across a codebase to ensure Archon has optimal project context.

**This skill can write to ARCHON.md files.** After presenting a quality report and getting user approval, it updates ARCHON.md files with targeted improvements.

## Workflow

### Phase 1: Discovery

Find all ARCHON.md files in the repository:

```bash
find . -name "ARCHON.md" -o -name ".archon.md" -o -name ".archon.local.md" 2>/dev/null | head -50
```

**File Types & Locations:**

| Type | Location | Purpose |
|------|----------|---------|
| Project root | `./ARCHON.md` | Primary project context (checked into git, shared with team) |
| Local overrides | `./.archon.local.md` | Personal/local settings (gitignored, not shared) |
| Global defaults | `~/.archon/ARCHON.md` | User-wide defaults across all projects |
| Package-specific | `./packages/*/ARCHON.md` | Module-level context in monorepos |
| Subdirectory | Any nested location | Feature/domain-specific context |

**Note:** Archon auto-discovers ARCHON.md files in parent directories, making monorepo setups work automatically.

### Phase 2: Quality Assessment

For each ARCHON.md file, evaluate against quality criteria. For detailed rubrics, read `references/quality-criteria.md` in this skill's directory — installed at `.archon/skills/archon-md-improver/references/quality-criteria.md` (project install; or the equivalent path under your user-global skill root).

**Quick Assessment Checklist:**

| Criterion | Weight | Check |
|-----------|--------|-------|
| Commands/workflows documented | High | Are build/test/deploy commands present? |
| Architecture clarity | High | Can Archon understand the codebase structure? |
| Non-obvious patterns | Medium | Are gotchas and quirks documented? |
| Conciseness | Medium | No verbose explanations or obvious info? |
| Currency | High | Does it reflect current codebase state? |
| Actionability | High | Are instructions executable, not vague? |

**Quality Scores:**
- **A (90-100)**: Comprehensive, current, actionable
- **B (70-89)**: Good coverage, minor gaps
- **C (50-69)**: Basic info, missing key sections
- **D (30-49)**: Sparse or outdated
- **F (0-29)**: Missing or severely outdated

### Phase 3: Quality Report Output

**ALWAYS output the quality report BEFORE making any updates.**

Format:

```
## ARCHON.md Quality Report

### Summary
- Files found: X
- Average score: X/100
- Files needing update: X

### File-by-File Assessment

#### 1. ./ARCHON.md (Project Root)
**Score: XX/100 (Grade: X)**

| Criterion | Score | Notes |
|-----------|-------|-------|
| Commands/workflows | X/20 | ... |
| Architecture clarity | X/20 | ... |
| Non-obvious patterns | X/15 | ... |
| Conciseness | X/15 | ... |
| Currency | X/15 | ... |
| Actionability | X/15 | ... |

**Issues:**
- [List specific problems]

**Recommended additions:**
- [List what should be added]

#### 2. ./packages/api/ARCHON.md (Package-specific)
...
```

### Phase 4: Targeted Updates

After outputting the quality report, ask user for confirmation before updating.

**Update Guidelines (Critical):**

1. **Propose targeted additions only** - Focus on genuinely useful info:
   - Commands or workflows discovered during analysis
   - Gotchas or non-obvious patterns found in code
   - Package relationships that weren't clear
   - Testing approaches that work
   - Configuration quirks

2. **Keep it minimal** - Avoid:
   - Restating what's obvious from the code
   - Generic best practices already covered
   - One-off fixes unlikely to recur
   - Verbose explanations when a one-liner suffices

3. **Show diffs** - For each change, show:
   - Which ARCHON.md file to update
   - The specific addition (as a diff or quoted block)
   - Brief explanation of why this helps future sessions

For expanded guidance and worked examples, read `references/update-guidelines.md` in this skill's directory — installed at `.archon/skills/archon-md-improver/references/update-guidelines.md` (project install; or the equivalent path under your user-global skill root).

**Diff Format:**

```markdown
### Update: ./ARCHON.md

**Why:** Build command was missing, causing confusion about how to run the project.

```diff
+ ## Quick Start
+
+ ```bash
+ npm install
+ npm run dev  # Start development server on port 3000
+ ```
```
```

### Phase 5: Apply Updates

After user approval, apply changes using the Edit tool. Preserve existing content structure.

## Templates

For ARCHON.md templates by project type, read `references/templates.md` in this skill's directory — installed at `.archon/skills/archon-md-improver/references/templates.md` (project install; or the equivalent path under your user-global skill root).

## Common Issues to Flag

1. **Stale commands**: Build commands that no longer work
2. **Missing dependencies**: Required tools not mentioned
3. **Outdated architecture**: File structure that's changed
4. **Missing environment setup**: Required env vars or config
5. **Broken test commands**: Test scripts that have changed
6. **Undocumented gotchas**: Non-obvious patterns not captured

## User Tips to Share

When presenting recommendations, remind users:

- **Capture learnings as you go**: run `/revise-archon-md` at the end of a session to fold session learnings into ARCHON.md
- **Keep it concise**: ARCHON.md should be human-readable; dense is better than verbose
- **Actionable commands**: All documented commands should be copy-paste ready
- **Use `.archon.local.md`**: For personal preferences not shared with team (add to `.gitignore`)
- **Global defaults**: Put user-wide preferences in `~/.archon/ARCHON.md`

## What Makes a Great ARCHON.md

**Key principles:**
- Concise and human-readable
- Actionable commands that can be copy-pasted
- Project-specific patterns, not generic advice
- Non-obvious gotchas and warnings

**Recommended sections** (use only what's relevant):
- Commands (build, test, dev, lint)
- Architecture (directory structure)
- Key Files (entry points, config)
- Code Style (project conventions)
- Environment (required vars, setup)
- Testing (commands, patterns)
- Gotchas (quirks, common mistakes)
- Workflow (when to do what)
