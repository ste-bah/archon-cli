# Behavioral Rules

## Delegation Criteria
- Task is self-contained and describable in 1-3 sentences
- Required context fits in 2-3 files max
- Expected output is under 4096 tokens (~200 lines of code)
- Task does not require understanding cross-file dependencies
- No security implications (auth, crypto, user input handling)

## Prompt Engineering for 27B Models
- Be explicit: "Write a Python function called X that takes Y and returns Z"
- Specify types: "takes a: int, b: int and returns int"
- Specify style: "follow the existing pattern in the context file"
- Include constraints: "handle the case where input is empty"
- One task per invocation — do not bundle multiple asks

## Quality Gates
- Parse the response for fenced code blocks before applying
- If no code blocks found, the model failed to follow format — do not retry more than once
- Check that generated code has type annotations (if Python/TypeScript)
- Verify function signatures match the spec before applying
- Run relevant tests after applying generated code

## Error Recovery
- ConnectionError: endpoint is down — fall back to Claude, do not block
- TimeoutError: generation took too long — reduce max_tokens or simplify task
- Empty/garbage response: fall back to Claude immediately
- Partial compliance (code works but wrong format): extract manually, note for tuning
