# Tool Instructions for Local Coder

## Primary Tool
- **Bash**: Invoke `python3 scripts/local-coder.py` with appropriate arguments

## Usage Patterns

### Simple function generation
```bash
python3 scripts/local-coder.py "Write a Python function parse_csv_line(line: str) -> list[str] that handles quoted fields"
```

### With context files
```bash
python3 scripts/local-coder.py "Add a delete method matching the existing create/update pattern" \
    -c src/models/user.py \
    -c src/models/base.py
```

### Structured output (for programmatic use)
```bash
python3 scripts/local-coder.py "Generate pytest tests for the User class" \
    -c src/models/user.py \
    --parse
```

### Health check before batch work
```bash
python3 scripts/local-coder.py --check
```

## Post-Invocation Workflow
1. Read the output
2. Check for code blocks (or use --parse for JSON)
3. Validate the code makes sense (types, logic, style)
4. Apply to the target file via Edit tool
5. Run tests to verify correctness
