# Local Coder Domain Context

## Endpoint Details
- Server: llama-server (llama.cpp)
- URL: http://192.168.1.125:1234/v1/chat/completions
- API: OpenAI-compatible chat completions
- Model: 27B parameter LLM with tool use capability
- Context window: 250k tokens
- Runs locally — no data leaves the machine

## Expected Output Format
The system prompt instructs the model to produce:
```
\`\`\`language
# filepath: path/to/file.ext
<code>
\`\`\`
```
Multiple blocks for multiple files. No prose outside fences.

## Known Limitations of 27B Models
- May not follow complex multi-step instructions reliably
- Can hallucinate function names or APIs that don't exist
- May produce syntactically valid but logically wrong code
- Format compliance degrades with very long contexts
- Best at: single-function tasks with clear specs and examples
- Worst at: complex control flow, concurrent code, subtle edge cases

## Cost Model
- Local inference: $0 marginal cost (electricity only)
- Claude API: ~$15/MTok input, ~$75/MTok output (Opus)
- Break-even: every successful delegation saves $0.01-0.50 depending on task size
- Failed delegation + Claude fallback: costs ~2x the Claude-only path (wasted time)
- Target: 70%+ success rate on delegated tasks to be net positive
