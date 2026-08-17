---
name: write-a-skill
description: Use when authoring a new SKILL.md, editing an existing one, or when a repeated workflow is worth capturing so it triggers on its own. Covers structure, progressive disclosure, and writing a description the model will actually match against.
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

- **Project-local**: `<workdir>/.archon/skills/<name>/SKILL.md` (only this project)
- **Global**: `~/.config/archon/skills/<name>/SKILL.md` (all projects)

Prefer project-local unless the user explicitly asks for global. If writing
global, first confirm the path is within the session's allowed write
directories; if not, tell the user to run `/add-dir ~/.config/archon` or use a
project-local skill.

Write the final SKILL.md with the Write tool. It can create parent directories
inside allowed roots. For bundled helpers, write scripts under
`<skill-dir>/scripts/`.

After writing, read the SKILL.md and any script files back. Only report success
if the files exist and their contents match what you intended to save. If any
write/read fails, report the failure and do not claim the skill was created.

### 5. Restart instruction

Tell the user to **restart archon** to pick up the new skill. `/refresh` does
not currently reload the SkillRegistry.

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

## The description is the whole trigger

Archon lists every skill's name and description in the system prompt, and the
model decides from that alone whether to invoke one. Nothing else about the
skill is visible until it loads. So the description is not a summary — it is
the matching rule, and a bad one means a good skill never runs.

**Write the situation, not the subject.** The reader is deciding "is this me,
right now?"

| Weak | Works |
|---|---|
| "Debugging helper" | "Use when something is broken and the cause is not obvious, or a first fix attempt did not work" |
| "For writing tests" | "Use before writing implementation code for a change whose behaviour can be stated as a test" |
| "Branch management" | "Use when a branch's work is finished and needs merging, a PR, or discarding" |

Name the moment the skill applies, including the moments the model would
otherwise skip past — "when you are about to guess", "when you are on your
second attempt". Those are where a skill earns its place.

Add the phrasings a user might type as a secondary clause, not the primary one.
A description that only matches "when the user says 'grill me'" can never fire
on its own.

## Test it before you trust it

A skill nobody verified is a guess about model behaviour written in markdown.

1. **Does it trigger?** Describe a situation it should cover, in your own words,
   without naming the skill. If it does not load, the description is wrong —
   not the model.
2. **Does it not over-trigger?** Describe an adjacent situation it should *not*
   cover. A skill that fires on everything is worse than none, because it
   crowds out the ones that fit.
3. **Does following it change the outcome?** Compare the work with and without.
   If they are the same, the skill is decoration.

Test 1 and 2 together, and re-run them after editing the description. Wording
changes triggering more than people expect.

## Review Checklist

After drafting, verify:
- [ ] Description names a situation, not a topic, and starts with "Use when..."
- [ ] Description would match without the user saying the skill's name
- [ ] Triggering tested both ways — fires when it should, stays quiet when it should not
- [ ] SKILL.md body under 100 lines (split to REFERENCE.md if larger)
- [ ] No time-sensitive info
- [ ] Consistent terminology
- [ ] Concrete examples included
- [ ] Points at what comes next, if it sits in a chain
