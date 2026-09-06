# R2b Acceptance Execution Amendment — host execution in scratch roots

**Status:** APPROVED by Steven 2026-09-06 ("yes"), replacing the withdrawn
`2026-09-05-r2b-acceptance-world-amendment.md` (Docker Desktop Linux world).
**Amends:** `2026-08-27-decomposition-r2-engine-native-design.md`, R2b bullet one
("mandatory ephemeral isolated acceptance world") and "Acceptance check execution
boundary and R2a deferral". **Does not amend:** ObserveOnly authority, legacy
admission, symbolic host capabilities, judge/freeze/pin semantics, or the rule
that unfrozen model text never executes anywhere.

## Why the Docker world is withdrawn

The goal of R2b is executable acceptance checks for the frozen contract, so the
task implementation can be proved. The container design worked against that goal:

- It builds a second target. Archon is developed and run on this Mac; a Linux
  build of the workspace is not something anyone ships, may not compile, and
  costs a VM release build per check.
- It cannot reach what the checks test. The local services the deliverable
  integrates with run on this Mac; from a container they need a network broker,
  which was most of the plan.
- It defends against a threat this deployment does not have: a hostile model on
  a shared machine. This is a single-operator machine, the commands are frozen
  in a readable file, and the judge already refused anything destructive.

## Replacement rule

> Frozen, judged acceptance commands (`AcceptanceCheck::Command`, nested floor
> `typed_verifier_command`, residual `fail_closed_check`) execute on the host,
> natively, but never in the live roots. The host prepares per-observation
> scratch roots: a git worktree of the repository at the implementation run's
> recorded commit, a copy of the declared project inputs at their relative
> paths, a private Cargo target and home, an environment built from a closed
> host-configured set with credentials stripped, and a scratch temp dir. The
> command's `TrustedCwd` maps to the scratch project or repository root. Before
> and after each observation the host hashes the live project, repository and
> task roots; any change is an integrity failure that voids the observation.
> Local services remain reachable exactly as they are for the operator. Model
> output cannot widen any of this: roots, environment, limits and the commit
> come from host state, the command bytes from the frozen contract only.

### What "frozen and judged" means

Only commands that appear byte-for-byte in a pinned acceptance contract or lock
that passed structure validation, host policy and the judge run. The observer
re-derives the command from the pinned chain at observation time; a command
that does not hash-match the pin is not executed and is recorded as a chain
integrity failure.

### Scratch root construction

1. Repository: `git worktree add --detach <scratch>/repo <commit>` where the
   commit is the implementation run's recorded HEAD. Uncommitted work in the
   live checkout is never present in scratch, which also closes the compiled
   input gap noted in the R2a evidence review for future observations.
2. Project inputs: copy the host-configured list of project directories (data
   roots, strategy artifacts, task root read-only) into `<scratch>/project` at
   their original relative paths. The task root and pinned chain are copied
   read-only; nothing in scratch is copied back.
3. Combined view: because checks select `project_root` and also build, the
   scratch project root contains the repository worktree's contents alongside
   the project inputs when the host config says `project_repository_view =
   "combined"`; conflicting nonidentical files refuse. Default is separate.
4. Build: `CARGO_TARGET_DIR=<scratch>/target`, `CARGO_HOME=<scratch>/cargo-home`
   seeded from a host-configured credential-free cache, and a symlink at the
   relative `target` path in the scratch root to the same directory so
   `./target/release/...` selects what this observation built. A per-run warm
   target cache is host policy: checks in one observation share it because
   their inputs are identical.
5. Environment: start empty; add PATH to the host toolchain, HOME/TMPDIR under
   scratch, the Cargo variables above, and a closed host-configured list of
   nonsecret variables. Provider credentials, workflow configs and tokens are
   never present. A command that needs a credential fails as a failed
   criterion, not as a containment event, and the operator sees why.
6. Limits: timeout, output bytes and scratch size from an operator profile
   sized for a native release build, recorded with the evidence.
7. Teardown: the worktree is removed with `git worktree remove --force`, scratch
   deleted, process group killed on timeout or parent death. Teardown failure
   is operational evidence, never a pass.

### Result semantics

- Normal nonzero exit is a failed criterion. Timeout, output overflow, setup
  failure, a live-root hash change, or teardown failure is operational evidence.
- Pass requires zero exit, unchanged live-root hashes and verified teardown.
- The observer never changes the implementation's terminal status; authority
  stays `ObserveOnly` until R4.

### Honest claim

This is confinement by construction and audit: scratch roots, stripped
environment, before-and-after hashes of the live roots, process-group teardown.
It is not a security sandbox against a hostile executable, and the spec's
existing "Honest macOS write-safety claim" applies unchanged. If a later
deployment needs a real boundary, an isolated backend can be added behind the
same `AcceptanceWorld` port without touching the observer or the contract.

## Effect on the R2b plan

- Task 9 becomes the host scratch executor: worktree, project copy, combined
  view, build cache, environment, limits, hashes, teardown. No image, broker,
  bridge, container inspection or Docker client. Its tests: a generic fixture
  project that builds and runs its binary from the relative target path, mutates
  scratch data, and leaves live roots byte-identical; a hostile fixture that
  writes to the live root path and is caught by the hash audit; timeout and
  parent-death teardown; credential absence.
- Task 10 is unchanged: route the three command-bearing check shapes through
  the port, table-driven, with call-site sabotage.
- Task 12A is unchanged except that the readiness matrix should show all eleven
  frozen checks executable, not five.
- Tasks 1–8 and 11 remain behind their separate approval.
