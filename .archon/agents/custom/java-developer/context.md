# Domain Context

## Background
Java work is overwhelmingly work inside an existing codebase. The constraints
that matter are rarely language features — they are the build's configured
rules, the conventions of the surrounding module, and the blast radius of a
signature change. The tooling is unusually good at making all three checkable,
which is why this agent is built around it rather than around Java knowledge.

## The analysis tools
- **Checkstyle** — source-level style and structure: file length, method
  length, parameter count, naming. Configured per project in `checkstyle.xml`.
- **PMD** — source-level defect patterns: empty catch blocks, excessive
  parameter lists, cyclomatic and cognitive complexity. Rules live in
  categories (`category/java/errorprone.xml`, `design.xml`, `bestpractices.xml`)
  and move between them across major versions.
- **SpotBugs** — bytecode analysis, so it sees what the compiler produced rather
  than what was written. Finds null dereferences, resource leaks, and
  concurrency errors source-level tools cannot.
- **FindSecBugs** — a SpotBugs plugin: 144 security patterns mapped to CWE
  identifiers. This is what makes a security claim checkable instead of
  rhetorical.
- **Error Prone / NullAway** — compile-time, with auto-fixes. House style
  enforced by the compiler rather than requested of a person.
- **PIT** — mutation testing. Measures whether the suite would actually catch a
  regression, as opposed to line coverage, which does not.

None of these need a system install: they are declared in the project's build
and fetched by it, so they behave identically on every OS.

## Key Concepts
- **The wrapper**: `gradlew` / `mvnw` pin the build-tool version the project was
  written against. Using the wrapper reproduces the project's build; using the
  tool on PATH runs a different one.
- **Report files**: every analyser writes machine-readable XML alongside its
  console summary. The console text differs between Gradle and Maven and carries
  neither rule identity nor CWE.
- **Rank vs severity**: SpotBugs' 1–20 rank measures confidence that a finding
  is real, not its cost. Security findings need floor-raising.
- **Cognitive complexity** (`java:S3776`): measures how hard a method is to
  read, unlike cyclomatic complexity which counts branches. Reducing nesting in
  one place routinely raises it in another.

## Common Patterns
- **Convention plugins**: shared build logic as plugins, applied per module.
  Preferred over `allprojects {}` / `subprojects {}` because cross-project
  configuration hides what is happening — Gradle's stated objection is that
  "build logic can be injected into a subproject which is not obvious when
  looking at its build script".
- **Where convention plugins live**: `buildSrc` or a `build-logic` composite
  build. Gradle documents both and recommends neither over the other. The
  distinction that matters in practice is that "changes to code in `buildSrc`
  will invalidate the configuration phase and require re-execution of all
  tasks" — costly on a large build if that logic changes often.
- **Version catalogs** (`gradle/libs.versions.toml`): dependency coordinates in
  one typed place.
- **BOM import** (Maven): a dependency's own bill of materials instead of
  repeated version properties.
- **Fix cascades**: one fix causing the next is the normal case, not an
  exception. It is the reason the process is a loop and not a verify step.
