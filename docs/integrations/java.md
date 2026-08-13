# Java support

Archon indexes Java source and drives a Java project's own build and analysis
tooling. Both Gradle and Maven are supported.

The design rests on one observation: **the model's prompt is the least valuable
part of producing good Java.** Static-analysis feedback in the loop moves the
numbers; a well-written persona produces code that is syntactically fine and
inconsistent from file to file. So the pieces below are tools, and the agent is
mostly a procedure for using them.

## What is installed

```bash
scripts/install-system-deps.sh --with-java
```

```powershell
scripts\install-system-deps.ps1 -WithJava
```

This installs exactly three things: **a JDK, Gradle and Maven**.

The JDK is OpenJDK — Eclipse Temurin on Windows and macOS (Adoptium's TCK-certified
build of the OpenJDK sources), and the distribution's own OpenJDK packages on
Linux. Pinned to **25**, the current LTS, rather than the newest feature release:
a six-month release is out of support as soon as its successor ships, so pinning
one means pinning something already at the end of its life.

On Linux the version is not hard-pinned at all — the installer probes for the
newest OpenJDK the distribution actually carries, newest-first, because a fixed
version is wrong on any release that lacks it and the distribution default is
often several LTS versions behind (Ubuntu 24.04 still defaults to 21).

This JDK's job is to **run** Gradle and Maven. What a project compiles against is
that project's own choice, via Gradle toolchains or `maven.compiler.release`, and
is not constrained by the version installed here.

Nothing else is needed. Checkstyle, PMD, SpotBugs, FindSecBugs, Error Prone,
NullAway and PIT are all declared as plugins in the project's own build and
fetched by it, so they need no system install and behave identically on every
OS. Semgrep is deliberately excluded — it has no native Windows support, and
FindSecBugs covers the same OWASP and CWE ground on the JVM.

Two platform notes:

- **Windows**: winget packages neither Gradle nor Maven. Both are installed
  from their projects' official archives into `%LOCALAPPDATA%\Programs`, with
  the published checksum verified before extraction, and their `bin`
  directories appended to the user PATH. Nothing requires elevation. The JDK is
  Temurin 25 from winget, installed with `FeatureJavaHome` so `JAVA_HOME` is
  set — Maven reads it directly and fails without it.
- **Linux**: the JDK and Maven come from the distribution. Gradle does not:
  Ubuntu 22.04 ships Gradle 4.4.1 and the Fedora/RHEL family does not package
  it at all, so it is installed from `services.gradle.org` with its SHA-256
  verified. Arch and Homebrew track upstream closely enough to use their
  packages.

`--check` / `-Check` reports exit code 2 when `java`, `javac`, `mvn` or
`gradle` is missing. `javac` is probed as well as `java` because a JRE
satisfies `java` and cannot compile.

## Indexing

`CartographerScan` handles `.java` alongside Rust, Python, TypeScript,
JavaScript and Go. Two things differ for Java:

**Names are qualified.** Java nests types arbitrarily and repeats method names
freely across classes, so a bare `Builder` or `process` identifies nothing. A
nested type is indexed as `OrderService.Builder`, and a method as
`OrderService.priceOf`. Substring search still finds the short form.

**The dependency graph is exact.** Java imports name a fully qualified type, so
the edges are not a guess at what a relative path resolves to — unlike every
other language the cartographer handles. On-demand imports
(`import java.util.*;`) resolve to the package; static imports have the trailing
member stripped so the edge points at the declaring type.

Records and annotation types are their own symbol kinds rather than being
flattened into `Struct` and `Interface`.

## The toolchain

`JavaToolchain` detects the build system from the project layout and runs it.
Operations:

| Operation | Runs a build | What it does |
|---|---|---|
| `detect` | no | Build system, project root, and which launcher will be used |
| `report` | no | Re-read reports a previous run wrote |
| `compile` | yes | `classes testClasses` / `test-compile` |
| `analyze` | yes | Checkstyle, PMD, SpotBugs, FindSecBugs |
| `test` | yes | The project's test task |

### It reads report files, not console output

Every one of these tools writes a machine-readable report and also prints a
human summary. The reports are what get read: console output is formatted for a
terminal, differs between Gradle and Maven, and carries neither the rule
identity nor the CWE mapping that makes a finding actionable.

| Tool | Gradle | Maven |
|---|---|---|
| Checkstyle | `**/build/reports/checkstyle/*.xml` | `**/target/checkstyle-result.xml` |
| PMD | `**/build/reports/pmd/*.xml` | `**/target/pmd.xml` |
| SpotBugs | `**/build/reports/spotbugs/*.xml` | `**/target/spotbugsXml.xml` |
| Tests | `**/build/test-results/**/TEST-*.xml` | `**/target/surefire-reports/TEST-*.xml` |

Compile is the one exception: neither build system persists javac's
diagnostics, so they are parsed from captured output. That keeps compile errors
anchored at a line like every other finding rather than arriving as a wall of
console text.

### The wrapper is preferred

If the project ships `gradlew` or `mvnw`, that is what runs. The wrapper pins
the build-tool version the project was written against, so using it is the
difference between reproducing the project's build and running a different one.
`detect` says which of the two it chose.

### An analyser that produced nothing is named

An analyser that ran and found nothing writes an empty report. One that crashed
writes none at all. Both contribute zero findings, so without checking that the
report *exists*, a dead analyser is indistinguishable from a clean pass. When a
report is absent, `analyze` says so explicitly rather than reporting a clean
stage.

An absent report is not proof of breakage — a project that does not configure
PMD will never write a PMD report — so on its own it is surfaced as information
rather than treated as a failure.

**When the build output says why, it becomes an error.** The case worth naming
is version skew between an analysis plugin and the JDK running the build.
SpotBugs reads bytecode including the JDK's own runtime classes, so its bundled
ASM has to understand the JDK it runs on; a plugin older than the JDK aborts
before writing anything, while Checkstyle and PMD carry on and the build still
exits zero. Nothing else in the run distinguishes that from a clean scan.

`analyze` detects it and fails the stage with the version named:

```
No report from: spotbugs. That analyser is either not configured in this
build or failed to run — which is NOT the same as it finding nothing.

An analysis tool could not read Java 25 class files and aborted before
writing its report. Its bundled bytecode reader predates that release.
...
Fix it by raising the plugin to a release newer than Java 25:
  Maven:  com.github.spotbugs:spotbugs-maven-plugin
  Gradle: id("com.github.spotbugs")
```

The JDK version is derived arithmetically — a class file's major version is its
JDK feature version plus 44, so 69 is Java 25 — rather than looked up in a table
of releases. This matters because the failure recurs with every new Java
version: whoever's plugin is older than their JDK hits it next, and the
diagnosis stays correct for JDKs that did not exist when it was written.

### Findings

Each finding carries a file, a line, a severity, a rule key, and a CWE where
the tool supplies one. Severities from the four tools are normalised onto one
scale so the batch can be ordered.

One deliberate correction sits in that mapping. SpotBugs ranks findings by how
likely a report is to be a real defect, not by what it costs when it is — a
live SQL injection comes back at rank 12, which on the rank scale alone lands
in the same band as an unused local. Anything in the SECURITY category is
therefore floored at `critical`, so an injection cannot sort below a style
violation.

### Batching

`analyze` returns the most severe handful of findings, not the whole report,
anchored at their lines. This is not a display convenience: presenting an
entire report at once measurably degrades the quality of the fixes. The
remaining count is reported so nothing is silently dropped.

The intended loop is compile → analyze → fix a batch → **compile again**. One
fix routinely causes the next: collapsing a nested conditional raises the
cognitive complexity of the enclosing method. A single verify pass ships that
regression. Four rounds is the practical ceiling — the measured gain is nearly
all in the first three or four.

## The agent

`.archon/agents/custom/java-developer/` encodes the procedure above: read the
map and the configured rules *before* writing, run the loop, work findings by
severity in small batches, and never describe a stage as clean without having
run it. It follows the same six-file layout as the other language specialists
(`agent.md`, `context.md`, `tools.md`, `behavior.md`, `memory-keys.json`,
`meta.json`).

It treats the 500-line file ceiling as a Checkstyle `FileLength` rule to be
configured rather than an instruction to be remembered — a limit that exists
only in a prompt is not a limit. The same applies to method length, parameter
count and cognitive complexity.

Build conventions it follows: for Gradle, convention plugins in preference to
`allprojects`/`subprojects`, version catalogs, flat modules. For Maven, versions
in `<dependencyManagement>` and plugins pinned in `<pluginManagement>`.

It does **not** claim `buildSrc` is wrong. Gradle documents both `buildSrc` and a
`build-logic` composite build and recommends neither over the other; the
difference worth knowing is that changes to `buildSrc` invalidate the
configuration phase and force re-execution of all tasks, which is expensive on a
large build whose convention logic changes often.

## Fixture projects

`crates/archon-tools/tests/fixtures/java/` holds a Gradle and a Maven project
that share one deliberately defective source tree — an injection sink, a
swallowed exception, a nine-parameter method and an over-length file. Because
the source is shared verbatim, a difference in findings between the two halves
is a difference between the build systems rather than between their inputs.

The end-to-end tests are opt-in:

```bash
ARCHON_JAVA_E2E=1 cargo test -p archon-tools --test java_e2e_tests
```

They run real builds, so they need the toolchain and a network fetch on first
run. When `ARCHON_JAVA_E2E=1` is set and the toolchain is *missing*, they fail
rather than skip — a test that quietly passes when it did not run makes a
broken toolchain look healthy.

The report-parsing contract is covered separately by `java_report_tests.rs`
against checked-in report shapes, so it is still tested on machines that never
run a JVM.

## Not covered

- No Kotlin, Scala or other JVM languages.
- No IDE or LSP integration beyond the existing `lsp` tool.
- No Gradle Tooling API. Using it from Rust would mean shipping a Java helper
  JAR — a JVM subprocess either way — and it does nothing for Maven. Gradle's
  daemon provides incremental compilation regardless of how it is invoked, so
  the only thing the API would add is structured diagnostics, which the report
  files already provide.
