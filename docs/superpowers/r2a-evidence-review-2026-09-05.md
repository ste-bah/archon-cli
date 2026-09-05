# R2a evidence review and R2b entry gate

Reviewed against source HEAD `0c81c4321` on 2026-09-05.
Reviewer: this session; evidence inspection only, not a fresh independent source-code review.
No Cargo, live workflow, deployment, source implementation or protected-file changes were performed.

## Decision

**Functional proof evidence verified. Operator accepted the scoped historical build-provenance exception on 2026-09-05; R2b planning may proceed.**
Do not invalidate the successful runs, re-freeze their task set, or launch another
proof merely because build provenance is not attached. First recover the existing
build record. R2b implementation still requires approval of its plan; the external task implementation is not authorized.

Authority: `specs/2026-08-27-decomposition-r2-engine-native-design.md`, especially
lines 80–127 (R2b), 895–925 (proof-two preconditions/evidence), and 982–997 (handoff).

## Evidence inspected and independently checked

### Synthetic clearance actually used by proof two

Root: `/private/tmp/archon-r2a-evidence-1788560433/synthetic`.
This is the later synthetic rerun, not the earlier successful run 23.

- 86/86 manifest entries exist and match byte lengths and SHA-256 hashes.
- Manifest digest recomputed from the canonical package/identity/entries tuple:
  `6e16e8c2da50ee60a54035c964416c5cdac9c3bff74c9a995323fc02849ea95e`.
- Runtime revision: `ff7a0ce8a`; full source revision, binary SHA-256, script and
  catalog identities are recorded in `identity.json` and the manifest.
- `review.json` records an approved independent review with zero unresolved
  critical/important findings. Its pre-review manifest digest and review artifact
  hash match the preserved bytes. The review is evidence, not a new instruction.
- The preserved harness output at `/private/tmp/archon-r2a-proof2/proof1-rerun.out`
  reports 1 passed, 0 failed in 11,497.76 seconds.
- Decomposition `wf-085ff395-71d2-4863-9d92-740998772092` ended needs_review for
  the intentionally prescribed commandless observer floor, as classified by the
  prior independent review. This is not the external decomposition's clean criterion.
- Implementation `wf-09a73229-8c17-43b1-9a87-5863f8ea2412` persisted its terminal
  event at sequence 79; observer start is 80 and policy shadow is 81, both
  `observe_only`. Observer record reports the prescribed missing target.
- `authority-probe.json` records exit 0 for
  `global_enforce_mode_cannot_promote_observer_authority`; its output hash and
  runtime identity binding are present in the verified package.
- Earlier run-23 package `/private/tmp/archon-r2a-evidence-1788425279/synthetic`
  also has 82/82 matching entries, but is not substituted for the actual clearance.

### External decomposition, run 12

Preservation root:
`/Volumes/Externalwork/project1-quarantine-20260826/PRD-TRADING-DATA-LAKE-AHDM-001.proof2-run12-COMPLETED-20260905-KEEP`.
Run: `wf-c040450a-7ed1-46e1-8849-a7873a450203`.

- 129/129 `evidence-external/manifest.json` entries match length and SHA-256.
- Canonical manifest digest matches
  `7268a930b619a600f35c849b43427561dc4295a0a614339f0d03982292766fb8`.
- Preserved harness: 1 passed, 0 failed, 4,839.39 seconds.
- Terminal snapshot says `completed`; terminal event sequence 265 is
  `stage_completed` with `detail.event=terminal_status` and accepted v3 status.
- Acceptance: 11 command checks; all verdicts accepted with temperature 0,
  resolved model and provider recorded. Logical attempt 2; lock finding count 0.
- Skeleton: 15 tasks, logical attempt 3; lock finding count 0.
- All 15 bodies have final accepted dispositions; two required a second attempt.
- Both final set-gate envelopes have no policy findings and satisfied postconditions.
- Acceptance/skeleton locks recompute correctly with BLAKE3. The pin binds both.
- 21 unique publication receipts found; all 41 latest destination entries match
  current bytes, lengths and BLAKE3. The entire preserved task-root copy matches
  the live task-root files; no frozen artifact was changed for this review.
- Pause sequence 27 and resume sequence 30 preserve the interrupted skeleton
  attempt. Explicit `detail.event=call_reused` records at 33, 37, 39 and 42 name
  the two author calls and two freeze calls. Auxiliary host progress events mark
  reused=false even for replay; use the explicit reuse events as authority.
- Protected before/after snapshots compare equal. The source trading WIP tracked
  diff SHA-256 remains `acdacc0ffb24f98358da39da5e98019f079277876284ccf84faa7a78682bd41e`.
  This is a present comparison to the recorded earlier hash, not build provenance.

## Shadow review

The copied project shadow JSONL contains 906 historical rows, not 906 findings
from run 12. Joining exact call IDs from run 12 yields 17 publication shadows:

- 6 acceptance refutations on the first judged candidate: checks too narrow for
  their criteria. The subsequent candidate is accepted with zero findings.
- 11 skeleton findings: unknown claimed obligations and missing requirement
  ownership. The third skeleton candidate clears them all.

These are repaired candidate defects, not unexplained final false positives.
Three additional refused, unpublished candidates appear in call envelopes: one
non-canonical skeleton ID and two body candidates binding no frozen subject.
They are not publication-shadow rows. TD-034 correctly tracks their attempt-budget
cost; it is not silently marked fixed by the clean final task set.

## Identity and release limits

1. Proof two ran on `192045760`, not current HEAD `0c81c4321`. The latter is a
   ledger-only successor (`git log 192045760..0c81c4321`). Both installed binaries
   currently report `0c81c4321`; that does not rewrite the proof's runtime identity.
2. `clearance-identity-override.json` explicitly records use of older synthetic
   clearance via `ARCHON_R2A_ACCEPT_PRIOR_CLEARANCE`. This is a visible operator
   exception, not same-identity synthetic coverage of every later engine change.
3. The original acceptance-loop requirement for two consecutive convergent runs
   and identical remote verdicts has not been demonstrated by this one successful
   external run. Temperature-zero records are not proof of remote determinism.
4. The release precondition at spec line 902 requires a clean committed-source
   build/snapshot excluding protected WIP, or a complete content-addressed manifest
   of compiled inputs. Neither verified package includes that evidence. The
   retained `chain-proof2.sh` builds directly from the source checkout; its revision
   check proves the embedded revision, not absence of dirty compiled inputs.
   **This review cannot certify that precondition from the available records.**
5. No fresh post-fix independent source-code review or full primitive/UI/legacy
   regression report is established merely by these live harness passes. Earlier
   test totals and the synthetic review have their own revisions/scopes.

Required next evidence: recover the build-source snapshot/manifest and final
review/test record for `192045760`, or have the operator explicitly decide the
release exception. Do not manufacture retrospective compiled-input provenance.
No additional live run has been requested or scheduled.

## R2b planning boundary

R2b preserves v3, symbolic host capabilities, frozen-chain kernels, legacy silence
and ObserveOnly authority. It is not the 15-task implementation and is not R3/R4.
The operator subsequently accepted the historical exception below. The proposed implementation plan is now at `plans/2026-09-05-decomposition-r2b-hardening.md`; execution remains subject to plan approval.
The approved scope naturally separates into the following dependency-ordered units:

1. **Durable ownership/state/events:** extend `crates/archon-workflow/src/store.rs`
   and `events.rs` using the existing OS locks, not a second lock system. Add
   generation-CAS transactions and atomic event allocation/append. Existing
   `with_run_lock` is real; separate `next_event_seq` then `emit` is not an atomic
   allocation/append transaction. Test with competing processes and crash cuts.
2. **Publication recovery and canonical task-root leases:**
   `src/command/workflow_host_command_publish.rs`, `workflow_task_set_publish.rs`,
   `workflow_host_command_exec.rs`, and fixed launcher/resume. Add retained-dirfd
   no-follow writes, expected-prior CAS, durable composite journal, exact committed
   receipt adoption and stale owner recovery. Tests kill at each rename/fsync/receipt
   boundary and prove no second judge execution for committed output.
3. **Executable identity and dispatch recovery:** catalog resolution/supervision,
   `workflow_decompose_resume.rs` and the live agent dispatch boundary. Run-owned
   executable snapshot, image handshake, parent-death handling, AuthorDispatchLedger,
   active-time heartbeats and prepared-result recovery. Never claim provider-side
   exactly-once after an unacknowledged remote request without provider support.
4. **Progress and finalization recovery:** `workflow_decompose_progress.rs`,
   `workflow_decompose_log.rs`, `workflow_live_v2_finalizer.rs` and
   `crates/archon-workflow/src/v2/finalization.rs`. Durable progress outbox;
   exclusive stale-recoverable observer claim; startup finalization-only recovery.
   Preserve terminal-before-observer, no terminal-status mutation, and LegacyAbsent
   silence. Race two finalizers and kill at every durable boundary.
5. **Isolated acceptance execution:** `workflow_run_end_observer.rs` plus a new
   small host-owned world adapter. Select and prove an actual isolation backend
   before implementation: read-only project/repository/task mounts, scratch-only
   tmpfs, no network/credentials/host sockets/devices, bounded resources and whole-
   world teardown. A process group or a successful Docker executable lookup is not
   this proof. Commands enter stdin only; no host fallback. All three command-bearing
   shapes must use the same adapter and the existing declarative/residual kernels.
6. **Explicit predecessor adoption:** new portable AdoptedPredecessorReceipt and
   explicit launcher policy, integrated with ordinary freeze validators and the
   new ownership/CAS layer. Bind exact chain bytes, policy results, shadow IDs,
   adopter run/binary/script/catalog and stable event identity. No mutation or
   skip on internal consistency alone; legacy admission remains untouched.

One unresolved execution design must be settled rather than hidden by a stub:
current authored command checks can build the repository and write project-local
outputs. The approved world permits writes only in scratch. A read-only mount
alone cannot run those checks successfully. The focused world design must specify
scratch staging/toolchain provisioning and path semantics without granting a host
write or quietly making a required read-only mount writable. Do not edit frozen
checks or weaken the isolation requirement to make the plan look executable.

After the release evidence is accepted, produce the focused R2b implementation
plan(s) with exact interfaces, red tests and call-site sabotage, covering all six
units. Do not treat this sequencing note as an implementation authorization.

## Build provenance recovery follow-up

The contemporaneous builder transcript was located at
`/Users/stevenbahia/.claude/projects/-Volumes-Externalwork/afe794af-3575-41fc-bceb-bfb96c825201.jsonl`.
Relevant tool records:

- `toolu_01LoHNBfu2WhmLz2k1jf2XSo`, 2026-09-05T18:10:59Z: explicitly stages
  the four TD-033 files, commits, prints HEAD, and invokes
  `/private/tmp/archon-r2a-proof2/chain-proof2.sh`.
- Completion retrieved by `toolu_01UAVrXfTCaaMzGmUgsGDRxw`,
  2026-09-05T18:21:58Z: HEAD `192045760`, binary hash prefix
  `5e51d55984ef7d5d`, harness paths, and launch of the preserved successful run.
- `toolu_01235aiH6KmE4YRpnybWjrz1`, 2026-09-05T18:22:03Z: deployed version
  `archon 1.9.3 (192045760)`.

The invoked script changes directory to the ordinary source checkout and runs
`cargo build --release`; it does not create or use a clean source snapshot and
it does not collect a compiled-input manifest. Root `Cargo.toml:211` includes
`archon-trading` as a path dependency. Revision embedding in `build.rs` reads
HEAD, not working-tree content. This establishes the build route and the tested
binary identity; it does not establish exact dirty source bytes at compile time.
No retrospective manifest was fabricated, and no source snapshot was rebuilt.

The remaining decision is an explicit operator release exception: accept the
identified, functionally proven binary as the R2a baseline despite incomplete
compiled-input provenance, or retain the gate pending clean-build evidence.
An exception must remain scoped to this historical proof and must not weaken
future clean-build or source-manifest requirements. The decision is recorded below; implementation and external-task execution are not implied.


## Operator decision — 2026-09-05

The operator selected **Accept scoped exception** in response to the explicit
question about accepting the identified passing proof-two binary despite incomplete
compiled-input provenance. This permits completing the R2b plan. It does not
retroactively prove clean compilation, claim same-revision synthetic coverage,
prove two-run remote judge reproducibility, or waive independent code review and
future clean-build requirements. Preserve the historical evidence and its limitation.
No R2b or external-task implementation was authorized by this selection.


## R2b plan review update

Fable's review confirmed the acceptance-world execution gap and unnecessary
critical-path dependencies. The proposed replacement is documented in
`specs/2026-09-05-r2b-acceptance-world-amendment.md` and the revised plan now starts
9 → 10 → 12A, with general hardening and adoption afterward. This supersedes the
original sequencing note above, not the verified historical proof observations.
Host-configured scratch build/data mapping and default-deny brokered services
require explicit amendment approval before inline implementation. See
`r2b-fable-plan-review-2026-09-05.md` for the code/contract-grounded review response.
