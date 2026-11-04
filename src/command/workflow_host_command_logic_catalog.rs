use super::{CONTRACT, CapabilityLogic, FREEZE, GATE, LINT, TRACE, VERIFY};
// Windows CPU fix re-pin only (freeze, land/lint and verify digests): utf8Bytes counts bytes with native string calls and namedFilesContext sizes the file list incrementally; same bytes, no verdict rule changed.
pub(crate) const CAPABILITY_LOGIC: &[CapabilityLogic] = &[
    CapabilityLogic {
        id: "freeze-acceptance",
        // 2: #366 live-root rule (sibling paths pass; a bad repository lock
        // is an error) changes acceptance freeze verdicts.
        // 3: #288 acceptance checks rerun only when their input closure
        // changed; a reused check verdict binds normalized literal paths,
        // their filesystem state, the shell, environment and this logic.
        // 4: #380 complete wrapped obligation text changes freeze verdicts.
        // #385's re-ask improves recovery from an unusable provider reply;
        // it does not change the verdict for a usable reply. #392 changes
        // which authored reply shapes can supply an acceptance entry.
        // #392 accepts a sole unnamed check block when command_block is omitted.
        // #399 fixes resume seed reconstruction; identical gate input keeps its verdict.
        // 9: #400 validates incremental subsets independently, allowing supplementary-only updates to pass.
        // 7: #386 an output overrun no longer fails the command; the verdict uses the full output.
        // 10: #374 resolves existing aliases by identity; unresolved paths on the live volume fail closed.
        // 11: #417 #418 #419 freeze-owned outputs leave check inputs; passability and judge verdicts reuse by scoped input keys.
        // #421 changes when a recorded freeze is reused (a later publication extends its pin), not how it is judged.
        // 12: #423 check blocks normalize CR/CRLF to LF; a shell setup error is refused as a pre-implementation proof.
        // 14: #417 task-root exclusions and copies use the same project-relative input set as its digest.
        // 15: #417 Windows task-root paths normalize before containment checks, so probes can run and produce a different verdict.
        // #425 re-pin only: the repository claim parser changed; freeze verdicts do not read claims, and requirements-trace's version bump in this table does not change them.
        // 16: #440 the scratch copy skips a repository nested inside a project input, as the probe digest does, so a check can no longer read that live tree and its verdict can change.
        // 17: #420 a pre-implementation failure at a setup path outside the probe's tree that the output reports absent (`cd X`, `--manifest-path X`) gives no verdict: unproven, then its author's, never a passability verdict.
        // #420 review, same unreleased version 17: the rule applies only in a host-made copy (scratch, hermetic), never on the live tree, and a top-level name the copy holds (`../repo`) stays inside it.
        // 18: #446 direct-site baseline runs use their hermetic copy's layout when applying that rule.
        // 19: #402 missing provider stop reasons cannot complete acceptance authoring; #419 pinned restoration requires identical check and covers fields after a fresh refusal.
        // #446 review 2, same unreleased version 18
        // 20: #374 reconstructs unresolved paths from the longest existing
        // ancestor before comparing with the live root; only an unresolvable
        // same-volume path fails closed. Sibling paths can now pass.
        // 21: #420 copies named sibling inputs into each temporary check site;
        // absent named siblings are refused at author time and copied paths
        // are inside the setup-path tree, changing passability verdicts.
        // 21, reverify acceptance: finished runs execute this frozen check set
        // in the same copy and judge setup failures with freeze's passability rule.
        // 22: reverify follows freeze's baseline classification, accepting a
        // missing-implementation failure while preserving setup findings.
        // 21: #366 joins shell-word paths and carries shell assignments into
        // later commands when detecting live-root acceptance checks.
        // #366 Windows fix: POSIX path comparison stays case-sensitive on Windows.
        // 24: sibling discovery and copies use plain canonical paths, so
        // verbatim Windows paths do not break containment, git, or root names.
        // 25: every native check applies its resolved cargo limits, including
        // observations without a configured build cache.
        // 26: only meaning-preserving Windows verbatim paths reach Git;
        // unsafe plain spellings are refused, changing acceptance findings.
        // 27: freeze persists reusable probe observations and resumes their
        // judge verdicts across passability failures; reuse identity and
        // which probe/judge results complete authoring can differ.
        // 28: baseline-passing checks use a stored semantic judgment only
        // when its frozen contract and exact command digests still match;
        // otherwise the input-mutation proof and findings still apply.
        // 29: #444 redaction now preserves ordinary prose such as "Bearer authentication" instead of inserting a marker; freeze marker validation can therefore give a different verdict.
        // 30: sibling mutation inputs must be named in the check's actual
        // relative context, changing can-fail proof verdicts.
        // 31: literal file reads in recognized inline interpreter programs
        // are included in mutation inputs, changing can-fail proof verdicts.
        // 32: every program following a recognized inline-program flag is
        // scanned, changing can-fail proof verdicts for repeated flags.
        // 33: mutation wrappers preserve named sibling setup paths, so their
        // copy layout matches baseline and absent-sibling failures give no verdict.
        // 37: #468 mutation verdicts route every final or transiently recreated input to the author, and are saved or memoized only when that run's markers show every moved input was restored.
        // 38: saved re-author candidates skip only authoring and candidate judging; every reuse reruns host proof and passability gates. Prompt and judge instruction sources bind the key; freeze logic version and digest do not.
        // 39: baseline no-verdict strikes identify the failure cause, so re-authoring the same missing setup path still reaches its author.
        // 40: alias inputs can change mutation proof verdicts.
        // 41: passability finding selection and baseline-pass recognition
        // change the findings returned by a freeze gate.
        // Re-pinned only (verdict rules unchanged): #483
        // 42: batch judging includes bounded contents of existing named check inputs, preventing refutations that contradict their assertions.
        // 43: #486 nested repository sibling aliases reuse the repository copy
        // without creating a conflicting build target, changing scratch verdicts.
        // 44: #487 partial judge replies retain valid decisions and ask only
        // for missing checks, which can change the acceptance verdict set.
        // 45: #488 named-file assertions make their regression guards valid; move-aside and duplicate-implementation states cannot refute them. #489 separator trimming preserves the actual sibling path for setup classification.
        // 46: #491 acceptance reauthor verification accepts an intact predecessor skeleton as pending after a normal acceptance publish.
        // 47: mutation checks attribute recreated inputs to the check run;
        // whole-set publication compares the preparation-time contract
        // snapshot and rebuilds on change, including first-draft publication.
        // 48: recreated inputs with an unknown writer remain unproven, and
        // preparation binds the publication identity to the source digest.
        // 49: merge of 48 (#493 #492) and #495: cross-root parent paths are
        // refused before judging; one check's own setup-path failure goes to
        // its author after host-side integrity checks.
        // 54: #497 authored judge reply exhaustion and non-accepted re-author
        // checks remain per-check pending; timeout and zero-work checks route
        // back through their author, changing freeze repair outcomes.
        // 55: #497 leaves authored timeouts and incomplete repairs pending
        // per check, so independent checks keep their freeze verdicts.
        // 56: #497 keeps passability-judge authored-incomplete checks pending
        // while publishing proven siblings; typed whole-set freeze judge
        // replies are retried without an operational no-progress strike.
        // 57: #498 a named regression guard is valid only when its result
        // depends on the deciding test or implementation; moved-aside host
        // proof and file naming alone cannot establish that dependency.
        // 58: review of #491-#498: the judge sees each named input's status
        // and applies guard rules only to an existing deciding test; Enforce
        // partial republish, first-stall and typed missing-program routing,
        // stale-base rebuild and authored judge-reply retries change freeze
        // outcomes.
        // Same unreleased version 58: combined review of the merged fixes
        // (typed judge-incomplete kinds, site-lacks cause, passability
        // deferral, coverage gating of extensions, unaccepted host pending).
        // Same unreleased version 58: exit-path audit (check-caused faults to the
        // author or the strike, refusals authored, per-candidate judge holds,
        // cached-candidate refusal checks, probe beside missing siblings,
        // sibling scan of glued quoted paths).
        // 59: #495 #501 re-author path refusals follow the site's own sibling
        // discovery: a project_root path the site links is accepted; a path
        // that leaves its root and comes back into it by name is refused.
        // Same unreleased version 59: #500 a judge re-ask for missing ids uses
        // the caller's own prompt; re-author keeps decided verdicts when the
        // judge cannot complete.
        // 60: #516 a toolchain usage error in a check's own invocation is refuted by the
        // can-pass judge; #477 repository-sibling inputs are provable in the Scratch site;
        // #497 F2 a freeze stalls only when every finding is operational.
        // 61: #518 a path a check names only in message text no longer outranks a movable
        // input class; #519 nested inputs prove without a false unsafe restore; #494
        // re-author prompts prefetch inputs per attempt.
        // 62: batch 26: #520 a pinned or cached verdict counts only under the current judge
        // identity; #523 the host refutes external-program usage errors; #478 bare . and ..
        // are whole-tree inputs; #490 a pending skeleton is reported, not compared.
        // #524 the can-pass instruction names the evidence tree, not only the pre-implementation tree.
        // 68: #534 a sibling check rebuilt after a racing writer gets a real verdict instead of
        // host-unproven; #495 setup-path roots follow the site layout (process groups inside the
        // scratch tree, a cached target outside it); #544 a stored result whose masked copy would
        // decide differently is re-run, not reused.
        // 69: #548 a check whose site lacks a program the host has goes to its author, not host-unproven. 70: #549 each check is judged in its own call with its own named-file budget; one whose own files exceed it goes to its author with the files, sizes and limit, never refuted (re-pin only for every other capability). 71: #552 a check the judge gives no usable verdict in its own replies within the no-progress window goes to its author as a not-judged finding, never an operational stop; the other checks keep their verdicts; when a later step stalls, the checks already known not judged keep their own findings; an identical undecided check reuses its saved outcome while its named-file content is unchanged (re-pin only for every other capability). 72: #554 a check whose text its shell rejects in parse-only mode (`-n`) goes to its author unrun; a mutation run with no recorded status is the check's when `-n` rejects it, otherwise ambiguous (a strike), never a host fault; checks left unproven keep their findings in a stalled outcome (re-pin only for every other capability). 73: #557 a check that starts a program neither its site nor the host has goes to its author at once on the probe path, in the same budget with no strike, with a text that says no host action makes it available (never an operational host_program_absent finding or install advice), as the re-author path already did (re-pin only for every other capability). 74: #558 the deterministic cannot-pass rule (an outside program rejects the check's own command line) runs in the same freeze for every accepted check that failed on its evidence tree and has no probe finding of its own, even while other checks have author-scoped probe findings; the model-judged part stays deferred then (re-pin only for every other capability). 75: #560 a named path the host finds no file for is shown to the judge and both authors with a detail that it may be a pattern or text argument and is not alone evidence of missing behaviour; the judge and author rules read absence as behaviour not implemented yet only when the path is established to name a deciding file, which can change acceptance verdicts (re-pin only for every other capability). 76: Issue 566 a cd to a missing directory leaves later words unresolved, binary files are not judged source (marked not_judgeable), and case and substitution scopes change probe input discovery, named-file statuses and the judge instruction, which can change acceptance verdicts. 77: Issue 573 tab-indented REQ- bullets are requirements, and Issue 572 a nested definition-form constraint sub-bullet ends its parent requirement's text; both change the requirement inventory and texts that coverage findings and supplementary criteria are built from. Issue 571 changes the covers finding text and shape hints. 78: Issue 574/575 an acceptance republish re-runs the skeleton gate (workflow_acceptance_republish.rs), which now raises uncited_section/unknown_section and lists id-keyed table-row constraints; acceptance covers stay REQ- ids only. 79: Issue 577 agent programs inherit PSMODULEPATH and SHELL and Linux Landlock grants REFER within readable trees, so a check can now pass or fail differently. 80: Issue 579 inherited MAKEFLAGS, MFLAGS, GNUMAKEFLAGS and CARGO_MAKEFLAGS keep only allowlisted make options (no NAME=value words, no --eval or -E), and a Linux agent launch is refused when a Landlock read or write ruleset cannot be built, so a check can now pass or fail differently.
        version: 81, // #584 P1: mutation proof routing and verdicts change.
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
        // File-size split re-pin only: items moved unchanged into child modules; the closure follows the moved items; no verdict rule changed. #555 re-pin only: entry author prompts in the decomposition scripts carry only their own and phase-level findings, with bounded earlier sections, and the phase seed's criterion reader (workflow_decompose_seed_record.rs) ends at those sections; no verdict reads either. #559 re-pin only: an entry author prompt states the entry's current covers, a reply with no covers key keeps them, and the phase seed (workflow_decompose_seed_record.rs) takes them from that call's own recorded prompt line; no verdict reads either. #561 re-pin only: the decomposition script's acceptance progress window (workflow_decompose_v1_progress.js, new workflow_decompose_v1_failing_set.js) changes only when the loop pauses as no_progress; no verdict reads either. #559 guard re-pin only: the entry author reads the current entry with state.entries?.get (workflow_decompose_v1_acceptance.js), as the named-files context beside it does; no verdict reads it. #562 re-pin only: a gate finding carries an optional gate_stage and judged_item, written into the envelope only when set; the findings, their text and every pass or fail are unchanged, and only the decomposition script's progress window (workflow_decompose_v1_failing_set.js) reads the new fields; no verdict reads them. #537/#531 batch re-pin only: acceptance_scratch_control.rs gains a test-only clock hook (production now() is Instant::now()) and archon-core watchers are bounded; no verdict reads either. Issue 563 re-pin only: acceptance author and re-author prompts state the check site PATH (acceptance_scratch_policy.rs check_site_path, passed as a script argument kept out of launch identity); no verdict reads it. Issue 568 re-pin only: the skeleton author prompt names the previous candidate the author submitted to freeze-skeleton in a host-written context file (path and sha256); no verdict reads it. Issue 566/568/569: freeze-acceptance bumped to 76 for Issue 566; Issue 568 and Issue 569 change only skeleton prompts, the prdObligationIds script argument and skeleton finding text here. #355 re-pin only: host child launches gain a sandbox-exec spawn preflight (a missing target is a spawn error) and read-denied credential paths gain write and ancestor denials; the verdict launches here start a resolved shell, so no verdict changes. #355 forgery fix re-pin only: the host creates missing ancestors of protected paths and the grant root (0700) before it builds the profile, and every ancestor gets a full write denial; the verdict launches here start a resolved shell, so no verdict changes. Issue 355 merge with Issue 566/568/569 and 571/572/573 re-pin only: no verdict rule changed.
        sources_digest: "870f8ed0fad88aec7acf01d2186ac1b28a2a4220eeca352235850c19038780cb",
        build_bound: false,
    },
    CapabilityLogic {
        id: "freeze-skeleton",
        // 3: #386 an output overrun no longer fails the command; the verdict uses the full output.
        // #425 re-pin only: the repository claim parser changed; skeleton freeze verdicts do not read claims, and requirements-trace's version bump in this table does not change them.
        // 4: #444 redaction now preserves ordinary prose such as "Bearer authentication" instead of inserting a marker; freeze marker validation can therefore give a different verdict.
        // 5: review of #491: an attempt with open findings stays open when its logic is stale, and a recorded skeleton without its lock stays acceptance-only. 7: Issue 569 the unknown obligation finding names a non-obligation id and the host-listed id sources; freeze reuses by version only, so a replayed outcome would keep the old text. 8: Issue 572 skeleton freeze accepts host-listed PRD constraint ids in implements and raises no unowned_obligation for them; Issue 573 tab-indented REQ- bullets become obligations a task must claim. 9: Issue 574/575 frozen skeleton tasks carry source_sections, skeleton freeze raises uncited_section/unknown_section, and id-keyed table rows under a header with no obligation word become host-listed PRD constraints.
        version: 9,
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
        // File-size split re-pin only: items moved unchanged into child modules; the closure follows the moved items; no verdict rule changed. #555 re-pin only: entry author prompts in the decomposition scripts carry only their own and phase-level findings, with bounded earlier sections, and the phase seed's criterion reader (workflow_decompose_seed_record.rs) ends at those sections; no verdict reads either. #557 re-pin only: freeze-skeleton runs no check, so the probe path's routing and text of a program neither the site nor the host has never reach it. #558 re-pin only: freeze-skeleton runs no check and judges no baseline failure, so the passability stage's routing never reaches it. #559 re-pin only: an entry author prompt states the entry's current covers, a reply with no covers key keeps them, and the phase seed (workflow_decompose_seed_record.rs) takes them from that call's own recorded prompt line; no verdict reads either. #561 re-pin only: the decomposition script's acceptance progress window (workflow_decompose_v1_progress.js, new workflow_decompose_v1_failing_set.js) changes only when the loop pauses as no_progress; no verdict reads either. #559 guard re-pin only: the entry author reads the current entry with state.entries?.get (workflow_decompose_v1_acceptance.js), as the named-files context beside it does; no verdict reads it. #562 re-pin only: a gate finding carries an optional gate_stage and judged_item, written into the envelope only when set; the findings, their text and every pass or fail are unchanged, and only the decomposition script's progress window (workflow_decompose_v1_failing_set.js) reads the new fields; no verdict reads them. #560 re-pin only: freeze-skeleton judges no acceptance check, so the named-file absence detail and the judge and author absent-path rules never reach it. #537/#531 batch re-pin only: acceptance_scratch_control.rs gains a test-only clock hook (production now() is Instant::now()) and archon-core watchers are bounded; no verdict reads either. Issue 563 re-pin only: acceptance author and re-author prompts state the check site PATH (acceptance_scratch_policy.rs check_site_path, passed as a script argument kept out of launch identity); no verdict reads it. Issue 568 re-pin only: the skeleton author prompt names the previous candidate the author submitted to freeze-skeleton in a host-written context file (path and sha256); no verdict reads it. Issue 566/568/569: freeze-skeleton bumped to 7 for Issue 569; Issue 566 changes acceptance probe inputs and the acceptance judge only. #355 re-pin only: host child launches gain a sandbox-exec spawn preflight (a missing target is a spawn error) and read-denied credential paths gain write and ancestor denials; the verdict launches here start a resolved shell, so no verdict changes. #355 forgery fix re-pin only: the host creates missing ancestors of protected paths and the grant root (0700) before it builds the profile, and every ancestor gets a full write denial; the verdict launches here start a resolved shell, so no verdict changes. Issue 355 merge with Issue 566/568/569 and 571/572/573 re-pin only: no verdict rule changed. Issue 579 re-pin only: build-flag variables and the Linux agent-launch refusal reach check commands, and freeze-skeleton runs no check, so no verdict changes.
        sources_digest: "870f8ed0fad88aec7acf01d2186ac1b28a2a4220eeca352235850c19038780cb",
        build_bound: false,
    },
    CapabilityLogic {
        id: "land-task-body",
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
        version: 31,
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
        sources_digest: "a3af30df0b347eec95744ac31ade8a04456a4296225037ce5b36f2b8d9b61d09",
        build_bound: false,
    },
    CapabilityLogic {
        id: "task-set-lint",
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
        version: 26,
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
        sources_digest: "a3af30df0b347eec95744ac31ade8a04456a4296225037ce5b36f2b8d9b61d09",
        build_bound: false,
    },
    CapabilityLogic {
        id: "requirements-trace",
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
        version: 12,
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
        sources_digest: "3171c973a3c1f01e70034176dd530d95b003c58348d33675dc9c228098642360",
        build_bound: true,
    },
    CapabilityLogic {
        id: "verify-frozen-acceptance",
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
        version: 20, // #584 P1 re-pin: frozen-chain verification runs no check or mutation proof.
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
        // File-size split re-pin only: items moved unchanged into child modules; the closure follows the moved items; no verdict rule changed. #555 re-pin only: entry author prompts in the decomposition scripts carry only their own and phase-level findings, with bounded earlier sections, and the phase seed's criterion reader (workflow_decompose_seed_record.rs) ends at those sections; no verdict reads either. #557 re-pin only: frozen verification runs no probe; only the probe path's routing and text of a program neither the site nor the host has changed. #558 re-pin only: only this catalog's freeze-acceptance version and notes changed in this closure; frozen verification judges no baseline failure. #559 re-pin only: an entry author prompt states the entry's current covers, a reply with no covers key keeps them, and the phase seed (workflow_decompose_seed_record.rs) takes them from that call's own recorded prompt line; no verdict reads either. #561 re-pin only: the decomposition script's acceptance progress window (workflow_decompose_v1_progress.js, new workflow_decompose_v1_failing_set.js) changes only when the loop pauses as no_progress; no verdict reads either. #559 guard re-pin only: the entry author reads the current entry with state.entries?.get (workflow_decompose_v1_acceptance.js), as the named-files context beside it does; no verdict reads it. #562 re-pin only: a gate finding carries an optional gate_stage and judged_item, written into the envelope only when set; the findings, their text and every pass or fail are unchanged, and only the decomposition script's progress window (workflow_decompose_v1_failing_set.js) reads the new fields; no verdict reads them. #560 re-pin only: frozen-chain verification does not call the acceptance judge or an author; only the named-file view text, the rule texts and this catalog's freeze-acceptance version changed in this closure. #537/#531 batch re-pin only: acceptance_scratch_control.rs gains a test-only clock hook (production now() is Instant::now()) and archon-core watchers are bounded; no verdict reads either. Issue 563 re-pin only: acceptance author and re-author prompts state the check site PATH (acceptance_scratch_policy.rs check_site_path, passed as a script argument kept out of launch identity); no verdict reads it. Issue 568 re-pin only: the skeleton author prompt names the previous candidate the author submitted to freeze-skeleton in a host-written context file (path and sha256); no verdict reads it. Issue 566/568/569 re-pin only: frozen verification reads stored judgments and runs no probe, judge or skeleton set validation. Issue 571/572/573 re-pin only: frozen verification validates acceptance ids and stored judgments and runs no requirement, constraint, covers or skeleton set validation; only catalog versions, id extraction and finding text changed in this closure. #355 re-pin only: host child launches gain a sandbox-exec spawn preflight (a missing target is a spawn error) and read-denied credential paths gain write and ancestor denials; the verdict launches here start a resolved shell, so no verdict changes. Issue 355 merge with Issue 566/568/569 and 571/572/573 re-pin only: no verdict rule changed. Issue 574/575 re-pin only: frozen acceptance verification reads no source_sections, PRD section or constraint, so no verdict changes.
        sources_digest: "e9e2b75a8087706a52fd5fb78fefe6c952e11293eb4558ab37854481b4bc9f34",
        build_bound: true,
    },
    CapabilityLogic {
        id: "verify-frozen-skeleton",
        // 3: #386 an output overrun no longer fails the command; the verdict uses the full output.
        // 4: #444 redaction now preserves ordinary prose such as "Bearer authentication" instead of inserting a marker; freeze marker validation can therefore give a different verdict.
        // 5: #491 a skeleton bound to the prior acceptance is pending and skipped by the frozen snapshot.
        // 6: review of #491: a recorded skeleton without its lock stays acceptance-only, and open findings keep a stale-logic stage open. 8: Issue 574/575 the frozen skeleton task shape (workflow_freeze_schema.rs, FrozenTask) gains source_sections.
        version: 8,
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
        // File-size split re-pin only: items moved unchanged into child modules; the closure follows the moved items; no verdict rule changed. #555 re-pin only: entry author prompts in the decomposition scripts carry only their own and phase-level findings, with bounded earlier sections, and the phase seed's criterion reader (workflow_decompose_seed_record.rs) ends at those sections; no verdict reads either. #557 re-pin only: frozen verification runs no probe; only the probe path's routing and text of a program neither the site nor the host has changed. #558 re-pin only: only this catalog's freeze-acceptance version and notes changed in this closure; frozen verification judges no baseline failure. #559 re-pin only: an entry author prompt states the entry's current covers, a reply with no covers key keeps them, and the phase seed (workflow_decompose_seed_record.rs) takes them from that call's own recorded prompt line; no verdict reads either. #561 re-pin only: the decomposition script's acceptance progress window (workflow_decompose_v1_progress.js, new workflow_decompose_v1_failing_set.js) changes only when the loop pauses as no_progress; no verdict reads either. #559 guard re-pin only: the entry author reads the current entry with state.entries?.get (workflow_decompose_v1_acceptance.js), as the named-files context beside it does; no verdict reads it. #562 re-pin only: a gate finding carries an optional gate_stage and judged_item, written into the envelope only when set; the findings, their text and every pass or fail are unchanged, and only the decomposition script's progress window (workflow_decompose_v1_failing_set.js) reads the new fields; no verdict reads them. #560 re-pin only: frozen-chain verification does not call the acceptance judge or an author; only the named-file view text, the rule texts and this catalog's freeze-acceptance version changed in this closure. #537/#531 batch re-pin only: acceptance_scratch_control.rs gains a test-only clock hook (production now() is Instant::now()) and archon-core watchers are bounded; no verdict reads either. Issue 563 re-pin only: acceptance author and re-author prompts state the check site PATH (acceptance_scratch_policy.rs check_site_path, passed as a script argument kept out of launch identity); no verdict reads it. Issue 568 re-pin only: the skeleton author prompt names the previous candidate the author submitted to freeze-skeleton in a host-written context file (path and sha256); no verdict reads it. Issue 566/568/569 re-pin only: frozen verification reads stored judgments and runs no probe, judge or skeleton set validation. Issue 571/572/573 re-pin only: frozen verification validates acceptance ids and stored judgments and runs no requirement, constraint, covers or skeleton set validation; only catalog versions, id extraction and finding text changed in this closure. #355 re-pin only: host child launches gain a sandbox-exec spawn preflight (a missing target is a spawn error) and read-denied credential paths gain write and ancestor denials; the verdict launches here start a resolved shell, so no verdict changes. Issue 355 merge with Issue 566/568/569 and 571/572/573 re-pin only: no verdict rule changed.
        sources_digest: "e9e2b75a8087706a52fd5fb78fefe6c952e11293eb4558ab37854481b4bc9f34",
        build_bound: true,
    },
];
