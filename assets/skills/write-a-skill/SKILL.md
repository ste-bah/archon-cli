---
name: write-a-skill
description: Meta-skill that helps users author new SKILL.md skills with proper structure, progressive disclosure, and bundled resources. Use when user wants to create, write, or build a new skill for archon-cli.
license-source: https://github.com/mattpocock/skills/blob/main/skills/productivity/write-a-skill/SKILL.md (MIT)
---
> Adapted from [mattpocock/skills](https://github.com/mattpocock/skills) (MIT licensed). Original: https://github.com/mattpocock/skills/blob/main/skills/productivity/write-a-skill/SKILL.md

# Write a Skill

Guide the user through creating a new SKILL.md skill for archon-cli.

## Process

### 1. Gather requirements

Ask the user:
- What task or domain does the skill cover?
- What specific use cases should it handle?
- When should the agent trigger this skill? (keywords, contexts, file types)
- Does it need executable scripts or just instructions?

### 2. Sketch the SKILL.md

Draft the frontmatter and process body:

```yaml
---
name: skill-name
description: Brief description of capability. Use when [specific triggers].
---
```

Process body: numbered steps with clear roles for the agent (which tools to use, when to ask the user, output expectations).

### 3. Review with user

Show the sketch to the user. Ask:
- Does this cover your use cases?
- Anything missing or unclear?
- Should any section be more or less detailed?

Iterate until approved.

### 4. Write the file

Once approved, ask the user where to save:

- **Project-local**: `<workdir>/.archon/skills/<name>.md` (only this project)
- **Global**: `~/.config/archon/skills/<name>.md` (all projects)

Write the final SKILL.md with the Write tool.

### 5. Restart instruction

Tell the user to **restart archon** to pick up the new skill. Note: the `/refresh` command does not currently reload the SkillRegistry (the registry is `Arc<SkillRegistry>` with no interior mutability). A future Phase 3.5 ticket can add live reload.

### 6. Summary

Print:
- The trigger phrase and a one-line invocation example
- The restart instruction
- The file path where the skill was written

## Description Requirements

The description is the only thing the agent sees when deciding which skill to load. It's surfaced alongside all other installed skills.

**Goal**: Give the agent just enough info to know:
1. What capability this skill provides
2. When to trigger it (specific keywords, contexts)

**Format**: First sentence what it does; second sentence "Use when [specific triggers]."

## Structure Reference

```
skill-name/
├── SKILL.md           # Main instructions (required)
├── REFERENCE.md       # Detailed docs (if > 100 lines)
├── EXAMPLES.md        # Usage examples (if needed)
└── scripts/           # Utility scripts (if needed)
```

## When to Add Scripts

Add utility scripts when:
- Operation is deterministic (validation, formatting)
- Same code would be generated repeatedly
- Errors need explicit handling

## Review Checklist

After drafting, verify:
- [ ] Description includes triggers ("Use when...")
- [ ] SKILL.md body under 100 lines (split to REFERENCE.md if larger)
- [ ] No time-sensitive info
- [ ] Consistent terminology
- [ ] Concrete examples included
