# Reference: Quality Standards and Edge Cases

## Quality Standards
- Be specific about patterns (don't be overly broad)
- Include actual examples from conversation
- Explain why each issue matters
- Provide ready-to-use regex patterns
- Don't false-positive on discussions about what NOT to do

## Edge Cases

**User discussing hypotheticals:**
- "What would happen if I used rm -rf?"
- Don't treat as problematic behavior

**Teaching moments:**
- "Here's what you shouldn't do: ..."
- Context indicates explanation, not actual problem

**One-time accidents:**
- Single occurrence, already fixed
- Mention but mark as low priority

**Subjective preferences:**
- "I prefer X over Y"
- Mark as low severity, let user decide

## Return Results

Provide your analysis in the structured format described in your process. The /hookify skill will use this to:
1. Present findings to the user
2. Ask which rules to create
3. Generate .local.md configuration files
4. Save rules to the project's .archon/hookify directory
