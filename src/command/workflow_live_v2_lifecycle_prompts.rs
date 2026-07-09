// Prompt strings for the Rust decomposed-PRD lifecycle — ported VERBATIM from
// the scaffold JS `task:` strings (body_a/body_b/noop/remediation/verification
// splice/ownership splice). Do not edit copy without updating the agents'
// structured-output expectations.

pub(crate) const DISCOVERY_TASK: &str = "Run read-only discovery in parallel. Return structured summaries only: files read, commands run, task coverage, implementation gaps, artifact requirements, and risks. Do not accept implementation tasks from read-only work.";

pub(crate) const DISCOVERY_ITEM_PRD_TASK_REVIEW: &str = "Read the PRD, decomposed task files, implementation slice, and context files. Return structured requirements, dependency evidence, task coverage requirements, verification requirements, artifact requirements, and residual risks. Distinguish repository source paths from project artifact/data paths under projectArtifactRoot.";

pub(crate) const DISCOVERY_ITEM_REPOSITORY_AUDIT: &str = "Inspect the target repository for existing implementation relevant to taskText. Return concrete files read, existing evidence, missing work, test commands, and safety concerns. Do not modify files.";

pub(crate) const DISCOVERY_ITEM_ACCEPTANCE_AUDIT: &str = "Map every canonical task in taskUniverse to acceptance criteria, required artifacts, provider/data constraints, and focused verification commands. Use governedLearningContext only as sanitized prior-run hints with evidence refs; verify every claim against current PRD/code. Artifact paths must be checked relative to projectArtifactRoot when they are project artifacts. Do not mark implementation tasks accepted from read-only evidence.";

pub(crate) const CANONICAL_INVENTORY_TASK: &str = "Using taskUniverse, discovery, and governedLearningContext, produce dependency-owned inventory items only for genuinely missing or provably no-op work. Treat learning context as advisory evidence references, not truth. Every item must include work_type. implementation items require item_id, canonical_task_ids, dependency_ids using canonical task IDs only, target_files, acceptance_criteria, focused_verification, and artifact_requirements. artifact_requirements is only for concrete project artifact paths; prose evidence expectations belong in expected_evidence and fix instructions belong in required_fix. target_files must be repo-owned implementation files under targetRepositoryRoot; task docs, context, progress, report, and artifact paths are evidence/artifact refs, not implementation write targets. Split grouped items by dependency readiness: no dependency_id may overlap the same item's canonical_task_ids, and do not group tasks when one claimed task depends on another claimed task. Prerequisite tasks must be represented as implementation or verified_noop items. verified_noop items require item_id, canonical_task_ids, dependency_ids, acceptance_criteria, noop_proof, proof reference entries, and artifact_requirements, using [] only with evidence that no artifact/output is required. Do not emit an empty inventory unless every canonical task has concrete accepted/noop proof.";

pub(crate) const INVENTORY_SHAPE_REPAIR_TASK: &str = "Repair inventory shape only. Preserve taskUniverse canonical IDs and all existing schedulable fields. Return repaired implementation items with work_type, item_id, canonical_task_ids, dependency_ids, target_files, acceptance_criteria, focused_verification, and artifact_requirements. Return repaired verified_noop items with item_id, canonical_task_ids, dependency_ids, acceptance_criteria, noop_proof, noop_proof_refs/proof_references, and artifact_requirements, using [] only with evidence that no artifact/output is required. Do not replace detailed items with skeletons. Put crosscutting/support notes in repair_summary or evidence_refs, not inventory items.";

pub(crate) const TASK_UNIVERSE_RECONCILE_TASK: &str = "Reconcile inventory dependency IDs to canonical task IDs in taskUniverse. Return repaired items or evidence that dependencies are genuinely unresolved.";

pub(crate) const DEPENDENCY_GRAPH_REPAIR_TASK: &str = "Repair dependency graph defects before scheduling. Split grouped items by dependency readiness while preserving or restoring every schedulable field required by the contract. implementation items must include target_files, acceptance_criteria, focused_verification, and artifact_requirements. verified_noop items must include acceptance_criteria, noop_proof, noop_proof_refs/proof_references, and artifact_requirements. Add verified_noop prerequisite items only with concrete proof refs. Remove impossible internal dependency edges only with evidence, or return concrete unrecoverable graph evidence. No dependency_id may overlap the same item's canonical_task_ids. Put crosscutting verification/support notes in repair_summary or evidence_refs, not inventory items.";

pub(crate) const TARGET_FILE_DISCOVERY_TASK: &str = "Investigate missing or invalid target files from PRD/TASK files and repository evidence. Return repaired implementation items whose target_files are repo-owned implementation files under targetRepositoryRoot, or convert to verified_noop with concrete proof refs, or return concrete evidence that safe repo-owned targets cannot be inferred. Do not use task context/progress/report/artifact files as implementation target_files.";

pub(crate) const VERIFICATION_REQUIREMENTS_DISCOVERY_TASK: &str = "Investigate missing acceptance criteria or focused verification requirements. Return repaired items with focused_verification and acceptance_criteria or concrete evidence that requirements are unavailable.";

pub(crate) const ARTIFACT_REQUIREMENTS_DISCOVERY_TASK: &str = "Investigate artifact requirements from PRD/TASK files. Return repaired items with artifact_requirements containing concrete project artifact paths only, using an empty array only when no artifact is required and that no-op is evidenced. Put prose evidence expectations in expected_evidence, not artifact_requirements.";

pub(crate) const PROVIDER_ENVIRONMENT_DISCOVERY_TASK: &str = "Investigate provider/environment evidence without exposing secrets. Return repaired items with redacted env keys checked, commands_run summaries, and provider evidence or concrete external-unavailable evidence.";

pub(crate) const EVIDENCE_REPAIR_TASK: &str = "Repair missing verified no-op evidence fields. Return repaired items with noop_proof and noop_proof_refs/proof_references tied to PRD/TASK criteria, or concrete evidence that no safe no-op proof exists.";

pub(crate) const BLOCKED_MALFORMED_INVENTORY_TASK: &str = "Stop after JS-owned repair/investigation attempts because inventory is still malformed or contains unresolved/non-canonical dependencies. Include repair_attempts with call IDs, issue kinds, canonical task IDs, files read, commands run, artifacts checked, redacted env keys checked, evidence refs, and exact reasons the workflow cannot safely continue.";

pub(crate) const BLOCKED_EMPTY_INVENTORY_TASK: &str = "Stop because the inventory is empty. Accept this only if every canonical task has concrete accepted/noop evidence; otherwise report the missing inventory evidence.";

pub(crate) const DEADLOCK_GRAPH_REPAIR_TASK: &str = "Repair a dependency deadlock before terminal stop. Split grouped items by dependency readiness, add verified_noop prerequisite items with concrete proof refs, or return concrete unrecoverable graph evidence with files read, commands/artifacts checked, and canonical task IDs.";

pub(crate) const BLOCKED_DEPENDENCY_DEADLOCK_TASK: &str = "Stop because no remaining implementation items are dependency-ready after bounded JS-owned dependency graph repair. Report exhausted graph repair evidence and next valid restart/remediation choices.";

pub(crate) const NOOP_PROOF_VERIFICATION_TASK: &str = "Verify the assigned dependency-ready no-op proof against PRD/TASK acceptance criteria. Do not modify files. Return item_id, canonical_task_ids, status accepted/noop only with concrete proof refs, artifacts checked, commands if any, and residual gaps.";

pub(crate) const NOOP_EVIDENCE_REPAIR_TASK: &str = "Repair failed dependency-ready no-op proof into re-verifiable no-op items or concrete proof gaps. Preserve canonical task IDs and return items only when further verification can be safely attempted.";

pub(crate) const NOOP_PROOF_REVERIFICATION_TASK: &str = "Re-verify repaired dependency-ready no-op proof against PRD/TASK criteria. Do not modify files. Return item_id, canonical_task_ids, accepted/noop only with concrete proof refs, artifacts checked, commands if any, and residual gaps.";

pub(crate) const BLOCKED_NOOP_PROOF_FAILED_TASK: &str = "Stop because dependency-ready no-op proof did not produce accepted/noop evidence after bounded JS-owned evidence repair and re-verification. Report exact proof gaps.";

pub(crate) const IMPLEMENTATION_WAVE_TASK: &str = "Implement only the assigned dependency-ready item. Return one structured outcome per item with item_id, canonical_task_ids, snake_case status, evidence, changed files, commands/tests, artifacts, and residual gaps. Use accepted or noop only with concrete task-linked proof.";

pub(crate) const REMEDIATION_INVENTORY_TASK: &str = "Create remediation items only for non-accepted/non-noop implementation outcomes from the current wave. Reuse target_files from the original readyImplementationItems/source item; do not target workflow.js, branch JSON, task docs, or artifacts unless the source item explicitly owned them. Each remediation item must include item_id, source_item_id, canonical_task_ids, target_files, failure_status, failure_evidence, required_fix, verification_requirements, dependency_ids, focused verification, and artifact requirements. Use target_files only for repo-owned implementation file paths; artifact-only remediation must set target_files to [] and put concrete project artifact paths in artifact_requirements. Never place instructions, evidence guidance, or prose in target_files or artifact_requirements; prose evidence belongs in expected_evidence and fix instructions belong in required_fix.";

pub(crate) const REMEDIATION_EMPTY_INVENTORY_REPAIR_TASK: &str = "Repair an empty or malformed remediation inventory into actionable remediation items for the non-accepted implementation outcomes, preserving canonical task IDs, target files, verification, and artifact requirements, or return concrete evidence that no safe remediation can be inferred.";

pub(crate) const BLOCKED_MALFORMED_REMEDIATION_TASK: &str = "Stop because implementation produced non-accepted outcomes but bounded JS-owned remediation inventory repair remained empty or malformed. Report exact unresolved outcomes and contract issues.";

pub(crate) const REMEDIATION_WAVE_TASK: &str = "Remediate only the assigned unresolved item. Return item_id, canonical_task_ids, snake_case status, concrete evidence, changed files, commands/tests, artifacts, and residual gaps.";

pub(crate) const REMEDIATION_OUTCOME_REPAIR_TASK: &str = "Repair unresolved remediation outcomes into a follow-up remediation inventory, preserving original source-owned target files, canonical task IDs, failure_status, failure_evidence, required_fix, verification, and artifact requirements. Do not infer target files from workflow scripts, branch result JSON, or artifact paths. Use target_files only for repo-owned implementation file paths; artifact-only remediation must set target_files to [] and put concrete project artifact paths in artifact_requirements. Never place instructions, evidence guidance, or prose in target_files or artifact_requirements; prose evidence belongs in expected_evidence and fix instructions belong in required_fix.";

pub(crate) const FOLLOWUP_REMEDIATION_WAVE_TASK: &str = "Run follow-up remediation only for the assigned unresolved item. Return item_id, canonical_task_ids, snake_case status, concrete evidence, changed files, commands/tests, artifacts, and residual gaps.";

pub(crate) const OWNERSHIP_EXPANSION_INVENTORY_TASK: &str = "Review unresolved write-branch evidence that proposes explicit repo path ownership expansion. Return follow-up remediation items only when each added path is an exact repo-owned file needed for the same canonical task and supported by branch evidence. Preserve existing target_files, append only justified explicit files, keep dependency_ids canonical, include focused_verification and artifact_requirements, and do not broaden by directory, language, framework, glob, or project artifact path.";

pub(crate) const OWNERSHIP_EXPANSION_WAVE_TASK: &str = "Run follow-up remediation only for the assigned unresolved item after JS-owned explicit ownership expansion. Return item_id, canonical_task_ids, snake_case status, concrete evidence, changed files, commands/tests, artifacts, and residual gaps.";

pub(crate) const BLOCKED_REMEDIATION_UNRESOLVED_TASK: &str = "Stop because bounded JS-owned remediation attempts left unresolved implementation outcomes. Report exact remaining gaps and next valid restart/remediation choices.";

pub(crate) const VERIFICATION_PLAN_TASK: &str = "Plan focused verification for newly implemented or remediated canonical task IDs before dependents can unblock. Return one verification item per exact command/check whenever possible. Each item must include item_id, canonical_task_ids, focused_verification, expected_evidence, optional artifact_requirements, and source_item_id. Artifact checks must enumerate exact artifact fields or paths; never fail on vague requirements such as all required artifact path fields or all expected artifacts unless the item lists them explicitly. Check project artifacts relative to projectArtifactRoot when present, and repository source files relative to targetRepositoryRoot. When provider/API credentials matter, include provider_env_requirements with exact env key names and require redacted provider_env_proof; never include values. Evidence is accepted when at least one intended target passes and no intended target fails; duplicate cargo workspace/lib/bin harness pass sections are valid as one canonical pass.";

pub(crate) const VERIFICATION_PLAN_REPAIR_TASK: &str = "Repair an empty or malformed focused verification plan into concrete verification items, preferably one item per exact command/check, with item_id, canonical_task_ids, focused_verification, expected_evidence, optional artifact_requirements, and source_item_id. Artifact checks must enumerate exact fields or paths; do not use vague all required artifact path fields or all expected artifacts wording. Check project artifacts relative to projectArtifactRoot when present. Include provider_env_requirements and redacted provider_env_proof requirements for provider-sensitive checks, or return concrete evidence that verification cannot be inferred safely.";

pub(crate) const BLOCKED_EMPTY_VERIFICATION_TASK: &str = "Stop because bounded JS-owned repair could not produce a focused verification plan for newly implemented work.";

pub(crate) const VERIFICATION_WAVE_TASK: &str = "Run focused verification only. Return structured status, item_id, canonical_task_ids, focused_verification executed, commands run with exit_code and output_summary, matched test/check names, pass/fail count, artifacts checked, source_item_id, verification_failure_class when failed, and residual gaps. Accept duplicate cargo workspace/lib/bin harness pass sections as one canonical pass when at least one intended target passes and no intended target fails. Do not modify files.";

pub(crate) const VERIFICATION_FAILURE_TRIAGE_TASK: &str = "Classify failed focused verification outcomes. Return actionable implementation failures as items requiring write remediation, retryable verification-shape issues as retry_items only for checks that actually require rerun, and accepted sibling-covered failures as superseded_items. retry_items must include item_id, source_item_id, canonical_task_ids, focused_verification or recommended_retry, and expected_evidence. Allowed classifications include retryable_verification_shape_issue, retry_resolved_by_sibling_evidence, and retry_resolved_verification_execution_issue. Return terminal external/runtime/safety blockers with concrete evidence. Do not mark tasks complete.";

pub(crate) const VERIFICATION_REMEDIATION_INVENTORY_TASK: &str = "Create write-capable remediation items only for actionable implementation failures proven by focused verification. Every item must include item_id, source_item_id, canonical_task_ids, dependency_ids using canonical task IDs only, target_files from the original implementation item ownership, failure_status, failure_evidence, required_fix, focused_verification, and artifact_requirements.";

pub(crate) const VERIFICATION_REMEDIATION_WAVE_TASK: &str = "Fix only the assigned focused-verification failure. Use the original implementation item's ownership. Return item_id, canonical_task_ids, snake_case status, concrete evidence, changed files, commands/tests, artifacts, and residual gaps.";

pub(crate) const POST_REMEDIATION_VERIFICATION_PLAN_TASK: &str = "Plan focused verification after write remediation. Return one item per exact command/check with item_id, canonical_task_ids, focused_verification, expected_evidence, optional artifact_requirements, and source_item_id. Include provider_env_requirements for provider-sensitive checks and require redacted provider_env_proof only.";

pub(crate) const POST_REMEDIATION_VERIFICATION_PLAN_REPAIR_TASK: &str = "Repair malformed post-remediation verification output into canonical focused verification items with item_id, canonical_task_ids, focused_verification, expected_evidence or artifact_requirements, and source_item_id.";

pub(crate) const POST_REMEDIATION_VERIFICATION_WAVE_TASK: &str = "Run focused post-remediation verification only. Return structured status, item_id, canonical_task_ids, commands run with exit_code and output_summary, matched checks, pass/fail count, artifacts checked, source_item_id, verification_failure_class when failed, and residual gaps. Do not modify files.";

pub(crate) const VERIFICATION_REPAIR_PLAN_TASK: &str = "Repair failed focused verification shape into a concrete retry plan only when the issue is missing/malformed verification evidence, target selection, or retryable command shape. Do not use verification repair for product-code or artifact-contract failures; those require write remediation.";

pub(crate) const VERIFICATION_REPAIR_SHAPE_REPAIR_TASK: &str = "Repair malformed focused verification retry output into canonical items. Return items with item_id, canonical_task_ids, focused_verification, expected_evidence or artifact_requirements, source_item_id, and provider_env_requirements when provider credentials are required. Artifact checks must enumerate exact fields or paths and must not use vague all required artifact path fields or all expected artifacts wording. Project artifact checks must name projectArtifactRoot-relative paths when possible.";

pub(crate) const RETRY_VERIFICATION_WAVE_TASK: &str = "Run repaired focused verification only. Return structured status, item_id, canonical_task_ids, focused_verification executed, commands run with exit_code and output_summary, matched test/check names, pass/fail count, artifacts checked, source_item_id, verification_failure_class when failed, and residual gaps. Do not modify files.";

pub(crate) const BLOCKED_VERIFICATION_FAILED_TASK: &str = "Stop because bounded JS-owned verification triage, verification repair, and write remediation did not produce accepted/noop evidence. Report exact commands/results, remediation attempts, and unresolved gaps.";

pub(crate) const WAVE_COMPLETION_EVIDENCE_REPAIR_TASK: &str = "Repair a dependency-ready wave that produced no newly completed canonical task IDs. Return concrete completion evidence to re-check, or exact evidence-backed gaps that prevent safe unblocking.";

pub(crate) const BLOCKED_NO_COMPLETION_TASK: &str = "Stop because the dependency-ready wave produced no new verified accepted/noop canonical task IDs after JS-owned completion evidence repair.";

pub(crate) const BLOCKED_LOOP_EXHAUSTION_TASK: &str = "Stop because the bounded dependency loop ended with unresolved implementation items. Report unresolved task IDs and dependency evidence.";

pub(crate) const ARTIFACT_INVENTORY_TASK: &str = "List every required generated dataset, registry entry, validation report, coverage matrix, Pine artifact, backtest artifact, and final evidence artifact. Include only real paths that should exist.";

pub(crate) const SAVE_ARTIFACT_INVENTORY_TASK: &str =
    "Persist the artifact inventory and evidence references for final acceptance.";

pub(crate) const ADVERSARIAL_REVIEW_TASK: &str = "Run read-only adversarial review against the PRD and every TASK acceptance criterion. Use governedLearningContext only to check known prior failure classes. Return accepted/noop only if no gaps remain; otherwise return issue items requiring remediation. Do not modify files.";

pub(crate) const ADVERSARIAL_RE_REVIEW_TASK: &str = "Re-run read-only adversarial review. Use governedLearningContext only to prevent repeating known evidenced failure classes. Return accepted/noop only when all PRD/TASK criteria have concrete evidence and artifact paths exist. Do not modify files.";

pub(crate) const REVIEW_REMEDIATION_INVENTORY_TASK: &str = "Turn adversarial review issues into dependency-safe remediation items. Every item must include item_id, canonical_task_ids using taskUniverse canonical task IDs only, dependency_ids using canonical task IDs only, source_item_id, failure_status, failure_evidence, required_fix, target_files, focused_verification, and artifact_requirements. Use target_files only for repo-owned implementation files; artifact-only remediation must set target_files to [] and put concrete project artifact paths in artifact_requirements. Prose evidence expectations belong in expected_evidence, not artifact_requirements. Do not invent remediation IDs as canonical_task_ids.";

pub(crate) const REVIEW_REMEDIATION_INVENTORY_REPAIR_TASK: &str = "Repair review remediation inventory shape before write fanout. Return full replacement items only. Use canonical taskUniverse IDs in canonical_task_ids/dependency_ids. Include target_files only for repo-owned source edits; use [] for artifact-only remediation with concrete artifact paths in artifact_requirements. Preserve failure context, required_fix, focused_verification, and artifact requirements. Put prose evidence expectations in expected_evidence. Do not invent synthetic canonical task IDs.";

pub(crate) const BLOCKED_EMPTY_REVIEW_REMEDIATION_TASK: &str = "Stop because adversarial review found issues but review remediation inventory is still empty or malformed after JS-owned repair.";

pub(crate) const REVIEW_REMEDIATION_WAVE_TASK: &str = "Fix only the assigned adversarial review issue. Return canonical task IDs, status, concrete evidence, commands/tests, artifacts, and residual gaps.";

pub(crate) const REVIEW_VERIFICATION_PLAN_TASK: &str = "Plan focused verification for review remediation before final review can pass. Return one item per exact command/check whenever possible. Every item must include item_id, canonical_task_ids, dependency_ids using canonical task IDs only, source_item_id or source_call_id when available, focused_verification, expected_evidence, and optional artifact_requirements. Evidence is accepted when at least one intended target passes and no intended target fails; duplicate cargo workspace/lib/bin harness pass sections are valid as one canonical pass.";

pub(crate) const BLOCKED_EMPTY_REVIEW_VERIFICATION_TASK: &str =
    "Stop because review remediation had no focused verification plan.";

pub(crate) const REVIEW_VERIFICATION_WAVE_TASK: &str = "Run focused verification for review remediation. Return commands with exit_code and output_summary, matched test/check names, pass/fail count, artifacts checked, and residual gaps. Accept duplicate cargo workspace/lib/bin harness pass sections as one canonical pass when at least one intended target passes and no intended target fails. Do not modify files.";

pub(crate) const BLOCKED_REVIEW_VERIFICATION_FAILED_TASK: &str =
    "Stop because review remediation verification failed.";

pub(crate) const BLOCKED_REVIEW_UNRESOLVED_TASK: &str =
    "Stop because adversarial review issues remain after bounded remediation.";

pub(crate) const BLOCKED_REVIEW_NOT_ACCEPTED_TASK: &str =
    "Stop because adversarial review did not explicitly accept the evidence.";

pub(crate) const FINAL_EVIDENCE_RECONCILIATION_TASK: &str = "Reconcile final completion claims, artifact existence, verification evidence, provider/data evidence, and residual gaps before final acceptance. Return issue items only for unsupported completion claims, missing artifacts, or repairable evidence gaps.";

pub(crate) const COMPLETION_CLAIM_REPAIR_TASK: &str = "Repair unsupported completion claims into concrete evidence requirements or residual gaps. Return updated evidence references and artifact checks without claiming completion without proof.";

pub(crate) const ARTIFACT_EXISTENCE_INVESTIGATION_TASK: &str = "Investigate artifact existence and path evidence. Return artifact paths checked, existence status, commands run if any, and residual gaps. Do not modify files.";

pub(crate) const FINAL_EVIDENCE_RE_RECONCILIATION_TASK: &str = "Re-run final evidence reconciliation after completion-claim repair and artifact investigation. Return no issue items only when all completion claims have concrete proof.";

pub(crate) const BLOCKED_FINAL_EVIDENCE_RECONCILIATION_TASK: &str = "Stop because final evidence reconciliation still has unsupported completion claims, missing artifacts, or repairable evidence gaps after bounded JS-owned repair and investigation.";

pub(crate) const REQUIRE_FINAL_ARTIFACTS_TASK: &str = "Verify all referenced artifact paths required by the PRD and TASK files exist after final evidence reconciliation and artifact investigation.";

pub(crate) const FINAL_ZERO_GAP_AUDIT_TASK: &str = "Perform final zero-gap acceptance audit across all canonical tasks, PRD criteria, code changes, tests, provider/data evidence, artifacts, residual gaps, repair attempts, final evidence reconciliation, and governedLearningContext failure classes.";

pub(crate) const FINAL_ACCEPTANCE_GATE_TASK: &str = "Accept only if every canonical task has concrete implementation or verified no-op evidence, focused tests, existing artifact paths, and no residual blocking gaps.";

pub(crate) const BLOCKED_FINAL_READINESS_TASK: &str =
    "Stop because final acceptance gate did not accept the evidence. Report exact residual gaps.";

pub(crate) const FINAL_ACCEPTANCE_REPORT_TASK: &str = "Produce the final acceptance report with canonical completed task IDs, source files changed, artifact paths, focused test commands/results, provider/data evidence, residual gaps if any, and explicit confirmation that no task was accepted without concrete evidence.";
