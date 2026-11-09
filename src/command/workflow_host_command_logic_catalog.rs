#[path = "workflow_host_command_logic_catalog_history.rs"]
mod history;

use super::{CONTRACT, CapabilityLogic, FREEZE, GATE, LINT, TRACE, VERIFY};
// Windows CPU fix re-pin only (freeze, land/lint and verify digests): utf8Bytes counts bytes with native string calls and namedFilesContext sizes the file list incrementally; same bytes, no verdict rule changed.
pub(crate) const CAPABILITY_LOGIC: &[CapabilityLogic] = &[
    CapabilityLogic {
        id: "freeze-acceptance",
        // Integration 2026-11-07 re-pin only (#611 #613 #623-#625 #627 #630 #634): closure files changed for acceptance author scheduling, macOS process identity, skeleton landing/seed reply rules (read only by skeleton reply validation before the gate) and tests; no verdict rule here changed.
        // #615, same version 85: a multi-filter cargo invocation matches each filter; inventory probe findings change.
        // Earlier bump and re-pin notes: workflow_host_command_logic_catalog_history.rs.
        version: 89, // #572 S7: floors judged by one strict native kernel (D-S7-10) (was 88: #572 S5: base snapshot from git at B, isolated base copy, typed baseline refusals (was 87: #572 S4a: acceptance lock records per-check base results (A4 pass/fail) (was 86: integ-1108: #572 S1 judge sees covered requirement text, every PRD id coverable with new classes diagnostic only, refuted covers drop only with a route; #567 parse refusal before the judge; #572 S3 floor implements cleared (was 85: #618: screen host file move/rename probes before forwarding refutations.))))
        // #584 P5 bump: republish now refuses legacy skeleton entry and constraint defects; operator re-pins digest.
        sources: &[GATE, CONTRACT, FREEZE],
        // #386 re-pin only: a candidate past the inline stdin bound reaches the child whole through a run spill; a candidate that fits is read as before, and one past the bound never had a verdict.
        // #442 re-pin only: the base-tree suffix lookup moved into RepositoryTree::paths_ending_with; freeze verdicts do not read claims, and requirements-trace's version bump in this table does not change them.
        // #446 follow-up re-pin only (version 18 at the time): cache compatibility now keeps old evidence;
        // Scratch schema-3 verdicts stay reusable, while Hermetic layoutless
        // evidence reruns under the version-18 acceptance pin. Version 19 records #402/#419 changes.
        // #457: baseline-passing checks may use a bound accepted judge verdict; the can-fail proof remains required.
        // #457 follow-up re-pin only: directory and variable flow jointly refine input discovery; freeze-acceptance is version 30 after the #444 merge.
        // #457 follow-up re-pin only: transient recreation of moved check inputs invalidates a stored verdict; freeze-acceptance remains version 28 (unreleased).
        // #448 re-pin only: acceptance_scratch_hooks.rs, acceptance_scratch_observe.rs, acceptance_scratch_guardian.rs and workflow_decompose.rs route resolved cargo limits into check execution; these limits affect execution environment and parallelism, not verdict rules.
        // 34: #420 keeps same-spelled siblings from different roots distinct,
        // so a false Scratch revision conflict no longer voids a check.
        // 35: sibling revisions are part of verdict reuse identity; changed
        // sibling inputs cannot reuse an earlier check result.
        // 36: sibling paths in inline and here-doc programs are copied into
        // each check site, and a failed mutation setup path stays explicitly
        // unproven rather than disappearing from the check's report.
        // #464 re-pin only: catalog version entries changed; freeze verdict logic is unchanged.
        // #468 follow-up re-pin: complete per-input state, transient recreation markers and status markers route recreated inputs to the author, unrestored inputs to host-unproven, and only restored inputs to the normal verdict; persistence still requires restoration markers.
        // #466 re-pin only: mutation evidence is discarded unless every moved input is restored.
        // #469 re-pin only: reply marker stripping and midline-fence diagnosis change source closure; freeze behavior version stays fixed.
        // #471 re-pin only: structured mutation-input diagnostics distinguish text-derived writes from observed recreation; findings change, verdict rules do not. The helper split only moves this source closure.
        // #472 re-pin only: reply unwrapping and mixed-form refusal text; verdict rules unchanged.
        // Re-pinned only (verdict rules unchanged): #434 #417 #439 #425 #438 #424 #369 #383 #422 #386 #387 #442 #443 #441 #447 #446 #340 #457 #444 #448 #219 #464 #467 #466 #468 #469 #471 #472
        // #475 re-pin only: strike identity helpers moved to their own file.
        // 41: restore passability finding selection while recognizing the
        // distinct baseline-pass lead; both can change freeze findings.
        // Re-pinned only (verdict rules unchanged): #476 #477 #479 #480
        // Re-pinned only (verdict rules unchanged): regression-fix diagnostic routing and its fixture.
        // Re-pinned for #487: acceptance judge source changed with version 44.
        // Re-pin only: resume lineage repairs and missing cross-root refusal do not change the freeze verdict.
        // 50: canonical symlink resolution refuses checks that escape the selected root into the other root.
        // 51: an in-round first freeze accepts an absent live contract and
        // binds publication to its recorded absence.
        // 52: a missing program at the probe site is an author finding and
        // host-unproven checks publish independently from proven siblings.
        // 53: #496 a CLI repair that publishes no checks exits incomplete.
        // CI re-pin only: refusal diagnostics render paths with forward slashes on every host; verdicts are unchanged.
        // 63: #526 a judge refutation counts only when its quoted rule is in the output or check text the judge saw; #528 the per-entry reuse key uses each entry's own named-file content.
        // 64: #533 a listing probe stopped before any output is a transient host fault, never an author finding; a silent scratch overrun says the host could not tell.
        // 65: #515 a re-author reply whose first entry is followed by a misplaced closing brace is refused, naming the continued field.
        // 66: #213 #466 #478 #498 #501 the acceptance site refuses only a closed list of operands that leave it (redirections, path-reading programs, cd/pushd targets, program-text file-open literals); a data heredoc for a text tool is not refused.
        // 67: #497 W3 a judge batch that stops with decided refutations returns them at once and leaves undecided checks uncached; transient host faults heal by retry instead of pausing.
        // #364 re-pin only: typed provider faults and outage backoff change how judge calls are retried, not what any verdict rule decides.
        // #497 W4 re-pin only: retention ordering and frozen routing do not change acceptance freeze verdict rules.
        // #541 re-pin only: the provider retry loop future is boxed to fit the debug stack budget; no verdict rule changed.
        // #497 W3 r9 + #541 merge re-pin only: invalid inputs map to SpecInvalid as on main and the provider retry future is boxed; no verdict rule changed, so versions are kept.
        // #543 re-pin only: scratch teardown releases the held site root before removing it (Windows sharing violation); no verdict rule changed.
        // #527 re-pin only: ARCHON_* names read through named constants instead of literals; no verdict rule changed.
        // integ-1029 re-pin only (#288 item 2 env closure, #534/#536, #527 item 5 constants, #494 author named-file prefetch): no verdict rule changed.
        // #538 provider diagnostics, #534 teardown fix and package B (#495 #492 #522 #532 #542 #544) integration re-pin: freeze-acceptance counts its verdict changes in version 68; for every other capability no verdict rule changed.
        // File-size split re-pin only: items moved unchanged into child modules; the closure follows the moved items; no verdict rule changed. #555 re-pin only: entry author prompts in the decomposition scripts carry only their own and phase-level findings, with bounded earlier sections, and the phase seed's criterion reader (workflow_decompose_seed_record.rs) ends at those sections; no verdict reads either. #559 re-pin only: an entry author prompt states the entry's current covers, a reply with no covers key keeps them, and the phase seed (workflow_decompose_seed_record.rs) takes them from that call's own recorded prompt line; no verdict reads either. #561 re-pin only: the decomposition script's acceptance progress window (workflow_decompose_v1_progress.js, new workflow_decompose_v1_failing_set.js) changes only when the loop pauses as no_progress; no verdict reads either. #559 guard re-pin only: the entry author reads the current entry with state.entries?.get (workflow_decompose_v1_acceptance.js), as the named-files context beside it does; no verdict reads it. #562 re-pin only: a gate finding carries an optional gate_stage and judged_item, written into the envelope only when set; the findings, their text and every pass or fail are unchanged, and only the decomposition script's progress window (workflow_decompose_v1_failing_set.js) reads the new fields; no verdict reads them. #537/#531 batch re-pin only: acceptance_scratch_control.rs gains a test-only clock hook (production now() is Instant::now()) and archon-core watchers are bounded; no verdict reads either. Issue 563 re-pin only: acceptance author and re-author prompts state the check site PATH (acceptance_scratch_policy.rs check_site_path, passed as a script argument kept out of launch identity); no verdict reads it. Issue 568 re-pin only: the skeleton author prompt names the previous candidate the author submitted to freeze-skeleton in a host-written context file (path and sha256); no verdict reads it. Issue 566/568/569: freeze-acceptance bumped to 76 for Issue 566; Issue 568 and Issue 569 change only skeleton prompts, the prdObligationIds script argument and skeleton finding text here. #355 re-pin only: host child launches gain a sandbox-exec spawn preflight (a missing target is a spawn error) and read-denied credential paths gain write and ancestor denials; the verdict launches here start a resolved shell, so no verdict changes. #355 forgery fix re-pin only: the host creates missing ancestors of protected paths and the grant root (0700) before it builds the profile, and every ancestor gets a full write denial; the verdict launches here start a resolved shell, so no verdict changes. Issue 355 merge with Issue 566/568/569 and 571/572/573 re-pin only: no verdict rule changed. #585 re-pin only: a mutation-marker local in workflow_acceptance_executability_mutation_markers.rs was renamed (nonce to saved); no verdict reads a local name.
        // #593 re-pin only: the decomposition script's acceptance author window (workflow_decompose_v1_acceptance.js) keeps starting entries after a shape refusal; no verdict reads it.
        // #599 re-pin: version 83 covers retained skeleton contract validation and shared contract refusals; closed-whitelist correction scans each byte once with an explicit delimiter stack (32 levels, 4096 bytes), checks nested operands and substitutions in that scan, and drops inert POSIX assignments, grouping, comments, redirections and here-doc bodies before judging all words; bare negation is never a template, refusal text describes unreplaced placeholders without claiming an exit status, and template rows live in the sibling test module and the inline tests. Same unreleased version; pins are NOT recomputed here; CI must recompute them.
        // integ-1108 (#572 S0-S3, #567, #497, #460): version 86, see the version line.
        // integ-1108 S2 re-pin only: link and ownership diagnostics at freeze-skeleton write .decompose.log lines only; A4 candidates, eligibility and exclusions are unchanged (observations moved to a child module); test-only harness files follow S1ab script functions; no verdict rule changed.
        sources_digest: "abca75910354422910609073c2a916931e6356e8a356f09c9504747b82d3a294",
        build_bound: false,
    },
    CapabilityLogic {
        id: "freeze-skeleton",
        // Integration 2026-11-07 re-pin only (#611 #613 #623-#625 #627 #630 #634): closure files changed for acceptance author scheduling, macOS process identity, skeleton landing/seed reply rules (read only by skeleton reply validation before the gate) and tests; no verdict rule here changed.
        // #618 re-pin only: shares acceptance judge sources; skeleton verdict rules are unchanged.
        // 3: #386 an output overrun no longer fails the command; the verdict uses the full output.
        // #425 re-pin only: the repository claim parser changed; skeleton freeze verdicts do not read claims, and requirements-trace's version bump in this table does not change them.
        // 4: #444 redaction now preserves ordinary prose such as "Bearer authentication" instead of inserting a marker; freeze marker validation can therefore give a different verdict.
        // 5: review of #491: an attempt with open findings stays open when its logic is stale, and a recorded skeleton without its lock stays acceptance-only. 7: Issue 569 the unknown obligation finding names a non-obligation id and the host-listed id sources; freeze reuses by version only, so a replayed outcome would keep the old text. 8: Issue 572 skeleton freeze accepts host-listed PRD constraint ids in implements and raises no unowned_obligation for them; Issue 573 tab-indented REQ- bullets become obligations a task must claim. 9: Issue 574/575 frozen skeleton tasks carry source_sections, skeleton freeze raises uncited_section/unknown_section, and id-keyed table rows under a header with no obligation word become host-listed PRD constraints.
        version: 14, // #572 S5: skeleton lock v2 carries base_snapshot; classes from the snapshot (was 13: #572 S4a: deliverable implements validated (deliverable_implements_unclaimed), requirement classification in task-skeleton.lock (was 12: integ-1108: #572 S3 freeze-skeleton may run checks (FreezeProvider), InheritedPredecessor blocks publication, authored implements/status cleared (was 11: #599: shared contract refusal and frozen-field repair ownership.)))
        sources: &[GATE, CONTRACT, FREEZE],
        // #386 re-pin only: a candidate past the inline stdin bound reaches the child whole through a run spill; a candidate that fits is read as before, and one past the bound never had a verdict.
        // #442 re-pin only: the base-tree suffix lookup moved into RepositoryTree::paths_ending_with; freeze verdicts do not read claims, and requirements-trace's version bump in this table does not change them.
        // #420 re-pin only: the skeleton freeze judges no check run, so the new no-verdict rule never applies.
        // #446 follow-up: archive collisions keep both prior byte sets.
        // #457 sibling-input classification changes can-fail proof outcomes only, not skeleton freeze verdicts.
        // #464 re-pin only: catalog version entries changed; freeze verdict logic is unchanged.
        // #466 re-pin only: mutation evidence is discarded unless every moved input is restored.
        // #472 re-pin only: reply unwrapping and mixed-form refusal text; verdict rules unchanged.
        // Re-pinned only (verdict rules unchanged): #434 #417 #439 #425 #438 #424 #369 #383 #366 #422 #386 #387 #442 #443 #441 #447 #440 #420 #446 #374 #340 #457 #448 #219 #464 #467 #466 #468 #469 #471 #472
        // #475 re-pin only: freeze source closure shares the moved strike helpers.
        // Re-pinned only (verdict rules unchanged): #476 #477 #479 #480
        // Re-pinned only (verdict rules unchanged): #483
        // Re-pinned only (verdict rules unchanged): regression-fix diagnostic routing and its fixture.
        // #487 re-pin only: the judge's partial-decision handling is not used by skeleton freeze.
        // #493 re-pin only: shared freeze sources changed; these acceptance
        // mutation and publication rules do not change skeleton verdicts.
        // Re-pin only: resume lineage repairs and missing cross-root refusal do not change the freeze verdict.
        // #496 re-pin only: partial CLI repair outcome reporting does not change skeleton freeze verdicts.
        // CI re-pin only: refusal diagnostics render paths with forward slashes on every host; verdicts are unchanged.
        // Batch 24 re-pin only: skeleton freeze verdicts do not read the acceptance judge, reply extraction or Scratch evidence.
        // Batch 25 re-pin only: skeleton verdicts do not read mutation proofs or re-author prompts.
        // Batch 26 re-pin only: skeleton freeze verdicts do not read acceptance judge identity, passability or path classes.
        // Batch 27 re-pin only (#525 read note, #527 ARCHON_* constants, #528 reuse key, #529/#530 author loop, passability file split, test-only named-file digests): no verdict rule changed.
        // #533 re-pin only: skeleton freeze verdicts do not run subcommand listing probes.
        // #514/#515/#527 re-pin only: skeleton freeze verdicts do not read the acceptance re-author reply parser.
        // #213/#466/#478/#498/#501 re-pin only: skeleton freeze verdicts do not run the acceptance site path policy; freeze-acceptance 66 counts the change.
        // #364 re-pin only: typed provider faults and outage backoff do not change skeleton freeze verdicts.
        // 6: #497 W4 the outgoing skeleton pin is retained under the publication lock before rebind; a stale lock or cache pair after interrupted retention is refused.
        // #497 W3 re-pin only: host-command fault healing (typed transient retry with observable attempts, committed publications never re-run) does not change this capability's verdict rules.
        // #541 re-pin only: the provider retry loop future is boxed to fit the debug stack budget; no verdict rule changed.
        // #497 W3 r9 + #541 merge re-pin only: invalid inputs map to SpecInvalid as on main and the provider retry future is boxed; no verdict rule changed, so versions are kept.
        // #543 re-pin only: scratch teardown releases the held site root before removing it (Windows sharing violation); no verdict rule changed.
        // #527 re-pin only: ARCHON_* names read through named constants instead of literals; no verdict rule changed.
        // integ-1029 re-pin only (#288 item 2 env closure, #534/#536, #527 item 5 constants, #494 author named-file prefetch): no verdict rule changed.
        // #538 provider diagnostics, #534 teardown fix and package B (#495 #492 #522 #532 #542 #544) integration re-pin: freeze-acceptance counts its verdict changes in version 68; for every other capability no verdict rule changed.
        // File-size split re-pin only: items moved unchanged into child modules; the closure follows the moved items; no verdict rule changed. #555 re-pin only: entry author prompts in the decomposition scripts carry only their own and phase-level findings, with bounded earlier sections, and the phase seed's criterion reader (workflow_decompose_seed_record.rs) ends at those sections; no verdict reads either. #557 re-pin only: freeze-skeleton runs no check, so the probe path's routing and text of a program neither the site nor the host has never reach it. #558 re-pin only: freeze-skeleton runs no check and judges no baseline failure, so the passability stage's routing never reaches it. #559 re-pin only: an entry author prompt states the entry's current covers, a reply with no covers key keeps them, and the phase seed (workflow_decompose_seed_record.rs) takes them from that call's own recorded prompt line; no verdict reads either. #561 re-pin only: the decomposition script's acceptance progress window (workflow_decompose_v1_progress.js, new workflow_decompose_v1_failing_set.js) changes only when the loop pauses as no_progress; no verdict reads either. #559 guard re-pin only: the entry author reads the current entry with state.entries?.get (workflow_decompose_v1_acceptance.js), as the named-files context beside it does; no verdict reads it. #562 re-pin only: a gate finding carries an optional gate_stage and judged_item, written into the envelope only when set; the findings, their text and every pass or fail are unchanged, and only the decomposition script's progress window (workflow_decompose_v1_failing_set.js) reads the new fields; no verdict reads them. #560 re-pin only: freeze-skeleton judges no acceptance check, so the named-file absence detail and the judge and author absent-path rules never reach it. #537/#531 batch re-pin only: acceptance_scratch_control.rs gains a test-only clock hook (production now() is Instant::now()) and archon-core watchers are bounded; no verdict reads either. Issue 563 re-pin only: acceptance author and re-author prompts state the check site PATH (acceptance_scratch_policy.rs check_site_path, passed as a script argument kept out of launch identity); no verdict reads it. Issue 568 re-pin only: the skeleton author prompt names the previous candidate the author submitted to freeze-skeleton in a host-written context file (path and sha256); no verdict reads it. Issue 566/568/569: freeze-skeleton bumped to 7 for Issue 569; Issue 566 changes acceptance probe inputs and the acceptance judge only. #355 re-pin only: host child launches gain a sandbox-exec spawn preflight (a missing target is a spawn error) and read-denied credential paths gain write and ancestor denials; the verdict launches here start a resolved shell, so no verdict changes. #355 forgery fix re-pin only: the host creates missing ancestors of protected paths and the grant root (0700) before it builds the profile, and every ancestor gets a full write denial; the verdict launches here start a resolved shell, so no verdict changes. Issue 355 merge with Issue 566/568/569 and 571/572/573 re-pin only: no verdict rule changed. Issue 579 re-pin only: build-flag variables and the Linux agent-launch refusal reach check commands, and freeze-skeleton runs no check, so no verdict changes. #585 re-pin only: a mutation-marker local in workflow_acceptance_executability_mutation_markers.rs was renamed (nonce to saved); no verdict reads a local name.
        // #593 re-pin only: the decomposition script's acceptance author window (workflow_decompose_v1_acceptance.js) keeps starting entries after a shape refusal; no verdict reads it.
        // #599 re-pin: version 11 covers shared contract refusals and repair ownership; closed-whitelist correction scans each byte once with an explicit delimiter stack (32 levels, 4096 bytes), checks nested operands and substitutions in that scan, and drops inert POSIX assignments, grouping, comments, redirections and here-doc bodies before judging all words; bare negation is never a template, refusal text describes unreplaced placeholders without claiming an exit status, and template rows live in the sibling test module and the inline tests. Same unreleased version; pins are NOT recomputed here; CI must recompute them.
        // integ-1108 (#572 S0-S3, #567, #497, #460): version 12, see the version line.
        // integ-1108 S2 re-pin only: link and ownership diagnostics at freeze-skeleton write .decompose.log lines only; A4 candidates, eligibility and exclusions are unchanged (observations moved to a child module); test-only harness files follow S1ab script functions; no verdict rule changed.
        sources_digest: "abca75910354422910609073c2a916931e6356e8a356f09c9504747b82d3a294",
        build_bound: false,
    },
    CapabilityLogic {
        id: "land-task-body",
        // Integration 2026-11-07 re-pin only (#611 #613 #623-#625 #627 #630 #634): closure files changed for acceptance author scheduling, macOS process identity, skeleton landing/seed reply rules (read only by skeleton reply validation before the gate) and tests; no verdict rule here changed.
        // 3: R8 body packaging strips chat around the one frozen subject's task
        // file and binds by its task_id; the fidelity critic re-asks on an
        // unparseable answer while it makes progress.
        // 4: #380's obligation model changes the frozen contract verdict.
        // 5: #386 an output overrun no longer fails the command; the verdict uses the full output.
        // 6: #425 repository claims recognize leading negation and explicit directory scopes.
        // 7: #428 the host stamps frozen skeleton frontmatter fields at landing.
        // 8: #434 a fidelity gap blocks only its weakest task's body; true verdicts quote a checked anchor and are reused per obligation.
        // 9: #425 dotted identifiers followed by a file citation are not repository-path claims; shared-scope negation and tree-aware dotted or single-word scope resolution now preserve claim polarity and identify the actual scoped path.
        // 10: #443 a path in the asserted clause of a test the body specifies ("assert: (a) `x` exists; (b) …")
        // is a runtime expectation, not a repository claim; a word before the path still claims.
        // 11: #443 review: the asserted clause ends at its first path's predicate; a marker continues only an
        // enumeration its lead opened; numbered items and indented openers carry only their own list; a
        // preposition before the path, repository wording ("(N lines)", "base commit") or a planned change claims.
        // 12: #443 re-review: a blank line before an assertion list's first item keeps the list; a path an output
        // noun locates ("the report at `x`") stays exempt; a prefixed lead ("re-verify that") is a lead.
        // 13: #443 re-review 3: a relative clause right after the path ("`x`, which already exists") claims.
        // No version bump: refused candidates now stay unpublished in observe mode; only those refused candidates are affected, recorded results remain valid, and the lineage guard makes a future bump safe.
        // #441 review, same unreleased version: PRD context, shared whole-set attribution and evidence-backed quotes change body findings.
        // 16: #447 accepted superseded rounds now receive the current
        // receipt-verified landing's verdict, rejudged without publication if
        // that landing's recorded logic is stale.
        // 18: malformed task-body parse failures are now classified as body findings, changing landing verdicts.
        // 19: landing repair uses the store's platform-aware directory flush, so recovered bodies can be judged on Windows.
        // 20: #444 redaction now preserves ordinary prose such as "Bearer authentication" instead of inserting a marker; task-body lint can therefore give a different verdict.
        // 21: reported claims are excluded and short negated path subjects are read as absence claims.
        // 22: declared write paths that match Files Forbidden to Change are rejected by the body scope gate using the shared matcher and refutation wording.
        // 23: #464 body and set gates normalize absolute declared paths against the task set's recorded repository root, changing collision findings when project and repository roots differ.
        // 24: #491 body lint treats an intact skeleton from an older acceptance as draft input.
        // 25: #497 stalled task-body authors persist findings per subject and resume only those subjects.
        // 26: exit-path audit: an unusable critic reply is typed authored_judge_reply, not gate_operational, which changes how body gate outcomes are routed.
        // 27: batch 24 (#497 F2 freeze findings, #515 reply extraction) shares sources
        // with body gating; bumped so no body verdict is reused across it.
        // 28: batch 25 (#494 read-guard ranges and prompt prefetch) shares sources with
        // body gating; bumped so no body verdict is reused across it.
        // 29: batch 26 (#490 landing reconcile, #521 per-agent cutoffs) shares sources with
        // body gating; bumped so no body verdict is reused across it. 30: Issue 572 body lint of a draft or pending skeleton accepts PRD constraint ids and raises no unowned_obligation for them; Issue 573 tab-indented REQ- bullets become obligations. 31: Issue 574/575 body lint of a draft or pending skeleton raises uncited_section/unknown_section, the frozen source_sections are stamped into bodies and compared, and id-keyed table rows become PRD constraints.
        version: 37, // Frozen command decisions can change body findings. Previously: #572 S9: body tool validation reads the project .mcp.json strictly (was 35: #572 S4a: deliverable_implements_unclaimed refused in bodies; stamped bodies carry deliverable implements (was 34: integ-1108: #572 S3 authored completion status is refused; frozen status stamped (was 33: #599: shared contract refusal and frozen-field repair ownership.)))
        // #601, same unreleased version: declared paths come from the ownership reader; missing observations are no finding; explicit contradictions and path-form errors stay findings.
        sources: &[GATE, CONTRACT, LINT],
        // #425 re-pin only: unscoped negated bare names, filler before a scoped predicate, base-name, slashed and multi-directory scopes (each directory one reading); same version bump as above.
        // #434d re-pin only: a fidelity reply is read verdict by verdict and each verdict kept passes every check a whole reply passes; the store keys on CRITIC_LOGIC_VERSION 1 and migrates schema-4/5 records, so no verdict changes meaning.
        // #443: the version bumps above; asserted runtime paths are no longer repository claims.
        // Re-pin only: observe-mode refused bodies now stay unpublished; refused candidates never publish, existing results remain valid, and lineage protects any future version bump.
        // #441: this gate asks fidelity with PRD context and the candidate fallback.
        // #441 review 2, same unreleased version: Acceptance policy headings reach the body critic.
        // #441 review 3, same unreleased version: the complete PRD reaches the body critic.
        // #434d, same unreleased version 14: task quote provenance accepts bounded normalization and ordered elisions.
        // #434d review re-pin: quote normaliser split into its own module; same unreleased versions
        // #441: task-shape checks now read focused test commands from the runtime task model.
        // 17: #447 receipt-mismatch self-healing settles the newest durable task body before judging it.
        // A12 re-pin only: A9 already bumped this version for malformed-body verdicts.
        // recovery; body-landing verdicts are unchanged.
        // #448 re-pin only: acceptance_scratch_hooks.rs and workflow_decompose.rs forward resolved cargo limits to observation commands; staged body lint verdict rules are unchanged.
        // #464 re-pin only: scope declarations share recorded repository-root resolution with the body gate; the version-23 verdict change is already counted above.
        // #469 re-pin only: acceptance reply marker handling changed the shared source closure; body landing version stays fixed.
        // #472 re-pin only: reply unwrapping and mixed-form refusal text; verdict rules unchanged.
        // Re-pinned only (verdict rules unchanged): #434 #386 #387 #425 #424 #369 #383 #438 #442 #441 #447 #446 #448 #464 #467 #466 #469 #471 #472
        // Re-pinned only (verdict rules unchanged): #476
        // #487 re-pin only: this acceptance judge path does not decide task body verdicts.
        // #497 re-pin only: body stall handling does not change task-set lint verdicts.
        // Batch 27 re-pin only (#525 read note, #527 ARCHON_* constants, #528 reuse key, #529/#530 author loop, passability file split, test-only named-file digests): no verdict rule changed.
        // #514/#515/#527 re-pin only: the acceptance re-author reply parser does not decide this capability's verdicts.
        // #213/#466/#478/#498/#501 re-pin only: the acceptance site path policy does not decide this capability's verdicts.
        // #497 W4 re-pin only: frozen-stage routing and ladder progress do not decide this capability's verdicts.
        // #497 W3 re-pin only: host-command fault healing (typed transient retry with observable attempts, committed publications never re-run) does not change this capability's verdict rules.
        // #543 re-pin only: scratch teardown releases the held site root before removing it (Windows sharing violation); no verdict rule changed.
        // #527 re-pin only: ARCHON_* names read through named constants instead of literals; no verdict rule changed.
        // integ-1029 re-pin only (#288 item 2 env closure, #534/#536, #527 item 5 constants, #494 author named-file prefetch): no verdict rule changed.
        // #538 provider diagnostics, #534 teardown fix and package B (#495 #492 #522 #532 #542 #544) integration re-pin: freeze-acceptance counts its verdict changes in version 68; for every other capability no verdict rule changed.
        // File-size split re-pin only: items moved unchanged into child modules; the closure follows the moved items; no verdict rule changed. #555 re-pin only: entry author prompts in the decomposition scripts carry only their own and phase-level findings, with bounded earlier sections, and the phase seed's criterion reader (workflow_decompose_seed_record.rs) ends at those sections; no verdict reads either. #559 re-pin only: an entry author prompt states the entry's current covers, a reply with no covers key keeps them, and the phase seed (workflow_decompose_seed_record.rs) takes them from that call's own recorded prompt line; no verdict reads either. #561 re-pin only: the decomposition script's acceptance progress window (workflow_decompose_v1_progress.js, new workflow_decompose_v1_failing_set.js) changes only when the loop pauses as no_progress; no verdict reads either. #559 guard re-pin only: the entry author reads the current entry with state.entries?.get (workflow_decompose_v1_acceptance.js), as the named-files context beside it does; no verdict reads it. #562 re-pin only: a gate finding carries an optional gate_stage and judged_item, written into the envelope only when set; the findings, their text and every pass or fail are unchanged, and only the decomposition script's progress window (workflow_decompose_v1_failing_set.js) reads the new fields; no verdict reads them. #560 re-pin only: the body critic reads no named-file view and no acceptance judge or author rule; only the shared judge rule text, named-file view and author rule text in this closure changed. Issue 563 re-pin only: acceptance author and re-author prompts state the check site PATH (acceptance_scratch_policy.rs check_site_path, passed as a script argument kept out of launch identity); no verdict reads it. Issue 568 re-pin only: the skeleton author prompt names the previous candidate the author submitted to freeze-skeleton in a host-written context file (path and sha256); no verdict reads it. Issue 566/568/569 re-pin only: decomposition script prompts, the acceptance judge instruction and the unknown obligation message text (wording only, as #472); no verdict rule changed. #355 re-pin only: host child launches gain a sandbox-exec spawn preflight (a missing target is a spawn error) and read-denied credential paths gain write and ancestor denials; the verdict launches here start a resolved shell, so no verdict changes. Issue 355 merge with Issue 566/568/569 and 571/572/573 re-pin only: no verdict rule changed.
        // #593 re-pin only: the decomposition script's acceptance author window (workflow_decompose_v1_acceptance.js) keeps starting entries after a shape refusal; no verdict reads it.
        // #599 re-pin: version 33 covers shared contract refusals and frozen-field repair ownership; closed-whitelist correction scans each byte once with an explicit delimiter stack (32 levels, 4096 bytes), checks nested operands and substitutions in that scan, and drops inert POSIX assignments, grouping, comments, redirections and here-doc bodies before judging all words; bare negation is never a template, refusal text describes unreplaced placeholders without claiming an exit status, and template rows live in the sibling test module and the inline tests. Same unreleased version; pins are NOT recomputed here; CI must recompute them.
        // integ-1108 (#572 S0-S3, #567, #497, #460): version 34, see the version line.
        // integ-1108 S2 re-pin only: link and ownership diagnostics at freeze-skeleton write .decompose.log lines only; A4 candidates, eligibility and exclusions are unchanged (observations moved to a child module); test-only harness files follow S1ab script functions; no verdict rule changed.
        sources_digest: "0c180d574f83f5762e39ff3ddb290ca814a401b837910a1aa821e41ad3110c5a",
        build_bound: false,
    },
    CapabilityLogic {
        id: "task-set-lint",
        // Integration 2026-11-07 re-pin only (#611 #613 #623-#625 #627 #630 #634): closure files changed for acceptance author scheduling, macOS process identity, skeleton landing/seed reply rules (read only by skeleton reply validation before the gate) and tests; no verdict rule here changed.
        // 3: the fidelity critic re-asks on an unparseable answer while it
        // makes progress (R8).
        // 4: #380's obligation model changes the frozen contract verdict.
        // 5: #386 an output overrun no longer fails the command; the verdict uses the full output.
        // 6: #425 repository claims recognize leading negation and explicit directory scopes.
        // 7: #434 true fidelity verdicts quote a checked anchor and are reused per obligation over byte-identical cluster texts.
        // 8: #425 dotted identifiers followed by a file citation are not repository-path claims; shared-scope negation and tree-aware dotted or single-word scope resolution now preserve claim polarity and identify the actual scoped path.
        // 9: #443 a path in the asserted clause of a test the body specifies is a runtime expectation, not a
        // repository claim; same rule as land-task-body 10.
        // 10: #443 review, same rule as land-task-body 11.
        // 11: #443 re-review, same rule as land-task-body 12.
        // 12: #443 re-review 3, same rule as land-task-body 13.
        // #441 review, same unreleased version: PRD context and shared whole-set attribution change set-gate findings.
        // #441: the set gate checks exactly the focused test commands the runtime records.
        // 15: malformed task-body parse failures are now body findings, changing task-set lint verdicts.
        // 16: #444 redaction now preserves ordinary prose such as "Bearer authentication" instead of inserting a marker; task-body lint can therefore give a different verdict.
        // 17: reported claims are excluded and short negated path subjects are read as absence claims.
        // 18: declared write paths that match Files Forbidden to Change are rejected by scope declarations using the shared matcher and refutation wording.
        // 19: #464 absolute declared paths use the task set's recorded repository root, matching the body gate when project and repository roots differ.
        // 20: #491 an intact skeleton from an older acceptance is draft input
        // until the skeleton phase freezes it; lints no longer refuse it as partial.
        // 21: exit-path audit: an unusable set-critic reply is typed authored_judge_reply and counts in the ordinary no-progress window.
        // 22: batch 24 (#497 F2 freeze findings, #515 reply extraction) shares sources
        // with set gating; bumped so no set verdict is reused across it.
        // 23: batch 25 (#494) shares sources with set gating; bumped likewise.
        // 24: batch 26 (#520 #521) shares sources with set gating; bumped likewise. 25: Issue 572 set coverage defines PRD constraint ids and raises one unowned_constraint finding per constraint no task names; Issue 573 tab-indented REQ- bullets become obligations. 26: Issue 574/575 set coverage raises one uncited_section per uncited PRD section and one unknown_section per bad citation, and id-keyed table rows become PRD constraints.
        version: 32, // Frozen command decisions can change set findings. Previously: #572 S9: PRD-declared MCP tools covered per server by task required_tools (was 30: #572 S4a: deliverable_implements_unclaimed finding (was 29: #599: shared contract refusal and frozen-field repair ownership.))
        // #601, same unreleased version: declared paths come from the ownership reader; missing observations are no finding; explicit contradictions and path-form errors stay findings.
        sources: &[GATE, CONTRACT, LINT],
        // #425 re-pin only: unscoped negated bare names, filler before a scoped predicate, base-name, slashed and multi-directory scopes (each directory one reading); same version bump as above.
        // #434d re-pin only: a fidelity reply is read verdict by verdict and each verdict kept passes every check a whole reply passes; the store keys on CRITIC_LOGIC_VERSION 1 and migrates schema-4/5 records, so no verdict changes meaning.
        // #443: the version bumps above; asserted runtime paths are no longer repository claims.
        // #441: this gate judges fidelity with PRD acceptance and gap context.
        // #441 review 2, same unreleased version: Acceptance policy headings reach the set critic.
        // #441 review 3, same unreleased version: the complete PRD reaches the set critic.
        // #434d, same unreleased version 13: the set gate shares the updated quote provenance check.
        // #434d review re-pin: quote normaliser split into its own module; same unreleased versions
        // #441: focused file ownership and declaration findings use the runtime command rule.
        // A12 re-pin only: A9 already bumped this version for malformed-body verdicts.
        // recovery; lint verdicts are unchanged.
        // #448 re-pin only: acceptance_scratch_hooks.rs and workflow_decompose.rs forward resolved cargo limits to observation commands; lint verdict rules are unchanged.
        // #464 re-pin only: scope declarations share recorded repository-root resolution with the body gate; the version-19 verdict change is already counted above.
        // #469 re-pin only: acceptance reply marker handling changed the shared source closure; lint version stays fixed.
        // #472 re-pin only: reply unwrapping and mixed-form refusal text; verdict rules unchanged.
        // Re-pinned only (verdict rules unchanged): #434 #386 #387 #425 #424 #369 #383 #438 #442 #441 #447 #446 #448 #464 #467 #466 #469 #471 #472
        // Re-pinned only (verdict rules unchanged): #476
        // #487 re-pin only: this acceptance judge path does not decide topology lint verdicts.
        // Batch 27 re-pin only (#525 read note, #527 ARCHON_* constants, #528 reuse key, #529/#530 author loop, passability file split, test-only named-file digests): no verdict rule changed.
        // #514/#515/#527 re-pin only: the acceptance re-author reply parser does not decide topology lint verdicts.
        // #213/#466/#478/#498/#501 re-pin only: the acceptance site path policy does not decide topology lint verdicts.
        // #497 W4 re-pin only: frozen-stage routing and ladder progress do not decide topology lint verdicts.
        // #497 W3 re-pin only: host-command fault healing (typed transient retry with observable attempts, committed publications never re-run) does not change this capability's verdict rules.
        // #543 re-pin only: scratch teardown releases the held site root before removing it (Windows sharing violation); no verdict rule changed.
        // #527 re-pin only: ARCHON_* names read through named constants instead of literals; no verdict rule changed.
        // integ-1029 re-pin only (#288 item 2 env closure, #534/#536, #527 item 5 constants, #494 author named-file prefetch): no verdict rule changed.
        // #538 provider diagnostics, #534 teardown fix and package B (#495 #492 #522 #532 #542 #544) integration re-pin: freeze-acceptance counts its verdict changes in version 68; for every other capability no verdict rule changed.
        // File-size split re-pin only: items moved unchanged into child modules; the closure follows the moved items; no verdict rule changed. #555 re-pin only: entry author prompts in the decomposition scripts carry only their own and phase-level findings, with bounded earlier sections, and the phase seed's criterion reader (workflow_decompose_seed_record.rs) ends at those sections; no verdict reads either. #559 re-pin only: an entry author prompt states the entry's current covers, a reply with no covers key keeps them, and the phase seed (workflow_decompose_seed_record.rs) takes them from that call's own recorded prompt line; no verdict reads either. #561 re-pin only: the decomposition script's acceptance progress window (workflow_decompose_v1_progress.js, new workflow_decompose_v1_failing_set.js) changes only when the loop pauses as no_progress; no verdict reads either. #559 guard re-pin only: the entry author reads the current entry with state.entries?.get (workflow_decompose_v1_acceptance.js), as the named-files context beside it does; no verdict reads it. #562 re-pin only: a gate finding carries an optional gate_stage and judged_item, written into the envelope only when set; the findings, their text and every pass or fail are unchanged, and only the decomposition script's progress window (workflow_decompose_v1_failing_set.js) reads the new fields; no verdict reads them. #560 re-pin only: the set critic reads no named-file view and no acceptance judge or author rule; only the shared judge rule text, named-file view and author rule text in this closure changed. Issue 563 re-pin only: acceptance author and re-author prompts state the check site PATH (acceptance_scratch_policy.rs check_site_path, passed as a script argument kept out of launch identity); no verdict reads it. Issue 568 re-pin only: the skeleton author prompt names the previous candidate the author submitted to freeze-skeleton in a host-written context file (path and sha256); no verdict reads it. Issue 566/568/569 re-pin only: decomposition script prompts, the acceptance judge instruction and the unknown obligation message text (wording only, as #472); no verdict rule changed. #355 re-pin only: host child launches gain a sandbox-exec spawn preflight (a missing target is a spawn error) and read-denied credential paths gain write and ancestor denials; the verdict launches here start a resolved shell, so no verdict changes. Issue 355 merge with Issue 566/568/569 and 571/572/573 re-pin only: no verdict rule changed.
        // #593 re-pin only: the decomposition script's acceptance author window (workflow_decompose_v1_acceptance.js) keeps starting entries after a shape refusal; no verdict reads it.
        // #599 re-pin: version 29 covers shared contract refusals and frozen-field repair ownership; closed-whitelist correction scans each byte once with an explicit delimiter stack (32 levels, 4096 bytes), checks nested operands and substitutions in that scan, and drops inert POSIX assignments, grouping, comments, redirections and here-doc bodies before judging all words; bare negation is never a template, refusal text describes unreplaced placeholders without claiming an exit status, and template rows live in the sibling test module and the inline tests. Same unreleased version; pins are NOT recomputed here; CI must recompute them.
        // integ-1108 re-pin only (#572 S3): a body deliverable `implements` is now parsed; no host-landed body can carry it because land-task-body stamps the frozen contracts; set-lint still refuses it as frozen_field_changed; #567/#497/#460/#572 S1 prompt and script changes are read by no verdict here.
        // integ-1108 S2 re-pin only: link and ownership diagnostics at freeze-skeleton write .decompose.log lines only; A4 candidates, eligibility and exclusions are unchanged (observations moved to a child module); test-only harness files follow S1ab script functions; no verdict rule changed.
        sources_digest: "0c180d574f83f5762e39ff3ddb290ca814a401b837910a1aa821e41ad3110c5a",
        build_bound: false,
    },
    CapabilityLogic {
        id: "requirements-trace",
        // Integration 2026-11-07 re-pin only (#611 #613 #623-#625 #627 #630 #634): closure files changed for acceptance author scheduling, macOS process identity, skeleton landing/seed reply rules (read only by skeleton reply validation before the gate) and tests; no verdict rule here changed.
        // 3: #386 an output overrun no longer fails the command; the verdict uses the full output.
        // 4: #442 a `path::symbol` or `path:line` span is a path plus an anchor judged in that file; a
        // package-relative path is read against every base path it is a suffix of (the claiming task's
        // own packages first) and refuted only when every reading refutes; a placeholder cargo target
        // is no observable; a bare name that is a Cargo target exists (or is derived when declared).
        // 5: typed malformed-body errors change task-universe parsing in requirements trace.
        // 6: #464 declared files forbidden by a whole-file bullet now refute the claim and name the conflicting declaration and bullet.
        sources: &[GATE, CONTRACT, TRACE],
        // #464 re-pin only: review 4's verdict change is already counted in version 6.
        // 7: exit-path audit: shared judge and critic sources changed with the typed reply kinds; bumped so no record judged by the previous logic is replayed. 8: Issue 572 constraint ids are defined (no phantom citation) and not reported unclaimed; Issue 573 tab-indented REQ- bullets become obligations. 9: #355 the falsify verifier's launch passes the sandbox-exec spawn preflight, so a claimed program that is missing or not executable is a spawn error, not a sandbox-exec exit code; a verdict can change. 10: Issue 574/575 requirements trace raises uncited_section/unknown_section from the task files' source_sections, and id-keyed table rows become PRD constraints. 11: Issue 577 the same environment and REFER change reaches the falsify verifier's commands. 12: Issue 579 the same build-flag allowlist and Linux launch refusal reach the falsify verifier's commands.
        version: 16, // Generic verifier policy and frozen command decisions can change trace findings. Previously: #599: shared deliverable-contract validation changes trace input verdicts.
        // Re-pinned only (verdict rules unchanged): #434 #417 #438 #441 #424 #369 #383 #440 #420 #447 #446 #464 #466 #469
        // Batch 25 re-pin only: the shared Scratch path helper moved; trace verdicts do not read mutation proofs.
        // Batch 27 re-pin only (#525 read note, #527 ARCHON_* constants, #528 reuse key, #529/#530 author loop, passability file split, test-only named-file digests): no verdict rule changed.
        // #213/#466/#478/#498/#501 re-pin only: the acceptance site path policy does not decide requirement trace verdicts.
        // #497 W4 re-pin only: frozen-stage routing and ladder progress do not decide requirement trace verdicts.
        // #497 W3 re-pin only: host-command fault healing (typed transient retry with observable attempts, committed publications never re-run) does not change this capability's verdict rules.
        // #543 re-pin only: scratch teardown releases the held site root before removing it (Windows sharing violation); no verdict rule changed.
        // integ-1029 re-pin only (#288 item 2 env closure, #534/#536, #527 item 5 constants, #494 author named-file prefetch): no verdict rule changed.
        // #538 provider diagnostics, #534 teardown fix and package B (#495 #492 #522 #532 #542 #544) integration re-pin: freeze-acceptance counts its verdict changes in version 68; for every other capability no verdict rule changed.
        // File-size split re-pin only: items moved unchanged into child modules; the closure follows the moved items; no verdict rule changed. #562 re-pin only: a gate finding carries an optional gate_stage and judged_item, written into the envelope only when set; the findings, their text and every pass or fail are unchanged, and only the decomposition script's progress window (workflow_decompose_v1_failing_set.js) reads the new fields; no verdict reads them. Issue 569 re-pin only: the unknown obligation message text in task_skeleton.rs; requirements-trace does not produce it. #355 bump to 9 (see the version note): the falsify verifier launch passes the sandbox-exec spawn preflight. #355 forgery fix, same unreleased version 9: the host creates missing ancestors and every ancestor gets a full write denial; no verdict changes beyond the version 9 note. Issue 355 merge with Issue 566/568/569 and 571/572/573 re-pin only, same version 9.
        // #599 re-pin: version 15 covers shared deliverable-contract validation of trace inputs; closed-whitelist correction scans each byte once with an explicit delimiter stack (32 levels, 4096 bytes), checks nested operands and substitutions in that scan, and drops inert POSIX assignments, grouping, comments, redirections and here-doc bodies before judging all words; bare negation is never a template, refusal text describes unreplaced placeholders without claiming an exit status, and template rows live in the sibling test module and the inline tests. Same unreleased version; pins are NOT recomputed here; CI must recompute them.
        // integ-1108 re-pin only (#572 S3): a body deliverable `implements` is now parsed; trace ignores it; #572 S1 coverable ids reach no trace verdict.
        // #572 S4a re-pin only: shared lock and deliverable types gain optional fields (byte-identical when empty); a verdict-invariance test proves no verification or trace verdict changes.
        sources_digest: "d86d3563ec81a97349897c615c2c1931645897e02f27db59b125c4f273cd856c",
        build_bound: true,
    },
    CapabilityLogic {
        id: "verify-frozen-acceptance",
        // Integration 2026-11-07 re-pin only (#611 #613 #623-#625 #627 #630 #634): closure files changed for acceptance author scheduling, macOS process identity, skeleton landing/seed reply rules (read only by skeleton reply validation before the gate) and tests; no verdict rule here changed.
        // 3: #386 an output overrun no longer fails the command; the verdict uses the full output.
        // 4: #417 copied check inputs omit freeze-owned files, which can change a check verdict.
        // 5: #417 harness-written project run state leaves hermetic check inputs, which can change a check verdict.
        // 6: #417 copied task-root inputs now match the digest exclusions, which can change a check verdict.
        // 8: #366 rechecks frozen acceptance paths using shell assignment scope when locating live roots (POSIX spelling case-sensitive on Windows too).
        // 9: every spawned check receives resolved resource limits.
        // 10: a legacy guardian request without resource_limits is rejected,
        // instead of silently running with default limits.
        // 11: #444 redaction now preserves ordinary prose such as "Bearer authentication" instead of inserting a marker; freeze marker validation can therefore give a different verdict.
        // 12: #486 scratch sibling aliases no longer manufacture a conflicting
        // build target during frozen acceptance reverify.
        // 13: #489 scanned sibling paths drop argument/list separators, so an
        // existing sibling is no longer reported missing; this can change verdicts.
        // 15: review of #489/#495/#491: non-ASCII path ends are kept, sibling
        // discovery sees every lexical path position, and open findings keep a
        // stale-logic stage open.
        // 16: #477 Scratch evidence schema 5 and repository-sibling proofs can change
        // frozen acceptance re-verification verdicts.
        // 17: #518 and #519 change which inputs a frozen-check mutation proof moves
        // and how it restores them.
        // 18: batch 26 #523 host refutation and #520 judge identity can change reverify verdicts.
        version: 24, // #572 S7: floor predicates strict native (D-S7-10) (was 23: integ-1108: #572 S1 coverable id set widened, so frozen covers of new id classes verify (was 22: #599: acceptance bundle validation reports contract field defects (21: #590 re-pin only, frozen verification runs no probe).))
        sources: &[GATE, CONTRACT, VERIFY],
        // #442 re-pin only: requirements-trace's version bump in this table; verification verdicts are unchanged.
        // #420 re-pin only: frozen-chain verification judges no check run, so the new no-verdict rule never applies.
        // #457 reverify report evidence and can-fail proof logic do not affect frozen-chain verification.
        // #448 re-pin only: acceptance_scratch_observe.rs and workflow_decompose.rs pass resolved cargo limits into frozen check execution; limits change execution environment and parallelism, not verification verdict rules.
        // #457 re-pin only: freeze-acceptance logic version 32 changes the can-fail proof, not frozen-chain verification.
        // #464 re-pin only: catalog version entries changed; verification verdict logic is unchanged.
        // #468 re-pin only: an unrestored mutation result cannot be reused; frozen-chain verification runs no mutation probe.
        // #466 re-pin only: mutation evidence is discarded unless every moved input is restored.
        // #472 re-pin only: reply unwrapping and mixed-form refusal text; verdict rules unchanged.
        // Re-pinned only (verdict rules unchanged): #417 #434 #439 #425 #438 #386 #387 #424 #369 #383 #442 #443 #441 #447 #440 #420 #446 #374 #340 #457 #448 #219 #464 #467 #466 #468 #469 #471 #472
        // #475 re-pin only: freeze source closure shares the moved strike helpers.
        // Re-pinned only (verdict rules unchanged): #476 #477 #480
        // Re-pinned only (verdict rules unchanged): #483
        // #487 re-pin only: frozen-chain verification does not call the acceptance judge.
        // #490 re-pin only: resume snapshot recovery does not change frozen verification verdicts.
        // 14: #491 an intact skeleton from an older acceptance no longer makes acceptance verification fail.
        // Re-pin only: proof diagnostics and resume repair selection do not change frozen-chain verification.
        // Re-pin only: bounded mutation output evidence and resume routing do not change frozen verification verdicts.
        // #496 re-pin only: in-round host-unproven state remains outside frozen verification.
        // #497 re-pin only: freeze repair findings do not change frozen-chain verification.
        // CI re-pin only: refusal diagnostics render paths with forward slashes on every host; verdicts are unchanged.
        // Batch 27 re-pin only (#525 read note, #527 ARCHON_* constants, #528 reuse key, #529/#530 author loop, passability file split, test-only named-file digests): no verdict rule changed.
        // 19: #533 a listing probe stopped before any output is a transient host fault, never an author finding.
        // #213/#466/#478/#498/#501 re-pin only: frozen acceptance verification reads stored policy findings; it does not run the acceptance site path policy.
        // 20: #497 W4 an authored finding in frozen acceptance verification reopens its owning author; transient inability retries in the progress window; an unsettled publish still pauses the run (Issue 338).
        // #497 W3 re-pin only: host-command fault healing (typed transient retry with observable attempts, committed publications never re-run) does not change this capability's verdict rules.
        // #497 W3 r9 + #541 merge re-pin only: invalid inputs map to SpecInvalid as on main and the provider retry future is boxed; no verdict rule changed, so versions are kept.
        // #543 re-pin only: scratch teardown releases the held site root before removing it (Windows sharing violation); no verdict rule changed.
        // #527 re-pin only: ARCHON_* names read through named constants instead of literals; no verdict rule changed.
        // integ-1029 re-pin only (#288 item 2 env closure, #534/#536, #527 item 5 constants, #494 author named-file prefetch): no verdict rule changed.
        // #538 provider diagnostics, #534 teardown fix and package B (#495 #492 #522 #532 #542 #544) integration re-pin: freeze-acceptance counts its verdict changes in version 68; for every other capability no verdict rule changed.
        // File-size split re-pin only: items moved unchanged into child modules; the closure follows the moved items; no verdict rule changed. #555 re-pin only: entry author prompts in the decomposition scripts carry only their own and phase-level findings, with bounded earlier sections, and the phase seed's criterion reader (workflow_decompose_seed_record.rs) ends at those sections; no verdict reads either. #557 re-pin only: frozen verification runs no probe; only the probe path's routing and text of a program neither the site nor the host has changed. #558 re-pin only: only this catalog's freeze-acceptance version and notes changed in this closure; frozen verification judges no baseline failure. #559 re-pin only: an entry author prompt states the entry's current covers, a reply with no covers key keeps them, and the phase seed (workflow_decompose_seed_record.rs) takes them from that call's own recorded prompt line; no verdict reads either. #561 re-pin only: the decomposition script's acceptance progress window (workflow_decompose_v1_progress.js, new workflow_decompose_v1_failing_set.js) changes only when the loop pauses as no_progress; no verdict reads either. #559 guard re-pin only: the entry author reads the current entry with state.entries?.get (workflow_decompose_v1_acceptance.js), as the named-files context beside it does; no verdict reads it. #562 re-pin only: a gate finding carries an optional gate_stage and judged_item, written into the envelope only when set; the findings, their text and every pass or fail are unchanged, and only the decomposition script's progress window (workflow_decompose_v1_failing_set.js) reads the new fields; no verdict reads them. #560 re-pin only: frozen-chain verification does not call the acceptance judge or an author; only the named-file view text, the rule texts and this catalog's freeze-acceptance version changed in this closure. #537/#531 batch re-pin only: acceptance_scratch_control.rs gains a test-only clock hook (production now() is Instant::now()) and archon-core watchers are bounded; no verdict reads either. Issue 563 re-pin only: acceptance author and re-author prompts state the check site PATH (acceptance_scratch_policy.rs check_site_path, passed as a script argument kept out of launch identity); no verdict reads it. Issue 568 re-pin only: the skeleton author prompt names the previous candidate the author submitted to freeze-skeleton in a host-written context file (path and sha256); no verdict reads it. Issue 566/568/569 re-pin only: frozen verification reads stored judgments and runs no probe, judge or skeleton set validation. Issue 571/572/573 re-pin only: frozen verification validates acceptance ids and stored judgments and runs no requirement, constraint, covers or skeleton set validation; only catalog versions, id extraction and finding text changed in this closure. #355 re-pin only: host child launches gain a sandbox-exec spawn preflight (a missing target is a spawn error) and read-denied credential paths gain write and ancestor denials; the verdict launches here start a resolved shell, so no verdict changes. Issue 355 merge with Issue 566/568/569 and 571/572/573 re-pin only: no verdict rule changed. Issue 574/575 re-pin only: frozen acceptance verification reads no source_sections, PRD section or constraint, so no verdict changes. #585 re-pin only: a mutation-marker local in workflow_acceptance_executability_mutation_markers.rs was renamed (nonce to saved); no verdict reads a local name.
        // #593 re-pin only: the decomposition script's acceptance author window (workflow_decompose_v1_acceptance.js) keeps starting entries after a shape refusal; no verdict reads it.
        // #599 re-pin: version 22 covers shared deliverable-contract validation of frozen inputs; closed-whitelist correction scans each byte once with an explicit delimiter stack (32 levels, 4096 bytes), checks nested operands and substitutions in that scan, and drops inert POSIX assignments, grouping, comments, redirections and here-doc bodies before judging all words; bare negation is never a template, refusal text describes unreplaced placeholders without claiming an exit status, and template rows live in the sibling test module and the inline tests. Same unreleased version; pins are NOT recomputed here; CI must recompute them.
        // integ-1108: version 23, see the version line.
        // integ-1108 S2 re-pin only: link and ownership diagnostics at freeze-skeleton write .decompose.log lines only; A4 candidates, eligibility and exclusions are unchanged (observations moved to a child module); test-only harness files follow S1ab script functions; no verdict rule changed.
        // #572 S4a re-pin only: shared lock and deliverable types gain optional fields (byte-identical when empty); a verdict-invariance test proves no verification or trace verdict changes.
        sources_digest: "b972d21d5ea583fa41621a86df68e3c4603de60454397555af7b59c3e4f17785",
        build_bound: true,
    },
    CapabilityLogic {
        id: "verify-frozen-skeleton",
        // Integration 2026-11-07 re-pin only (#611 #613 #623-#625 #627 #630 #634): closure files changed for acceptance author scheduling, macOS process identity, skeleton landing/seed reply rules (read only by skeleton reply validation before the gate) and tests; no verdict rule here changed.
        // 3: #386 an output overrun no longer fails the command; the verdict uses the full output.
        // 4: #444 redaction now preserves ordinary prose such as "Bearer authentication" instead of inserting a marker; freeze marker validation can therefore give a different verdict.
        // 5: #491 a skeleton bound to the prior acceptance is pending and skipped by the frozen snapshot.
        // 6: review of #491: a recorded skeleton without its lock stays acceptance-only, and open findings keep a stale-logic stage open. 8: Issue 574/575 the frozen skeleton task shape (workflow_freeze_schema.rs, FrozenTask) gains source_sections.
        version: 10, // #599: shared contract refusal and frozen-field repair ownership.
        sources: &[GATE, CONTRACT, VERIFY],
        // #442 re-pin only: requirements-trace's version bump in this table; verification verdicts are unchanged.
        // #420 re-pin only: frozen-chain verification judges no check run, so the new no-verdict rule never applies.
        // #457 reverify report evidence and can-fail proof logic do not affect frozen-chain verification.
        // #457 re-pin only: freeze-acceptance logic version 32 does not affect frozen skeleton verification.
        // #464 re-pin only: catalog version entries changed; verification verdict logic is unchanged.
        // #466 re-pin only: mutation evidence is discarded unless every moved input is restored.
        // #472 re-pin only: reply unwrapping and mixed-form refusal text; verdict rules unchanged.
        // Re-pinned only (verdict rules unchanged): #366 #434 #417 #439 #425 #438 #386 #387 #424 #369 #383 #442 #443 #441 #447 #440 #420 #446 #374 #340 #457 #448 #219 #464 #467 #466 #468 #469 #471 #472
        // #475 re-pin only: freeze source closure shares the moved strike helpers.
        // Re-pinned only (verdict rules unchanged): #476 #477 #480
        // Re-pinned only (verdict rules unchanged): #483
        // #487 re-pin only: frozen-chain verification does not call the acceptance judge.
        // #490 re-pin only: resume snapshot recovery does not change frozen verification verdicts.
        // Re-pin only: proof diagnostics and resume repair selection do not change frozen-chain verification.
        // Re-pin only: bounded mutation output evidence and resume routing do not change frozen verification verdicts.
        // #496 re-pin only: in-round host-unproven state remains outside frozen verification.
        // #497 re-pin only: freeze repair findings do not change frozen skeleton verification.
        // CI re-pin only: refusal diagnostics render paths with forward slashes on every host; verdicts are unchanged.
        // Batch 24 re-pin only: frozen skeleton verification does not read Scratch evidence or the acceptance judge.
        // Batch 25 re-pin only: skeleton verdicts do not read mutation proofs or re-author prompts.
        // Batch 26 re-pin only: skeleton reverify verdicts do not read passability or acceptance judge identity.
        // Batch 27 re-pin only (#525 read note, #527 ARCHON_* constants, #528 reuse key, #529/#530 author loop, passability file split, test-only named-file digests): no verdict rule changed.
        // #533 re-pin only: skeleton reverify verdicts do not run subcommand listing probes.
        // #213/#466/#478/#498/#501 re-pin only: frozen skeleton verification reads stored policy findings; it does not run the acceptance site path policy.
        // 7: #497 W4 an authored finding in frozen skeleton verification reopens its owning author instead of stopping the run.
        // #497 W3 re-pin only: host-command fault healing (typed transient retry with observable attempts, committed publications never re-run) does not change this capability's verdict rules.
        // #497 W3 r9 + #541 merge re-pin only: invalid inputs map to SpecInvalid as on main and the provider retry future is boxed; no verdict rule changed, so versions are kept.
        // #543 re-pin only: scratch teardown releases the held site root before removing it (Windows sharing violation); no verdict rule changed.
        // #527 re-pin only: ARCHON_* names read through named constants instead of literals; no verdict rule changed.
        // integ-1029 re-pin only (#288 item 2 env closure, #534/#536, #527 item 5 constants, #494 author named-file prefetch): no verdict rule changed.
        // #538 provider diagnostics, #534 teardown fix and package B (#495 #492 #522 #532 #542 #544) integration re-pin: freeze-acceptance counts its verdict changes in version 68; for every other capability no verdict rule changed.
        // File-size split re-pin only: items moved unchanged into child modules; the closure follows the moved items; no verdict rule changed. #555 re-pin only: entry author prompts in the decomposition scripts carry only their own and phase-level findings, with bounded earlier sections, and the phase seed's criterion reader (workflow_decompose_seed_record.rs) ends at those sections; no verdict reads either. #557 re-pin only: frozen verification runs no probe; only the probe path's routing and text of a program neither the site nor the host has changed. #558 re-pin only: only this catalog's freeze-acceptance version and notes changed in this closure; frozen verification judges no baseline failure. #559 re-pin only: an entry author prompt states the entry's current covers, a reply with no covers key keeps them, and the phase seed (workflow_decompose_seed_record.rs) takes them from that call's own recorded prompt line; no verdict reads either. #561 re-pin only: the decomposition script's acceptance progress window (workflow_decompose_v1_progress.js, new workflow_decompose_v1_failing_set.js) changes only when the loop pauses as no_progress; no verdict reads either. #559 guard re-pin only: the entry author reads the current entry with state.entries?.get (workflow_decompose_v1_acceptance.js), as the named-files context beside it does; no verdict reads it. #562 re-pin only: a gate finding carries an optional gate_stage and judged_item, written into the envelope only when set; the findings, their text and every pass or fail are unchanged, and only the decomposition script's progress window (workflow_decompose_v1_failing_set.js) reads the new fields; no verdict reads them. #560 re-pin only: frozen-chain verification does not call the acceptance judge or an author; only the named-file view text, the rule texts and this catalog's freeze-acceptance version changed in this closure. #537/#531 batch re-pin only: acceptance_scratch_control.rs gains a test-only clock hook (production now() is Instant::now()) and archon-core watchers are bounded; no verdict reads either. Issue 563 re-pin only: acceptance author and re-author prompts state the check site PATH (acceptance_scratch_policy.rs check_site_path, passed as a script argument kept out of launch identity); no verdict reads it. Issue 568 re-pin only: the skeleton author prompt names the previous candidate the author submitted to freeze-skeleton in a host-written context file (path and sha256); no verdict reads it. Issue 566/568/569 re-pin only: frozen verification reads stored judgments and runs no probe, judge or skeleton set validation. Issue 571/572/573 re-pin only: frozen verification validates acceptance ids and stored judgments and runs no requirement, constraint, covers or skeleton set validation; only catalog versions, id extraction and finding text changed in this closure. #355 re-pin only: host child launches gain a sandbox-exec spawn preflight (a missing target is a spawn error) and read-denied credential paths gain write and ancestor denials; the verdict launches here start a resolved shell, so no verdict changes. Issue 355 merge with Issue 566/568/569 and 571/572/573 re-pin only: no verdict rule changed. #585 re-pin only: a mutation-marker local in workflow_acceptance_executability_mutation_markers.rs was renamed (nonce to saved); no verdict reads a local name.
        // #593 re-pin only: the decomposition script's acceptance author window (workflow_decompose_v1_acceptance.js) keeps starting entries after a shape refusal; no verdict reads it.
        // #599 re-pin: version 10 covers shared contract refusals and frozen-field repair ownership; closed-whitelist correction scans each byte once with an explicit delimiter stack (32 levels, 4096 bytes), checks nested operands and substitutions in that scan, and drops inert POSIX assignments, grouping, comments, redirections and here-doc bodies before judging all words; bare negation is never a template, refusal text describes unreplaced placeholders without claiming an exit status, and template rows live in the sibling test module and the inline tests. Same unreleased version; pins are NOT recomputed here; CI must recompute them.
        // integ-1108 re-pin only (#572 S3): shared schema closure (FrozenTask.status, deliverable implements) round-trips byte-identical; no verdict rule changed.
        // integ-1108 S2 re-pin only: link and ownership diagnostics at freeze-skeleton write .decompose.log lines only; A4 candidates, eligibility and exclusions are unchanged (observations moved to a child module); test-only harness files follow S1ab script functions; no verdict rule changed.
        // #572 S4a re-pin only: shared lock and deliverable types gain optional fields (byte-identical when empty); a verdict-invariance test proves no verification or trace verdict changes.
        sources_digest: "b972d21d5ea583fa41621a86df68e3c4603de60454397555af7b59c3e4f17785",
        build_bound: true,
    },
];
