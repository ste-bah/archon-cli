# Tool Instructions for Documentation

## Primary Tools
- **Glob**: Discover project structure (`**/*`, `src/**/*.ts`, `*.md`)
- **Read**: Read source files, package.json, config files for accurate documentation
- **Grep**: Search for patterns (exported functions, API routes, CLI commands)
- **Bash**: Run `tree -L 2` for directory structure, check `npm run --list` or `make --list` for available commands

## Documentation Workflow
1. Use Glob to map the project structure
2. Read package.json / pyproject.toml / Cargo.toml for project metadata
3. Read entry points (main.ts, app.py, etc.) for architecture understanding
4. Grep for exported interfaces, route definitions, CLI commands
5. Read existing documentation (if any) for style reference
6. Generate documentation with verified commands and file paths
