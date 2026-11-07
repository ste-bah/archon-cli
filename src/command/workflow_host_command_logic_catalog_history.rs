//! Historical freeze-acceptance bump and re-pin notes.
//! Kept separately so the catalog has room for integration notes.

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

// Previous #584 P5 requirements-trace note: section remedy text only; no pass/fail
// change; operator re-pins digest. Previous P5 verify-frozen-acceptance note:
// reads no changed skeleton or section rule; operator re-pins digest.
// #584 integration, relative to main 6f456cde8 (one combined note per capability):
// freeze-acceptance 80 -> 81: P1 mutation/baseline proof, P2+P3 setup status and
// sibling/site rules (including R1), P4 judge/author rules and P5 republish
// validation change verdicts; P6 and A20 remedy wording share the source closure.
// freeze-skeleton 9 -> 10 (P5, already bumped): skeleton validation changes
// verdicts; re-pin P1, P2+P3, P4, P6 and integration source changes only.
// land-task-body 31 -> 32: P5 pending skeleton validation and P6 body-phase
// findings change verdicts; re-pin P1, P2+P3, P4 and integration closure changes.
// task-set-lint 26 -> 27 (P5, already bumped): P5 skeleton claims and P6 set-gate
// findings change verdicts; re-pin P1, P2+P3, P4 and integration closure changes.
// requirements-trace 12 -> 13: P6 shared untestable-claims rules change
// verdicts; re-pin P1, P2+P3, P4, P5 and integration closure changes.
// verify-frozen-acceptance 20 -> 21: P2+P3 sibling copying and setup-failure
// classification change verdicts; re-pin P1, P4, P5, P6 and A20 wording changes.
// verify-frozen-skeleton 8 -> 9 (P5, already bumped): normalized skeleton
// equality changes verdicts; re-pin P1, P2+P3, P4, P6 and integration changes only.

// #587/#588: task-set-lint 28 routes critic findings without verified body
// evidence to Skeleton. Re-pin other changed closures only: landing completion
// and repair hints change author sessions, not their gate verdict rules.

// #589: requirements-trace 13 -> 14: the empty normalized repository-root
// path is present, so a claim naming the root is no longer falsely refuted.
// Re-pin other capabilities only: their verdict rules do not read this index.

// #590: freeze-acceptance 81 -> 82:
// B2 resolves package selectors through the check site's combined repository
// layout and reports every distinct directory/package mismatch in a script.
// Guarded scripts and heredoc data supply no B2 finding; only workspace package
// selectors are scanned. verify-frozen-acceptance stays 21 (runs no probe).
// Re-pin other changed closures only; their verdict rules do not use B2.

// 83: #597 fatal check expansion diagnostics override a masked exit status.
// 84: #599 acceptance republish also validates the retained skeleton contracts.
