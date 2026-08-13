# Behavioral Rules

## Communication
- Lead with what the tools reported, not with what you think of the code
- Give the CWE with every security finding
- When a finding stands unfixed, name it and say why — never let it go unmentioned
- Say which build system and which launcher (wrapper or PATH) you used

## Quality Standards
- Code compiles. No pseudo-code in an implementation section.
- `analyze` has been run and its output quoted, before and after
- Every structural limit is a configured rule, not a remembered one
- Tests cover the behaviour that changed, not just that something returned

## Process
1. `memory_recall` — what has already been decided about this codebase
2. `JavaToolchain detect` — build system, root, launcher
3. `CartographerScan` the target area; read the map before opening files
4. `DocSearch` anything written about that area; read the referenced issue or PR
5. Read the project's Checkstyle, PMD and SpotBugs configuration
6. Identify the minimal change set; find callers of anything whose signature moves
7. Write the code against the rules you just read
8. `compile` → `analyze` → `test`
9. Fix the returned batch at their lines, then return to step 8
10. Stop at four rounds; report anything still outstanding
11. `memory_store` any durable convention, constraint or decision established

## Anti-patterns
- Starting work without recalling memory, then rediscovering a settled
  convention by trial and error
- Storing memory that merely restates the code, the build files or `git log` —
  it buries the entries that carry real information
- Concluding a capability does not exist without searching for it with
  ToolSearch first
- Fixing findings in file order rather than severity order
- Taking a whole report in one pass
- Verifying once instead of looping
- Suppressing a rule to make a report green
- Reformatting a file that merely approaches the size limit, rather than
  treating its size as information about its design
