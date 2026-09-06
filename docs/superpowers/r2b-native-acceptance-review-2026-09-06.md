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

---

# Second review — fixes at `de6c805157918435eac55e942a287bedaa5e518e`

**Reviewed:** 2026-09-06 14:55, diff `c585b7912..de6c80515`, focused suites re-run here.

## Verdict: APPROVE for integration, with one test fix required before merge.

B1, B2, B3, M1, M2, M3 and M4 are fixed at the production call sites, each with a
regression that fails without the fix (both sabotage pairs verified net-zero).

## Verified

- **B1.** `acceptance_scratch_inputs::live` audits tracked files enumerated from the
  recorded commit, declared project inputs minus operator exclusions, and the task root.
  It never enters `target/`, `.git/` or `.archon/workflows`; nonregular objects are
  skipped; a tracked file replaced by a directory or link reads as
  `missing-or-nonregular` and voids. Regression uses a 144 GB sparse untracked file, a
  socket, and a check that writes into the project's workflow store. On the real
  project the declared input (the data root) is 4.2 MB, so the audit is seconds.
- **B2.** Quota walk on a 5 s cadence measured from the end of the previous walk, plus
  one final walk; count recorded per check. Cancellation and output checks stay at 25 ms.
- **B3.** `snapshot_project` at prepare; `reset_project` before every check removes
  added or changed paths deepest-first and restores from the baseline without rewriting
  identical files, so Cargo freshness holds and the warm target survives. Regression:
  a second check that needs the first check's data fails.
- **M1.** Only root-level credential and config names are refused; nested data config
  files copy; `project_input_excludes` applies to copy and audit alike.
- **M2.** On any failure the run dir receives the guardian's observation record or a
  minimal operational record.
- **M3.** Ordinary timeout or output overflow no longer stops later checks; integrity,
  cancellation and unverified teardown still do.
- **M4.** Shared `project_root` resolver reused.
- `cargo check --bins --tests` clean; workflow acceptance suites 37 pass; root native
  tests 10 pass single-threaded. Every changed file under 500 lines. Ledger TD-050 to
  TD-055 and the plan's R3 commit-before-finalization section are accurate.

## Required before merge

**T1. Flaky test.** `native_execution_lock_rejects_overlapping_observations`
(`workflow_run_end_native_tests.rs:280`) failed in 3 of 6 parallel runs here and passes
alone and with `--test-threads 1`. The failing assertion is the re-acquire after
`drop(lease)`. Other tests in the same process fork children with `process_group`, and
between fork and exec the child holds a copy of the lease descriptor and therefore the
flock; the re-acquire lands in that window. Production is unaffected: the guardian
releases the lease by exiting after every child is reaped. Fix the test: retry the
re-acquire for a bounded period, or run the lease tests serially. Do not change
`acquire_lease`.

## Notes, not blocking

- **N1.** In `separate` view, untracked files a check leaves in the repository worktree
  are not reset between checks; only the project view is. Tracked-source changes are
  caught by the identity check. All eleven frozen checks use `project_root` with the
  combined view, so this does not bite now. Add the worktree to the reset when a
  `repo_root` check appears.
- **N2.** The continue-or-stop decision matches substrings in error text
  (`teardown`, `reap`, `pipes`). Works, brittle. A typed failure class would be safer.
- **N3.** The baseline snapshot doubles the scratch footprint of source plus inputs.
  `scratch_bytes` in the profile must cover two copies plus the release target;
  say so in the config guidance.
- **N4.** `reset_project` reads every project file three times per check. Fine at
  4 MB of inputs; revisit if inputs grow to gigabytes.

## Next

1. Fix T1, re-run root native tests in parallel five times, merge.
2. Profile in project-1 config: repository, scratch parent, `project_inputs`
   (`.archon/trading-lab`), combined view, toolchain path, Cargo seed, limits sized for
   a native release build and N3.
3. Dry-run observation of the frozen contract at the current commit with no
   implementation: expect eleven executed, criterion failures allowed, zero operational.
4. Then the PRD implementation workflow, with commit-before-finalization enforced.
