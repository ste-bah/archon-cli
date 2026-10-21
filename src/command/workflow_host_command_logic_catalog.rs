use super::{CONTRACT, CapabilityLogic, FREEZE, GATE, LINT, TRACE, VERIFY};

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
        version: 41,
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
        sources_digest: "9f71cf0a08fb43c589907366749ec55e9148981c38d48af9060a0326b7b0313b",
        build_bound: false,
    },
    CapabilityLogic {
        id: "freeze-skeleton",
        // 3: #386 an output overrun no longer fails the command; the verdict uses the full output.
        // #425 re-pin only: the repository claim parser changed; skeleton freeze verdicts do not read claims, and requirements-trace's version bump in this table does not change them.
        // 4: #444 redaction now preserves ordinary prose such as "Bearer authentication" instead of inserting a marker; freeze marker validation can therefore give a different verdict.
        version: 4,
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
        sources_digest: "9f71cf0a08fb43c589907366749ec55e9148981c38d48af9060a0326b7b0313b",
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
        version: 23,
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
        sources_digest: "89f09f50fdad9704332fe4c2f668fe04c0487cedde9ec6335249983a22eb414f",
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
        version: 19,
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
        sources_digest: "89f09f50fdad9704332fe4c2f668fe04c0487cedde9ec6335249983a22eb414f",
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
        version: 6,
        // Re-pinned only (verdict rules unchanged): #434 #417 #438 #441 #424 #369 #383 #440 #420 #447 #446 #464 #466 #469
        sources_digest: "6bbb593801828f7f3c8ba44cab243ea3d5baae9c60e1ed28a685abe5ca4b2c6f",
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
        version: 11,
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
        sources_digest: "e8f6a2412af8ece820b622efcc09fed54d90b03d649f45b3a84f00b6dbf39ba3",
        build_bound: true,
    },
    CapabilityLogic {
        id: "verify-frozen-skeleton",
        // 3: #386 an output overrun no longer fails the command; the verdict uses the full output.
        // 4: #444 redaction now preserves ordinary prose such as "Bearer authentication" instead of inserting a marker; freeze marker validation can therefore give a different verdict.
        version: 4,
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
        sources_digest: "e8f6a2412af8ece820b622efcc09fed54d90b03d649f45b3a84f00b6dbf39ba3",
        build_bound: true,
    },
];
