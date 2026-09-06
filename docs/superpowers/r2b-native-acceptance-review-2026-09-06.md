# Native acceptance slice — adversarial review

**Reviewed:** `r2b/native-acceptance` at `c585b79127cfefc931d0b00826455deda39c4fb3`, base `f44f55b85`,
checkout `/Volumes/Externalwork/archon-cli/r2b-native-acceptance`, 2026-09-06 13:35.
**Reviewer:** Claude (Fable), adversarial. Code read in full; focused suites re-run here.

## Verdict: REJECT for integration at c585b7912. Three blocking findings, all fixable inside the slice.

The authority chain, process control and guardian are sound and I could not break
them on paper. What fails is contact with a real project: the live-root audit and the
scratch quota walk were written against a 3-file fixture and will not survive the
repository and project they are meant to observe.

## Verified (no finding)

- Command authority: pin bytes digested in the parent, re-read and re-digested in the
  guardian; `validate_full_chain` and `validate_acceptance_bundle` against the pin;
  per-command `content_digest` match; accepted verdict and no policy finding required;
  residual text refused. Model bytes reach only the shell's stdin.
- Terminal identity: persisted terminal state and event, snapshot equality, pin equality
  with the launch identity, Completed or NeedsReview only.
- Scratch: detached worktree at the recorded commit with revision check; relative
  `target` symlink in both cwd views to one scratch target; `env_clear` plus a closed
  binding set; task root copied read-only; scratch never inside a live root.
- Process control: process group, SIGKILL on timeout, overflow, cancel and after normal
  exit; group verified gone by ESRCH; pipes drained with a deadline.
- Guardian: parent liveness pipe, flock lease held for the guardian's lifetime, request
  read bounded, total wait bounded. Parent SIGKILL test passes here.
- Tests: workflow crate 31 pass, root crate 12 pass (native_tests 9, policy 3). Five root
  failures are pre-existing on the base and read Steven's uncommitted trading crate.
- Sabotage commits net to zero (`git diff` across each pair). Every changed Rust file is
  under 500 lines. No PRD-specific strings in production code.

## Blocking

**B1. The live-root audit hashes the whole repository and project, twice.**
`acceptance_scratch_observe.rs::live` calls `inventory` on `policy.repository`,
`policy.project` and `policy.task_root` with no exclusions. On the real roots that is:

| root | size | files |
|---|---|---|
| repository `target/` | 144 GB | 881,355 |
| repository `.git/` | 1.8 GB | |
| project `.archon/` | 31 GB | |

Every file is read and hashed before setup and again after teardown, under a
`Control` deadline of `timeout_secs`. The audit alone will exceed any sane timeout and
report "after audit failed". If it ever completed, any write to the project's workflow
store, logs or daemon output during the observation makes `live_roots_unchanged` false
and voids the result; on a real project that is always. `inventory` also errors on any
non-regular file under either root. The tests use a three-file fixture and cannot see
this. The amendment text I wrote said "hashes the live roots" and the code follows it
literally; the amendment is wrong too and I will correct it. Required: audit exactly the
inputs scratch was built from and nothing else — tracked source files at the recorded
commit (`git ls-tree`), the declared project inputs, and the task root — and skip
non-regular files rather than failing. Add a test whose live repository contains a large
untracked directory and whose live project has a file that changes during the
observation; it must pass.

**B2. The scratch quota walk runs every 25 ms over a growing build tree.**
`acceptance_scratch_process.rs::run` calls `scratch_size(roots.root())` in the 25 ms
select arm, a full recursive stat of scratch including `target/`. A release build of
this workspace produces hundreds of thousands of files; the walk will take longer than
its interval and run continuously, competing with the build for I/O and CPU. Required: a
coarse interval, seconds not milliseconds, or an incremental measure; assert in a test
that the walk count during a multi-second command is small.

**B3. One shared mutable project copy across all checks makes results order-dependent.**
`ScratchRoots` copies the declared inputs once; `execute_check` runs every check against
the same copy and only records `changed_project_paths` (`input_reset: false` always). A
check that ingests data leaves it for the next check, which may then pass only because
of it. The frozen checks were judged independently. Required: restore the declared
mutable inputs to their snapshot before each check (or a private copy per check), record
that the reset happened, and keep the warm build target shared. Test: two checks where
the second passes only if the first's mutation persists must fail.

## Must fix before live use, not blocking integration

**M1.** `copy_tree_inner` refuses any file named `config.json`, `config.toml`, `.env` or
`credentials*` anywhere inside a declared project input. Data roots legitimately contain
files with those names. Declared inputs are operator-approved; restrict the refusal to
the Cargo home seed and to a small explicit list at the project root, and let the
operator exclude paths in config.

**M2.** On a voided observation (audit or teardown failure) the guardian exits nonzero
and `evaluate` never writes `observer/native-observation.json` into the run dir; the
per-check output survives only in the scratch evidence directory. Write the observation
record to the run dir on every path so the run carries its own evidence.

**M3.** The first operational error stops all remaining checks. A timeout on check 3
voids 4 to 11. Continue unless scratch integrity itself failed.

**M4.** `run_end_native::evaluate` derives the project root as `store.root()` two
parents up instead of reusing `workflow_run_end_snapshot::project_root`. Reuse it.

## Accepted as stated by the builder

- Residual `fail_closed_check` execution refused: correct. Decision proposed: residual
  records stay unexecuted until a later slice adds a pinned command reference to the
  residual schema; no freeze semantics are invented now.
- Literal `/tmp` in five frozen checks: accepted; the host `/tmp` is not a live root.
- Only committed source is observed: correct and intended. Consequence for R3: the
  implementation workflow must commit its work before finalization or the checks build
  without it. State this in the R3 plan.
- Credentials stripped: provider checks needing keys fail as failed criteria. Intended.
- Not a hostile-code sandbox: agreed and documented.

## Before integration

1. Fix B1, B2, B3 with the named tests; M1 and M2 as well.
2. Correct the amendment's audit sentence to the scoped inputs (Claude does this).
3. Re-run the workflow acceptance suites, the root native tests and the clean release
   build; second review of the diff only.

## Before the PRD implementation workflow

1. Add `[workflow.acceptance_execution]` to project-1 config: repository, scratch
   parent, project inputs (the data root and strategy directories), combined view,
   toolchain path, Cargo seed, limits sized for a native release build.
2. Dry run: observe the frozen 15-task contract against the current commit with no
   implementation. Expect 11 executed checks, most failing as criteria, none
   operational. That proves the pipeline end to end before any trading code exists.
3. The R3 plan must state that task implementations commit before finalization.
