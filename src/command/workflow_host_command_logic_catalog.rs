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
        // #434 re-pin only: the fidelity critic's store and reply schema are in this closure; freeze never asks the critic.
        // 14: #417 task-root exclusions and copies use the same project-relative input set as its digest.
        // #417 re-pin only: direct copy calls now normalize roots like the production caller.
        // 15: #417 Windows task-root paths normalize before containment checks, so probes can run and produce a different verdict.
        // #439 re-pin only: resume phase selection shares this closure; freeze verdicts are unchanged.
        // #417 re-pin only: the landing destination allowlist names the topology stores; freeze verdicts are unchanged.
        // #425 re-pin only: the repository claim parser changed; freeze verdicts do not read claims.
        // #425 re-pin only: the repository claim parser changed; freeze verdicts do not read claims, and requirements-trace's version bump in this table does not change them.
        // #434c re-pin only: validated fidelity-cache reuse does not alter frozen verdicts.
        // #438 re-pin only: relative path keys use '/' on every OS; macOS/Linux keys and verdicts are unchanged.
        // #424 #369 #383 #438 re-pin only: stream usage counting, the acceptance author's refusal redaction and Windows-only stored path spelling changed; no verdict changes, and on macOS/Linux this subcommand's findings and report text are unchanged.
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
        // re-pin only: verdicts are unchanged.
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
        version: 35,
        sources: &[GATE, CONTRACT, FREEZE],
        // #434d re-pin only: the fidelity critic keeps each passing verdict of a reply and its store keys on a critic logic version; freeze never asks the critic, so its verdicts are unchanged.
        // #422 re-pin only: a resume after SIGKILL probes the recorded process scope of a host command; no freeze verdict reads it.
        // #386 re-pin only: a candidate past the inline stdin bound reaches the child whole through a run spill; a candidate that fits is read as before, and one past the bound never had a verdict.
        // #387 re-pin only: the launch and resume paths in this closure (frozen-contract launch decision, requirement inventory) do not change how a freeze judges.
        // #387 re-pin only: a launch routes a frozen check this build's entry rules refuse to the existing per-check repair, after it claims the task root; the freeze judges as before.
        // #442 re-pin only: the base-tree suffix lookup moved into RepositoryTree::paths_ending_with; freeze verdicts do not read claims, and requirements-trace's version bump in this table does not change them.
        // #443 re-pin only: the repository claim parser reads assertion clauses and the lint versions moved; freeze verdicts do not read claims.
        // #441 re-pin only: tolerant event-log reads changed; freeze-acceptance verdicts are unchanged.
        // #447 re-pin only: landing history replay and reconciliation changed; acceptance-freeze verdicts are unchanged.
        // #447 round 9 re-pin only: atomic receipt-mismatch capture changes landing recovery, not frozen acceptance verdicts.
        // #446 follow-up re-pin only (version 18 at the time): cache compatibility now keeps old evidence;
        // Scratch schema-3 verdicts stay reusable, while Hermetic layoutless
        // evidence reruns under the version-18 acceptance pin. Version 19 records #402/#419 changes.
        // #446 follow-up re-pin only: archive collisions keep both prior byte sets; verdicts are unchanged.
        // integration re-pin: merge of #447, #446, misc2, batch3 and resume2 sources; no logic change
        // integration re-pin: merge of #447, #446, misc2, batch3 and resume2 sources; no logic change
        // integration re-pin: #386 spill view cap removed (house rule: no new limits)
        // #369 re-pin only: usage availability and cost recording affect telemetry; sanitizing persisted records changes representation, not acceptance verdicts.
        // #340 re-pin only: the acceptance scratch uses the stdio-only spawn helper for Git; its freeze verdicts are unchanged.
        // # resource defaults re-pin only: configured build parallelism changes scheduling, not check verdicts.
        // #457: baseline-passing checks may use a bound accepted judge verdict; the can-fail proof remains required.
        // #457 follow-up re-pin only: directory and variable flow jointly refine input discovery; freeze-acceptance is version 30 after the #444 merge.
        // #457 follow-up re-pin only: transient recreation of moved check inputs invalidates a stored verdict; freeze-acceptance remains version 28 (unreleased).
        // Re-pin only: the held executor lease now authenticates paused-run
        // recovery; acceptance-freeze verdicts are unchanged.
        // #448 re-pin only: acceptance_scratch_hooks.rs, acceptance_scratch_observe.rs, acceptance_scratch_guardian.rs and workflow_decompose.rs route resolved cargo limits into check execution; these limits affect execution environment and parallelism, not verdict rules.
        // #448 re-pin only: script-tool config propagation and Bash classifier preservation do not change host-command verdicts.
        // 34: #420 keeps same-spelled siblings from different roots distinct,
        // so a false Scratch revision conflict no longer voids a check.
        // 35: sibling revisions are part of verdict reuse identity; changed
        // sibling inputs cannot reuse an earlier check result.
        // #219 re-pin only: reverify remediation routes check defects to frozen check entries; freeze verdicts are unchanged.
        // Re-pin only: the parser change does not alter freeze verdicts.
        sources_digest: "cb1f5f6fa54ead8df91b7fabb40798f1ffcbcf6ec38c4e19e270276dbecb1fbb",
        build_bound: false,
    },
    CapabilityLogic {
        id: "freeze-skeleton",
        // 3: #386 an output overrun no longer fails the command; the verdict uses the full output.
        // #434 and #417 re-pin only: harness input paths, pipeline outputs, and copy API normalization do not alter this verdict.
        // #439 re-pin only, as freeze-acceptance.
        // #425 re-pin only: the repository claim parser changed; skeleton freeze verdicts do not read claims.
        // #425 re-pin only: the repository claim parser changed; skeleton freeze verdicts do not read claims, and requirements-trace's version bump in this table does not change them.
        // #434c re-pin only: validated fidelity-cache reuse does not alter frozen verdicts.
        // #438 re-pin only: relative path keys use '/' on every OS; macOS/Linux keys and verdicts are unchanged.
        // #424 #369 #383 #438 re-pin only: stream usage counting, the acceptance author's refusal redaction and Windows-only stored path spelling changed; no verdict changes, and on macOS/Linux this subcommand's findings and report text are unchanged.
        // #366 re-pin only: skeleton freeze performs no check path classification.
        // re-pin only: verdicts are unchanged.
        // 4: #444 redaction now preserves ordinary prose such as "Bearer authentication" instead of inserting a marker; freeze marker validation can therefore give a different verdict.
        version: 4,
        sources: &[GATE, CONTRACT, FREEZE],
        // #434d re-pin only: the fidelity critic keeps each passing verdict of a reply and its store keys on a critic logic version; freeze never asks the critic, so its verdicts are unchanged.
        // #422 re-pin only: a resume after SIGKILL probes the recorded process scope of a host command; no freeze verdict reads it.
        // #386 re-pin only: a candidate past the inline stdin bound reaches the child whole through a run spill; a candidate that fits is read as before, and one past the bound never had a verdict.
        // #387 re-pin only: the launch and resume paths in this closure (frozen-contract launch decision, requirement inventory) do not change how a freeze judges.
        // #387 re-pin only: a launch routes a frozen check this build's entry rules refuse to the existing per-check repair, after it claims the task root; the freeze judges as before.
        // #442 re-pin only: the base-tree suffix lookup moved into RepositoryTree::paths_ending_with; freeze verdicts do not read claims, and requirements-trace's version bump in this table does not change them.
        // #443 re-pin only: the repository claim parser reads assertion clauses and the lint versions moved; freeze verdicts do not read claims.
        // #441 re-pin only: tolerant event-log reads changed; freeze-skeleton verdicts are unchanged.
        // #447 re-pin only: landing history replay and reconciliation changed; skeleton-freeze verdicts are unchanged.
        // #447 round 9 re-pin only: atomic receipt-mismatch capture changes landing recovery, not frozen skeleton verdicts.
        // #440 re-pin only: the skeleton freeze runs no check and copies no input.
        // #420 re-pin only: the skeleton freeze judges no check run, so the new no-verdict rule never applies.
        // #440 #420 re-pin only: line-ceiling layout of the scratch and hermetic modules (imports, a re-export); no logic change.
        // #446 re-pin only: the skeleton freeze judges no baseline run.
        // #446 follow-up re-pin only: the shared probe cache source changed;
        // freeze-skeleton judges no check runs.
        // #446 follow-up: archive collisions keep both prior byte sets.
        // #374 re-pin only: skeleton freeze performs no check path classification.
        // integration re-pin: merge of #447, #446, misc2, batch3 and resume2 sources; no logic change
        // integration re-pin: merge of #447, #446, misc2, batch3 and resume2 sources; no logic change
        // integration re-pin: #386 spill view cap removed (house rule: no new limits)
        // #369 re-pin only: usage availability and cost recording affect telemetry; sanitizing persisted records changes representation, not skeleton verdicts.
        // #340 re-pin only: the acceptance scratch uses the stdio-only spawn helper for Git; skeleton freeze verdicts are unchanged.
        // # resource defaults re-pin only: skeleton freeze does not judge checks.
        // Re-pin only: terminal finalizer helpers moved into sibling modules; freeze verdicts are unchanged.
        // #457 re-pin only: acceptance probe changes do not affect skeleton freeze verdicts.
        // #457 follow-up re-pin only: this closure is shared; the skeleton verdict is unchanged.
        // #457 sibling-input classification changes can-fail proof outcomes only, not skeleton freeze verdicts.
        // #457 re-pin only: directory and variable flow are shared; the skeleton verdict is unchanged.
        // #457 re-pin only: inline acceptance input discovery does not change skeleton freeze verdicts.
        // Re-pin only: the held executor lease now authenticates paused-run
        // recovery; skeleton-freeze verdicts are unchanged.
        // #448 re-pin only: acceptance_scratch_hooks.rs, acceptance_scratch_observe.rs, acceptance_scratch_guardian.rs and workflow_decompose.rs route resolved cargo limits into scratch checks; skeleton freeze runs no checks, so its verdict is unchanged.
        // #448 re-pin only: script-tool config propagation and Bash classifier preservation do not change host-command verdicts.
        // #420 re-pin only: sibling input handling is outside the frozen skeleton verdict.
        // #420 re-pin only: sibling revision evidence is outside skeleton judgments.
        // #219 re-pin only: reverify remediation routing does not change frozen skeleton verification verdicts.
        // Re-pin only: the parser change does not alter freeze verdicts.
        sources_digest: "cb1f5f6fa54ead8df91b7fabb40798f1ffcbcf6ec38c4e19e270276dbecb1fbb",
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
        // #434c re-pin only: identical critic logic and request identities reuse validated verdicts across binary rebuilds.
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
        version: 21,
        sources: &[GATE, CONTRACT, LINT],
        // #386 re-pin only: the staged lint reads a spilled candidate whole; a candidate that fits the pipe is read as before.
        // #387 re-pin only: the entry validator is re-exported for the launch decision; lint verdicts are unchanged.
        // #387 re-pin only: launch routing of refused frozen checks; lint verdicts are unchanged.
        // #425 re-pin only: unscoped negated bare names, filler before a scoped predicate, base-name, slashed and multi-directory scopes (each directory one reading); same version bump as above.
        // #434d re-pin only: a fidelity reply is read verdict by verdict and each verdict kept passes every check a whole reply passes; the store keys on CRITIC_LOGIC_VERSION 1 and migrates schema-4/5 records, so no verdict changes meaning.
        // #434d review re-pin only: a build whose critic sources are not the pinned ones keeps no fidelity record and asks afresh; an opposite re-answer of a kept verdict is only logged. Verdict meaning is unchanged.
        // #424 #369 #383 #438 re-pin only: stream usage counting, the acceptance author's refusal redaction and Windows-only stored path spelling changed; no verdict changes, and on macOS/Linux this subcommand's findings and report text are unchanged.
        // #442 re-pin only: scope resolution reads base-tree suffixes through RepositoryTree::paths_ending_with, the same paths as before; verdicts are unchanged.
        // #443: the version bumps above; asserted runtime paths are no longer repository claims.
        // Re-pin only: observe-mode refused bodies now stay unpublished; refused candidates never publish, existing results remain valid, and lineage protects any future version bump.
        // #441: this gate asks fidelity with PRD context and the candidate fallback.
        // #441 review 2, same unreleased version: Acceptance policy headings reach the body critic.
        // #441 review 3, same unreleased version: the complete PRD reaches the body critic.
        // #434d, same unreleased version 14: task quote provenance accepts bounded normalization and ordered elisions.
        // #441 regression re-pin only: task ellipses are literal evidence when the whole quote matches; verdict meaning is unchanged.
        // #434d review re-pin: quote normaliser split into its own module; same unreleased versions
        // #441: task-shape checks now read focused test commands from the runtime task model.
        // 17: #447 receipt-mismatch self-healing settles the newest durable task body before judging it.
        // A12 re-pin only: A9 already bumped this version for malformed-body verdicts.
        // integration re-pin: merge of #447, #446, misc2, batch3 and resume2 sources; no logic change
        // integration re-pin: merge of #447, #446, misc2, batch3 and resume2 sources; no logic change
        // integration re-pin: #386 spill view cap removed (house rule: no new limits)
        // #369 re-pin only: usage availability and cost recording affect telemetry; sanitizing persisted records changes representation, not body landing verdicts.
        // #386 re-pin only: staged lint reads spill-backed candidates whole; inline candidates follow the existing path.
        // # resource defaults re-pin only: configured build parallelism changes scheduling, not body verdicts.
        // Re-pin only: the held executor lease now authenticates paused-run
        // recovery; body-landing verdicts are unchanged.
        // #448 re-pin only: acceptance_scratch_hooks.rs and workflow_decompose.rs forward resolved cargo limits to observation commands; staged body lint verdict rules are unchanged.
        // #448 re-pin only: script-tool config propagation and Bash classifier preservation do not change host-command verdicts.
        sources_digest: "d052b7539d6c98b81d15ad52afb1474800b6a3c3580542c4a004f8663e386de7",
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
        // #434c re-pin only: identical critic logic and request identities reuse validated verdicts across binary rebuilds.
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
        version: 17,
        sources: &[GATE, CONTRACT, LINT],
        // #386 re-pin only: the staged lint reads a spilled candidate whole; a candidate that fits the pipe is read as before.
        // #387 re-pin only: the entry validator is re-exported for the launch decision; lint verdicts are unchanged.
        // #387 re-pin only: launch routing of refused frozen checks; lint verdicts are unchanged.
        // #425 re-pin only: unscoped negated bare names, filler before a scoped predicate, base-name, slashed and multi-directory scopes (each directory one reading); same version bump as above.
        // #434d re-pin only: a fidelity reply is read verdict by verdict and each verdict kept passes every check a whole reply passes; the store keys on CRITIC_LOGIC_VERSION 1 and migrates schema-4/5 records, so no verdict changes meaning.
        // #434d review re-pin only: a build whose critic sources are not the pinned ones keeps no fidelity record and asks afresh; an opposite re-answer of a kept verdict is only logged. Verdict meaning is unchanged.
        // #424 #369 #383 #438 re-pin only: stream usage counting, the acceptance author's refusal redaction and Windows-only stored path spelling changed; no verdict changes, and on macOS/Linux this subcommand's findings and report text are unchanged.
        // #442 re-pin only: scope resolution reads base-tree suffixes through RepositoryTree::paths_ending_with, the same paths as before; verdicts are unchanged.
        // #443: the version bumps above; asserted runtime paths are no longer repository claims.
        // Re-pin only: the staged CLI exposes the body packaging step to the resume's landing
        // reconciliation; no gate calls the new entry, so every verdict is unchanged.
        // #441: this gate judges fidelity with PRD acceptance and gap context.
        // #441 review 2, same unreleased version: Acceptance policy headings reach the set critic.
        // #441 review 3, same unreleased version: the complete PRD reaches the set critic.
        // #434d, same unreleased version 13: the set gate shares the updated quote provenance check.
        // #441 regression re-pin only: task ellipses are literal evidence when the whole quote matches; verdict meaning is unchanged.
        // #434d review re-pin: quote normaliser split into its own module; same unreleased versions
        // #441: focused file ownership and declaration findings use the runtime command rule.
        // #447 re-pin only: receipt mismatch repair stays outside the set lint verdict.
        // A12 re-pin only: A9 already bumped this version for malformed-body verdicts.
        // integration re-pin: merge of #447, #446, misc2, batch3 and resume2 sources; no logic change
        // integration re-pin: merge of #447, #446, misc2, batch3 and resume2 sources; no logic change
        // integration re-pin: #386 spill view cap removed (house rule: no new limits)
        // #369 re-pin only: usage availability and cost recording affect telemetry; sanitizing persisted records changes representation, not lint verdicts.
        // #386 re-pin only: staged lint reads spill-backed candidates whole; inline candidates follow the existing path.
        // # resource defaults re-pin only: configured build parallelism changes scheduling, not lint verdicts.
        // Re-pin only: the held executor lease now authenticates paused-run
        // recovery; lint verdicts are unchanged.
        // #448 re-pin only: acceptance_scratch_hooks.rs and workflow_decompose.rs forward resolved cargo limits to observation commands; lint verdict rules are unchanged.
        // #448 re-pin only: script-tool config propagation and Bash classifier preservation do not change host-command verdicts.
        sources_digest: "d052b7539d6c98b81d15ad52afb1474800b6a3c3580542c4a004f8663e386de7",
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
        version: 5,
        sources: &[GATE, CONTRACT, TRACE],
        // #434 and #417 re-pin only: trace never reads the newly excluded run state.
        // #434d re-pin only: trace never asks the fidelity critic or reads its store.
        // #438 re-pin only: relative path keys use '/' on every OS; macOS/Linux keys and verdicts are unchanged.
        // #441 re-pin only: shared source closure changed; requirements-trace does not run fidelity.
        // #424 #369 #383 #438 re-pin only: stream usage counting, the acceptance author's refusal redaction and Windows-only stored path spelling changed; no verdict changes, and on macOS/Linux this subcommand's findings and report text are unchanged.
        // #440 re-pin only: trace calls none of the scratch copy, audit or probe digest walkers that changed.
        // #440 #420 re-pin only: line-ceiling layout of the scratch and hermetic modules (imports, a re-export); no logic change.
        // integration re-pin: merge of #447, #446, misc2, batch3 and resume2 sources; no logic change
        // # resource defaults re-pin only: configured build parallelism changes scheduling, not trace verdicts.
        sources_digest: "82860cd934b1173968563bd548c96bd1ada1360dcd70a4ab3c3f633a69462ad1",
        build_bound: true,
    },
    CapabilityLogic {
        id: "verify-frozen-acceptance",
        // 3: #386 an output overrun no longer fails the command; the verdict uses the full output.
        // 4: #417 copied check inputs omit freeze-owned files, which can change a check verdict.
        // 5: #417 harness-written project run state leaves hermetic check inputs, which can change a check verdict.
        // 6: #417 copied task-root inputs now match the digest exclusions, which can change a check verdict.
        // #417 re-pin only: the direct copy API now shares the production caller's root normalization.
        // #434c re-pin only: validated fidelity-cache reuse does not alter frozen-chain verification verdicts.
        // 8: #366 rechecks frozen acceptance paths using shell assignment scope when locating live roots (POSIX spelling case-sensitive on Windows too).
        // re-pin only: verdicts are unchanged.
        // 9: every spawned check receives resolved resource limits.
        // 10: a legacy guardian request without resource_limits is rejected,
        // instead of silently running with default limits.
        // 11: #444 redaction now preserves ordinary prose such as "Bearer authentication" instead of inserting a marker; freeze marker validation can therefore give a different verdict.
        version: 11,
        sources: &[GATE, CONTRACT, VERIFY],
        // #434 and #417 re-pin only: skeleton verification never reads excluded run state or pipeline outputs.
        // #439 re-pin only: resume phase selection does not change verification verdicts.
        // #425 re-pin only: the repository claim parser changed; acceptance verification verdicts do not read claims.
        // #434d re-pin only: verification never asks the fidelity critic or reads its store.
        // #438 re-pin only: relative path keys use '/' on every OS; macOS/Linux keys and verdicts are unchanged.
        // #386 re-pin only: stdin spill and progress-event spill code in this closure; verification takes no stdin and its verdicts are unchanged.
        // #387 re-pin only: the launch decision lives beside verify_frozen_stage, which is unchanged.
        // #387 re-pin only: the launch decision returns its routing instead of refusing; verify_frozen_stage is unchanged.
        // #424 #369 #383 #438 re-pin only: stream usage counting, the acceptance author's refusal redaction and Windows-only stored path spelling changed; no verdict changes, and on macOS/Linux this subcommand's findings and report text are unchanged.
        // #442 re-pin only: requirements-trace's version bump in this table; verification verdicts are unchanged.
        // #443 re-pin only: the repository claim parser reads assertion clauses and the lint versions moved; verification verdicts do not read claims.
        // Re-pin only: the executor names whether a capability publishes, and moved its digest
        // lookup; the reuse rules a verification verdict is checked under are unchanged.
        // #441 re-pin only: capability logic versions changed; frozen acceptance verdicts are unchanged.
        // #447 re-pin only: landing history replay and reconciliation changed; frozen acceptance verdicts are unchanged.
        // #447 round 9 re-pin only: atomic receipt-mismatch capture does not change frozen acceptance verification.
        // #440 re-pin only: frozen-chain verification runs no check and calls none of the changed copy, audit or digest walkers.
        // #420 re-pin only: frozen-chain verification judges no check run, so the new no-verdict rule never applies.
        // #420 review re-pin only: rustfmt layout of the executability modules; no logic change.
        // #440 #420 re-pin only: line-ceiling layout of the scratch and hermetic modules (imports, a re-export); no logic change.
        // #446 re-pin only: baseline judgment changes; frozen-artifact verification does not.
        // #446 follow-up re-pin only: verification shares cache source files
        // but reads no probe result layout or evidence.
        // #374 re-pin only: frozen-chain verification does not classify check paths.
        // integration re-pin: merge of #447, #446, misc2, batch3 and resume2 sources; no logic change
        // integration re-pin: merge of #447, #446, misc2, batch3 and resume2 sources; no logic change
        // integration re-pin: file split, no logic change
        // #369 re-pin only: usage availability and cost recording affect telemetry; sanitizing persisted records changes representation, not frozen-chain acceptance verdicts.
        // #340 re-pin only: the acceptance scratch uses the stdio-only spawn helper for Git; frozen acceptance verification is unchanged.
        // # resource defaults re-pin only: frozen verification's check execution is scheduling-only.
        // #457 re-pin only: frozen-chain verification does not run the can-fail probe or read named inputs.
        // #457 reverify report evidence and can-fail proof logic do not affect frozen-chain verification.
        // #457 re-pin only: inline acceptance input discovery does not affect frozen-chain verification.
        // Re-pin only: the held executor lease now authenticates paused-run
        // recovery; frozen acceptance verification is unchanged.
        // #448 re-pin only: acceptance_scratch_observe.rs and workflow_decompose.rs pass resolved cargo limits into frozen check execution; limits change execution environment and parallelism, not verification verdict rules.
        // #448 re-pin only: script-tool config propagation and Bash classifier preservation do not change host-command verdicts.
        // #457 re-pin only: freeze-acceptance logic version 32 changes the can-fail proof, not frozen-chain verification.
        // #420 re-pin only: per-check sibling report evidence does not change frozen acceptance verdicts.
        // #420 re-pin only: frozen acceptance checks run under the build-bound verifier.
        // #219 re-pin only: report remediation routing does not alter frozen acceptance verification verdicts.
        // Re-pin only: the parser change does not alter verification verdicts.
        sources_digest: "e374ec73892723cd13598a18f867bf06c025c499d43541ac8e21de756b22f1d9",
        build_bound: true,
    },
    CapabilityLogic {
        id: "verify-frozen-skeleton",
        // 3: #386 an output overrun no longer fails the command; the verdict uses the full output.
        // #366 re-pin only: frozen skeleton verification does not classify check paths.
        // re-pin only: verdicts are unchanged.
        // 4: #444 redaction now preserves ordinary prose such as "Bearer authentication" instead of inserting a marker; freeze marker validation can therefore give a different verdict.
        version: 4,
        sources: &[GATE, CONTRACT, VERIFY],
        // #434 and #417 re-pin only: verify never asks the fidelity critic or reads pipeline outputs.
        // #439 re-pin only: resume phase selection does not change verification verdicts.
        // #425 re-pin only: the repository claim parser changed; skeleton verification verdicts do not read claims.
        // #434c re-pin only: validated fidelity-cache reuse does not alter frozen-chain verification verdicts.
        // #434d re-pin only: verification never asks the fidelity critic or reads its store.
        // #438 re-pin only: relative path keys use '/' on every OS; macOS/Linux keys and verdicts are unchanged.
        // #386 re-pin only: stdin spill and progress-event spill code in this closure; verification takes no stdin and its verdicts are unchanged.
        // #387 re-pin only: the launch decision lives beside verify_frozen_stage, which is unchanged.
        // #387 re-pin only: the launch decision returns its routing instead of refusing; verify_frozen_stage is unchanged.
        // #424 #369 #383 #438 re-pin only: stream usage counting, the acceptance author's refusal redaction and Windows-only stored path spelling changed; no verdict changes, and on macOS/Linux this subcommand's findings and report text are unchanged.
        // #442 re-pin only: requirements-trace's version bump in this table; verification verdicts are unchanged.
        // #443 re-pin only: the repository claim parser reads assertion clauses and the lint versions moved; verification verdicts do not read claims.
        // Re-pin only: the executor names whether a capability publishes, and moved its digest
        // lookup; the reuse rules a verification verdict is checked under are unchanged.
        // #441 re-pin only: capability logic versions changed; frozen skeleton verdicts are unchanged.
        // #447 re-pin only: landing history replay and reconciliation changed; frozen skeleton verdicts are unchanged.
        // #447 round 9 re-pin only: atomic receipt-mismatch capture does not change frozen skeleton verification.
        // #440 re-pin only: frozen-chain verification runs no check and calls none of the changed copy, audit or digest walkers.
        // #420 re-pin only: frozen-chain verification judges no check run, so the new no-verdict rule never applies.
        // #420 review re-pin only: rustfmt layout of the executability modules; no logic change.
        // #440 #420 re-pin only: line-ceiling layout of the scratch and hermetic modules (imports, a re-export); no logic change.
        // #446 re-pin only: baseline judgment changes; frozen-artifact verification does not.
        // #446 follow-up re-pin only: verification shares cache source files
        // but reads no probe result layout or evidence.
        // #374 re-pin only: frozen-chain verification does not classify check paths.
        // integration re-pin: merge of #447, #446, misc2, batch3 and resume2 sources; no logic change
        // integration re-pin: merge of #447, #446, misc2, batch3 and resume2 sources; no logic change
        // integration re-pin: file split, no logic change
        // #369 re-pin only: usage availability and cost recording affect telemetry; sanitizing persisted records changes representation, not frozen-chain skeleton verdicts.
        // #340 re-pin only: the acceptance scratch uses the stdio-only spawn helper for Git; frozen skeleton verification is unchanged.
        // # resource defaults re-pin only: frozen skeleton verification runs no checks.
        // #457 re-pin only: frozen verification runs no check and judges no probe result.
        // #457 reverify report evidence and can-fail proof logic do not affect frozen-chain verification.
        // #457 re-pin only: inline acceptance input discovery does not affect frozen-chain verification.
        // Re-pin only: the held executor lease now authenticates paused-run
        // recovery; frozen skeleton verification is unchanged.
        // #448 re-pin only: acceptance_scratch_observe.rs and workflow_decompose.rs pass resolved cargo limits into shared observation code; skeleton verification runs no checks, so its verdict is unchanged.
        // #448 re-pin only: script-tool config propagation and Bash classifier preservation do not change host-command verdicts.
        // #457 re-pin only: freeze-acceptance logic version 32 does not affect frozen skeleton verification.
        // #420 re-pin only: sibling report evidence is outside frozen skeleton verification.
        // #420 re-pin only: sibling revision evidence is outside skeleton judgments.
        // #219 re-pin only: report remediation routing does not alter frozen skeleton verification verdicts.
        // Re-pin only: the parser change does not alter verification verdicts.
        sources_digest: "e374ec73892723cd13598a18f867bf06c025c499d43541ac8e21de756b22f1d9",
        build_bound: true,
    },
];
