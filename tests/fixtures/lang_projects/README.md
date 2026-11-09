Dependency-free proof projects for issue #631. Each has one implementation source, one passing test, and one intentionally failing test. Python uses `unittest`; TypeScript runs through Node 22 native type stripping and `node:test`; Go uses `testing`. The Python `setup.cfg` additionally declares the repository CLI used by the passability proof.

The Rust helper detects tools before executing them, prints `SKIP` if absent, and forbids package downloads. Its temporary directories and Go/npm caches live under `tests/agnostic-proof-results/`. These projects must not be installed into the operator environment.

# Issue 631 Java proof fixtures

Each project has one production source and two JUnit 5 cases: `passing` and
`failing` (the latter intentionally expects 5 from 2 + 2).

`transcripts/recorded/` contains real operator recordings made outside the
sandbox with JDK 27.0.1, Maven 3.10.0 and Gradle 9.8.2 on macOS arm64.
Each directory includes provenance, a failing run, a missing-selector run,
a successful zero-test run, and JUnit XML. Maven's zero-test run uses
`failIfNoSpecifiedTests=false` with `-Dtest=NoSuchTest`: it has BUILD SUCCESS
and no test summary line. Gradle uses `failOnNoMatchingTests=false`.
Reconstructed JVM test transcripts and XML were deleted; no test uses them.

Compiler output (`javac-error.txt`, `kotlinc-error.txt`, `go-compile-error.txt`)
is genuine, with adjacent invocation and exit-status provenance JSON.
Historical capture/offline-build attempts remain as provenance only.

`python3 tests/fixtures/lang_projects/check_toolchains.py` requires a functioning
Java/Javac from the environment's JAVA_HOME, Maven, Gradle and Go. Missing or
unusable tools fail with `toolchain missing: <tool>`; missing JAVA_HOME fails
as missing java. Offline builds must actually execute CalculatorTest; uncached
dependencies and sandbox errors fail instead of silently skipping. Temporary
cache state stays inside this worktree. This suite performs no installations.
