# Tool Instructions

## Primary Tools
- **JavaToolchain**: drives the project's own Gradle or Maven build.
  Operations: `detect`, `compile`, `analyze`, `test`, `report`. It reads the
  tools' **report files**, so findings come back with a rule key, a line, and a
  CWE where the tool supplies one — none of which the console output carries.
- **CartographerScan**: symbol index. Java nested types are indexed qualified
  (`OrderService.Builder`) and methods by their enclosing type
  (`OrderService.priceOf`). Java imports are fully qualified, so the dependency
  edges are exact rather than heuristic — better than for any other indexed
  language.
- **LeannSearch**: semantic search when you know what the code does but not what
  it is called.
- **Read / Write / Edit**: `.java`, `build.gradle[.kts]`, `pom.xml`,
  `checkstyle.xml`, PMD rulesets, `gradle/libs.versions.toml`
- **Grep / Glob**: locate callers, annotations, configuration

## The loop, in order
1. **`JavaToolchain detect`** — Gradle or Maven, and whether a wrapper exists
2. **`CartographerScan`** the area you are about to touch, before opening files
3. Read the configured Checkstyle/PMD/SpotBugs rules
4. **`compile`** — nothing else can run on code that does not build
5. **`analyze`** — Checkstyle, PMD, SpotBugs, FindSecBugs
6. **`test`**
7. Fix the returned batch at the lines given, then **go back to step 4**

`analyze` deliberately returns only the most severe handful. That is not a
display limit to work around — presenting a whole report at once measurably
degrades fix quality. Take the batch, fix it, re-run.

## Domain-Specific Patterns
- SpotBugs reads bytecode, not source, so `analyze` is meaningless until
  `compile` has succeeded
- A non-zero build exit with no parsed findings means the failure is not one the
  reports describe — a missing plugin, an unresolvable dependency, a broken
  wrapper. Read the build output.
- Prefer adding a rule to the project's Checkstyle config over policing a
  convention by hand
- Gradle: convention plugins in a `build-logic` composite build, never
  `buildSrc` (a change there invalidates the whole build cache) and never
  `allprojects`/`subprojects`
- Maven: versions in `<dependencyManagement>`, plugins pinned in
  `<pluginManagement>` — an unpinned plugin makes the build non-reproducible
