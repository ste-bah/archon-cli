# Phase 2 working notes — un-JS the PRD scaffold (in progress)

## STATUS 2026-07-07 (post-milestone)

DONE (committed d35b0632, 7a7093ab, 8899b7ca):
- Rust lifecycle driver executes decomposed-PRD runs natively via
  WorkflowScriptHost::execute (files: workflow_live_v2_lifecycle{,_waves,_impl,
  _verify,_review}.rs + workflow_live_v2_lifecycle_prompts.rs + support/
  remediation/outcomes helper modules). Canary trace identical call-for-call;
  288 workflow bin tests green (same 2 known-red); crate suite unchanged.
- String-surgery splice fns DELETED; payloads baked into body_a.js
  byte-identically (hash stable).

REMAINING for Phase 2 close-out:
1. The scaffold JS (body_a 613 + body_b 189 + noop 62 + remediation 97 +
   contract JS 669) is now generation-and-record only — never executed for
   decomposed runs. Deleting it requires swapping the recorded artifact to a
   Rust-rendered descriptor (hash identity change → bundle/approval/semantics
   test churn). Decide: do now for net-negative LOC, or fold into Phase 5
   legacy demolition (semantics substring validators die at the same time).
   Current Phase 2 net LOC is POSITIVE (+~2.2k driver) until this lands.
2. cargo test -p archon-workflow + ci-gate full run + phase report.
3. Delete this notes file when the phase report lands.

Delete when Phase 2 lands. Goal: rewrite the decomposed-PRD lifecycle as plain
Rust driving `WorkflowV2Scheduler` + result store; QuickJS path remains only
for LLM-authored scripts (`RunTemplate`).

## Source of truth being ported (read in full, port faithfully)

- `workflow_live_generated_scaffold_body_a.js` (501) — main loop
- `workflow_live_generated_scaffold_body_b.js` (189) — review + final gates
- `workflow_live_generated_scaffold_noop.js` (62) — noop evidence matching
- `workflow_live_generated_scaffold_remediation.js` (97) — remediation normalize/ownership
- `workflow_live_generated_scaffold_verification.rs` splice (165) — triage lifecycle (REPLACES body_a's plain verification block between the `let verificationPlan =` marker and `let newlyCompletedIds =`)
- `workflow_live_generated_scaffold_ownership.rs` splice (56) — TO READ
- `workflow_live_generated_contract.rs` (489) — `generated_prd_contract_js()` JS helpers — TO READ
- Prompt strings: every `task:` string in the scaffold is a real prompt; port verbatim.

## Business rules catalogued (do not "improve")

1. `maxDependencyWaves = max(1, canonicalTaskUniverse.size * 3)`.
2. `maxRepairIterations` / `maxInvestigationIterations` from config, clamped 1..=8.
3. Three fixed discovery items: prd-task-review, repository-implementation-audit,
   acceptance-evidence-audit (paths = taskUniverse.source_roots).
4. Inventory items: work_type implementation|verified_noop; implementation needs
   item_id, canonical_task_ids (⊆ universe), dependency_ids (⊆ universe, no
   overlap with own canonical ids), target_files (repo-owned, present),
   acceptance_criteria, focused_verification, artifact_requirements (may be []).
   verified_noop needs acceptance_criteria, noop_proof, noop_proof_refs,
   artifact_requirements.
5. Inventory repair loop, per attempt, in order, each gated by issue kind:
   inventory_shape_repair (≤maxRepair), task_universe_reconcile (≤maxRepair),
   dependency_graph_repair (≤maxRepair), target_file_discovery (≤maxInvest),
   verification_requirements_discovery (≤maxInvest),
   artifact_requirements_discovery (≤maxInvest),
   provider_environment_discovery (≤maxInvest), evidence_repair (≤maxRepair).
   Loop while unresolved_issues non-empty && attempt ≤ max(maxRepair,maxInvest).
   → blocked-malformed-inventory if malformed/unresolved after loop.
   → blocked-empty-implementation-inventory if zero items.
6. Wave loop: readyItems = deps ⊆ completedIds. Deadlock → bounded
   dependency-graph-repair-deadlock loop → blocked-dependency-deadlock (status
   "blocked", not needs_review!). blocked-loop-exhaustion (also "blocked") when
   waves exhausted with remainders.
7. Noop path: noop-proof-verification parallel; failed proofs → bounded
   noop-evidence-repair + reverification; acceptance requires
   outcomeHasNoopSourceEvidence: accepted/noop status + concrete evidence + (if
   source item had artifact_requirements) at least one of artifacts/
   artifact_paths/artifacts_checked/current_artifacts_checked/commands_run/
   current_commands_run/completion_evidence non-empty. → blocked-noop-proof-failed.
8. Implementation wave (fanout worktree, targetFilesFromItem, maxParallelism
   configured). Non-accepted outcomes → remediation-inventory reduce →
   normalizeRemediationInventoryForSources(sourceItems=readyImplementationItems,
   fallback=[], sourceCallId="implementation-wave-N") → bounded
   remediation-empty-inventory-repair → blocked-malformed-remediation if not
   ready. Remediation wave → follow-up loop remediation-outcome-repair-N-K
   (filterRemediationInventoryByTaskIds by original remediation task ids;
   normalize with sources=remediationInventory.items,
   fallback=readyImplementationItems, sourceCallId="remediation-wave-N") →
   blocked-remediation-unresolved if unresolved after ≤maxRepair.
9. Remediation source ownership: match by source_item_id/sourceItemId/
   failed_item_id/failedItemId/item_id/id (with sourceCallId prefix stripping),
   else by unique canonical-task overlap; merge target_files (source wins when
   present), canonical ids fallback, dependency_ids/artifact_requirements/
   focused_verification/acceptance_criteria fallback, source_item_id fallback.
10. Remediation item required fields: item_id, canonical ids, source_item_id,
    failure_status, failure_evidence, required_fix, focused_verification (or
    verification_requirements), target_files present (may be []), artifact
    requirements present.
11. Verification lifecycle (SPLICED version, body_a's inline version is dead):
    verification-plan reduce → normalizeGeneratedInventory → bounded
    verification-plan-repair while !generatedContractVerificationInventoryReady
    → items = generatedContractVerificationItems → blocked-empty-verification.
    verification-wave parallel (itemKind focused_verification). While status
    not accepted/noop && verificationRepairAttempt ≤ maxRepair:
      actionable = outcomes where data.verification_remediation_required===true
        || verification_failure_class==="actionable_implementation_failure"
        || verification_failure_next_action==="write_remediation";
      if actionable: triage reduce → verification-remediation-inventory reduce
        (normalizeRemediationInventoryForSources) → not ready → break;
        remediation-wave-N-verification-K fanout; unresolved → record + break;
        post-remediation-verification-plan (+bounded shape repair) → not ready →
        break; re-verify (verification-wave-N-post-remediation-K); K+=1; attempt+=1;
        continue.
      else: verification-repair-plan reduce → constrain to task ids from plan
        items (generatedContractConstrainInventoryTasks) → bounded
        verification-repair-shape-repair while !ready && unresolved_issues>0 →
        not ready/empty → break; re-run verification-wave-N-K.
    → blocked-verification-failed-N. Accepted ids → acceptedThisWave (∩
    implementationCandidateIdsUnique).
12. Wave completion: newlyCompletedIds empty → wave-completion-evidence-repair
    reduce → matchingAcceptedCompletionIds → still empty →
    blocked-no-completion-N. Else completedIds ∪= newly; remainingItems
    filtered by itemIsCompleted (all canonical ids completed, non-empty);
    implementationWaveIndex increments only if readyImplementationItems>0.
13. Evidence acceptance rule (hasConcreteEvidence): evidence array/object
    non-empty; else completion_evidence/artifact_paths/artifacts/task_coverage/
    commands_run present. acceptedOrNoopCanonicalTaskIdsFrom filters
    status ∈ {accepted, noop} && hasConcreteEvidence, ids ∩ universe.
14. Review lifecycle (body_b): artifact-inventory reduce → saveArtifact.
    adversarial-review-N (reviewer tier); reviewNeedsRemediation: status not
    accepted/noop && (items|residual_gaps|evidence non-empty || summary
    present). Loop while needs remediation && reviewIteration ≤ 6 (HARD CAP 6):
    review-remediation-inventory reduce (input = review.items if non-empty else
    review) → normalizeReviewRemediationInventory (own issue kinds:
    review_remediation_shape_repair/task_reconcile/target_discovery/
    artifact_discovery; artifact-only remediation: target_files==[] requires
    non-empty artifact_requirements) → bounded repair → blocked-empty-review-
    remediation-N; review-remediation-wave-N fanout (worktree);
    review-verification-plan-N reduce → empty → blocked-empty-review-
    verification-N; splitFocusedVerificationItems; review-verification-wave-N
    parallel; not accepted/noop → blocked-review-verification-failed-N;
    re-review adversarial-review-(N+1).
    After loop: still needs remediation → blocked-review-unresolved; status not
    accepted/noop → blocked-review-not-accepted.
15. Final gates (body_b): final-evidence-reconciliation-N reduce; while items
    non-empty && N ≤ maxRepair: completion-claim-repair-N reduce;
    artifact_checks = (artifact_checks||items) having path/artifact_path/
    artifactPath/artifact_id/artifactId → artifact-existence-investigation-N
    parallel; N+=1; re-reconcile (extra input finalEvidenceRepairAttempts).
    Items remain → blocked-final-evidence-reconciliation.
    require-final-artifacts (requireArtifact on artifactInventory) →
    final-zero-gap-audit reduce → final-acceptance-gate qualityGate → not
    accepted/noop → blocked-final-readiness → else final-acceptance-report
    (status accepted).
16. recordRepairAttempt(list, call_id, kind, issues, result) — repairAttempts
    and finalEvidenceRepairAttempts arrays flow into blocked reports.
17. Every blocked finalReport carries `inputs` (evidence bundles) + `task`
    prompt; statuses: needs_review except blocked-dependency-deadlock and
    blocked-loop-exhaustion which use "blocked".
18. Ownership expansion splice: TO READ (ownership-expansion-inventory family).
19. Contract helpers (generatedContract*): TO READ — define present/array/
    rawStrings/issue shapes, target-file validation (repo-owned; rejects task
    docs/context/progress/report/artifact paths), verification item readiness,
    splitFocusedVerificationItems, mergeInventoryRepair,
    generatedContractInventoryGraphIssues, normalizeGeneratedInventory/Item.

## KEY DISCOVERY (2026-07-07)

The contract layer is DUAL-MAINTAINED: `workflow_live_generated_contract.rs`
(+ `_validation.rs`, `_helpers.rs`, `_retry.rs`) is a full Rust implementation
(used live by `workflow_live_v2_source_graph_build.rs:129` and by the contract
tests), while `workflow_live_generated_contract{,_retry,_preflight}.js` (669
lines) is the JS twin injected into the scaffold. Phase 2 = port the
ORCHESTRATION loops; the item/inventory normalization + issue logic already
exists in Rust. The JS twins die with the scaffold.

Ownership expansion splice (read): inside the remediation follow-up loop, when
unresolvedAfterRemediation outcomes have data.ownership_expansion_required ===
true or non-empty data.proposed_ownership_expansions →
ownership-expansion-inventory-N-K reduce (recordRepairAttempt kind
ownership_expansion_inventory) → normalizeRemediationInventory → if ready →
remediation-wave-N-ownership-K fanout (worktree) → merge accepted ids, replace
remediationInventory/Wave, recompute unresolved. Then the blocked check.

## Porting design (decided)

The Rust lifecycle drives the EXISTING `WorkflowScriptHost::execute(method,
payload_json)` — the exact entry point the QuickJS bridge calls. This
preserves bit-for-bit: result-store reuse, dynamic_wave_source_metadata,
input hashing, run-control polling, events/TUI, call records, terminal
accumulator behavior (latch removed in Phase 4, not now). The Rust driver
replaces only the JS interpretation; JS contract helpers are replaced by calls
into the existing Rust contract module. Terminal finalReport host errors
(TERMINAL_HOST_CALL_MARKER path) end the lifecycle exactly as the JS unwind
did. Need to make WorkflowScriptHost constructible without a QuickJS runtime
(it already is — it's plain Rust; the runner holds it).

## Porting design (agreed direction)

- New Rust module (bin crate, near workflow_live_v2): drives the SAME host call
  sequence via the existing per-call execution path the script host uses
  (`execute_v2_live_call` + result store + run-control polling), so events,
  reuse, checkpoints, TUI, and persistence behave identically.
- Reuse dynamic_wave_source_metadata/source fingerprint logic bit-for-bit (call
  through the same helpers the script host uses).
- The scaffold JS files + splice fns + contract JS + semantics substring
  validators get DELETED once the Rust lifecycle passes the existing
  decomposed-PRD integration tests (workflow_live_execution_tests TASK-TDL
  tests + canary shape).
- decomposed_prd_plan_calls() stays as the approval-plan declaration.
- Keep prompts byte-identical (extract every task: string into consts).

## Helper port list (JS-only, no Rust twin yet)

`mergeInventoryRepair`, `generatedContractConstrainInventoryTasks`,
`splitFocusedVerificationItems` (JS orchestration versions),
`generatedContractVerificationInventoryReady` / `generatedContractVerificationItems`
(Rust has `generated_focused_verification_item` — check parity),
remediation family from remediation.js (`normalizeRemediationInventory{,ForSources}`,
source-ownership matching, `remediationItemIssues`, `filterRemediationInventoryByTaskIds`,
`remediationInventoryReady`, `remediationTaskIdSet`),
review family from body_b.js (`normalizeReviewRemediationInventory`,
`reviewRemediationItemIssues`, `reviewNeedsRemediation`, `reviewRemediationInput`),
noop family from noop.js (`outcomeHasNoopSourceEvidence`, `matchingAcceptedNoopIds`,
`matchingAcceptedCompletionIds`), `hasConcreteEvidence`/`acceptedOrNoopCanonicalTaskIdsFrom`/
`nonAcceptedOutcomes`/`matchingAcceptedIds`/`readyItemsFrom`/`itemIsCompleted`/
`validImplementationItem`/`validVerifiedNoopItem` from body_a.js,
`recordRepairAttempt`/`issuesOfKind` (trivial).

## OPEN DESIGN FORK (decide at implementation)

Run-bundle identity: today workflow.js is the recorded artifact and
scaffold_hash = hash(source); metadata/approval/reuse key on it. Options:
(a) Keep generating scaffold JS as the RECORDED artifact + hash basis, execute
    via Rust — minimal test churn but splice fns survive (violates Phase 2
    delete gate for string-surgery fns; could inline splices into the checked-in
    JS bodies to delete the surgery while keeping a static record).
(b) Replace the recorded artifact with a Rust-rendered lifecycle descriptor
    (plan + prompts); scaffold_hash = hash(descriptor). Cleaner end-state,
    larger test churn (bundle/approval/execution tests assert workflow.js).
Leaning (a)-then-(b): inline the two splices into the JS body files textually
(deletes apply_* string surgery immediately, keeps byte-identical scaffold
output → scaffold_hash stable), Rust executes; JS record becomes vestigial and
dies in Phase 5 with a descriptor swap. Verify hash stability with the
plan-snapshot test.

## Execution wiring

`workflow_live_v2_run.rs::execute_generated_v2_run` calls
`WorkflowV2ScriptRunner::new(...).run(&harness)`. Phase 2: when
plan.task_universe.is_some(), call the Rust lifecycle entry instead (same
runner struct, new method `run_decomposed_lifecycle()` driving
`WorkflowScriptHost::execute`). Saved templates keep `.run(harness)`.

## Phase 1 leftovers to remember

- Canary stays red until Phase 4 (artifact contract + error-as-value).
- Pre-existing red: 12 crate failures + generated_v2_read_only_verification_
  branch_stays_read_only (bin) + ci-gate file-size offenders allowlisted.
- WSL: -j 4 + nice 19, one cargo at a time; test-threads=2.
