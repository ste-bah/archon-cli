# Local Coder

## INTENT
Delegate small, well-scoped coding tasks to a local 27B LLM running on
llama-server (llama.cpp) at http://192.168.1.125:1234 to reduce API costs.
The parent agent (Claude) reads the output, validates quality, and applies
changes. Sherlock reviews catch shortfalls.

## SCOPE
### In Scope
- **Single-function generation**: Write a function matching a spec
- **Snippet refactoring**: Rewrite a function to improve clarity, types, or style
- **Unit test generation**: Generate pytest/vitest tests for existing code
- **Language conversion**: Convert a snippet between Python/TypeScript/Rust/Go
- **Boilerplate generation**: Config files, dataclass/model definitions, CLI arg parsers
- **Docstring/comment generation**: Add documentation to existing functions

### Out of Scope
- Multi-file architecture changes (use Claude directly)
- Security-sensitive code (auth, crypto, input validation at trust boundaries)
- Complex algorithms requiring deep reasoning
- Code that needs access to the full project graph
- Anything requiring more than ~4096 tokens of output

## INVOCATION
```bash
python3 scripts/local-coder.py "TASK" --context-file FILE [--context-file FILE2] [--parse]
```

### Key Flags
- `--context-file FILE` / `-c FILE`: Include source files as context (repeatable)
- `--context "inline code"`: Pass inline context
- `--parse`: Output structured JSON with extracted code blocks
- `--check`: Verify endpoint is reachable
- `--temperature 0.0`: Sampling temp (default 0.0 for deterministic)
- `--max-tokens N`: Max generation tokens (default 4096)
- `--timeout N`: HTTP timeout seconds (default 300)

## CONSTRAINTS
- The local model is 27B parameters — capable but not frontier-grade
- Must provide explicit, unambiguous task descriptions
- Always include relevant context files so the model sees the code style
- Review ALL output before applying — never blindly trust
- Output format: fenced code blocks with `# filepath:` annotations

## FORBIDDEN OUTCOMES
- DO NOT apply local-coder output without reading it first
- DO NOT send security-sensitive context (API keys, credentials) to the endpoint
- DO NOT use for tasks requiring multi-step reasoning across many files
- DO NOT retry more than once on format non-compliance — fall back to Claude

## WHEN IN DOUBT
If the task might benefit from the full project context, deeper reasoning, or
architectural awareness, use Claude directly instead of the local model.
The cost savings are not worth incorrect output that wastes debugging time.
