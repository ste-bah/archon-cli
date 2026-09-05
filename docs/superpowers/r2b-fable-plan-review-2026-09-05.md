# R2b plan review response

**Status:** Plan revised for resubmission; not approved for execution.
**Reviewed:** 2026-09-05 against source `0c81c4321` and the live frozen contract.
**Execution preference:** inline, one coordinator, no implementation subagents.

## Findings verified

1. **World design gap — confirmed.** Eight of the eleven frozen checks invoke
   `cargo build --release --bin archon`, then `./target/release/archon`. The old
   read-only guest layout and empty env did not provide a build location, dependency
   home or usable target path. Mutable-data checks also need private writable state.
2. **Ordering — confirmed.** The current finalizer persists terminal state/event
   before invoking the observer (`src/command/workflow_live_v2_finalizer.rs:55–110`).
   `workflow_run_end_snapshot.rs` already captures launch eligibility/identity.
   Full Tasks 3/5/8 are not prerequisites for routing checks through a new world.
   The world needs its own minimum teardown and duplicate-execution controls now.
3. **Configuration authority — confirmed.** Writable data/build paths, environment
   and service allowlists must be host policy, never extracted as authority from
   model command text. Task 9 now owns strict launch-bound policy and evidence.
4. **Spec conflict — explicit.** Original spec line 711 requires network disabled,
   empty env and immutable input mounts. The proposed amended policy lives in
   `specs/2026-09-05-r2b-acceptance-world-amendment.md` pending approval; the original
   approved spec was not silently rewritten.

## Corrections to the review's factual premises

- It is **8 build checks, 3 read-only checks**, not all 11 build/nextest. None of
  the frozen checks contains nextest. The read-only checks can run if their inputs
  are mapped and present; an absent artifact is a failed criterion, not necessarily
  “containment”. No check was executed during this review.
- Environment redirection alone does not fix the build checks: their subsequent
  relative executable path must point to the very same scratch build output.
- All checks select `project_root`; that host directory currently has no
  `Cargo.toml`. Repository and project are separate. An explicit config-owned
  guest combined view/collision policy is needed; changing cwd silently is wrong.
- Two checks invoke provider capability operations; one also calls fetch-native.
  Actual endpoint use depends on implementation/config. That fetch may require
  external-origin access, not only local-service egress. A sandbox-denied request
  masquerading as a genuine unavailable provider is not a passing acceptance result.
- Allowing host localhost from a container is not achieved by allowing guest
  localhost. A precise broker/bridge mapping and deny-bypass tests are required.
  Local-service methods can mutate host state or proxy arbitrary requests; a port
  allowlist alone does not establish the required confinement.

## Changes made to the plan

- Physically reordered stable task sections to 9, 10, 12A, 1–8, 11, 12B.
- Split early acceptance-slice verification from full hardening release verification;
  Task 11 predecessor adoption is retained, not lost from the sequence.
- Removed Tasks 3/5/8 as first-slice prerequisites. Narrow world guardian/OS lock,
  bounded lifetime and orphan handling remain immediate containment obligations.
- Added host-owned policy/config modules, scratch build target/home/cache, exact
  relative target alias, writable data copies and project/repository layout.
- Added default-off brokered endpoint rules, host-loopback routing, operation-level
  restrictions, redirect/DNS/address checks and evidence of denied attempts.
- Added real guest build + binary + data mutation + service positive controls,
  not just `/bin/true` tests, plus corresponding call-site sabotage.
- Readiness and observed criterion failures are distinguished from environment,
  policy and containment failures; no promise that these changes alone make all
  frozen checks green. Missing implementation remains a legitimate failed check.
- Added the proposed spec amendment and preserved clean-checkout future builds,
  independent review, ObserveOnly, protected WIP and separately tracked TD-034.

## Resubmission boundary

Approve the revised plan **and** amendment before inline execution. Nothing was
built, deployed or launched; no production source, frozen contract, live config,
protected directory or endpoint was modified. The historical R2a build-provenance
exception remains scoped and does not waive future clean builds.
