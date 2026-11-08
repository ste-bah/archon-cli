// Issue 362: a clean author round leaves nothing to re-author. A freeze that
// fails operationally (no findings) is retried with the same entries and no
// author call; a persisting outage pauses (resumable), never re-authors.
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_v1_body.js', 'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js', "workflow_decompose_v1_prompt.js", 'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_failing_set.js', 'workflow_decompose_v1_context.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');
const clean = () => ({publicationReceipt:{call_id:'freeze'}, postcondition:{satisfied:true},
  gateEnvelope:{policy_findings:[]}});
const outage = () => ({gateEnvelope:{operational_error:{text:'freeze backend unavailable'}}});
const refute = id => ({...clean(), gateEnvelope:{policy_findings:[{
  text:`check '${id}' was refuted: repair`, subject:id, remediation_scope:'candidate_artifact'}]}});

// `gates` is the freeze outcome sequence. A pause is resumed (returns).
async function run(gates) {
  const ctx = {args:{acceptanceCriteria:{A:'a', B:'b', C:'c'}, authorMaxParallelism:1, gateMode:'enforce'},
    __archonValidateAcceptanceEntry: () => '[]',
    __archonHost: (kind, payload) => { throw new Error(`${kind}: ${JSON.parse(payload).reason}`); }};
  vm.createContext(withAuthorContext(ctx)); vm.runInContext(source, ctx);
  const authored = [], stdins = [], pauses = [], prompts = [];
  let error;
  try {
    await ctx.authorCandidate({
      agent: async (_, options) => {
        if (authored.length > 30) throw new Error('bounded test exhausted');
        const id = options.task.match(/Author ONLY entry ([^:]+):/)[1];
        authored.push(id);
        prompts.push(options.task);
        return {status:'accepted', stopReason:'end_turn',
          content:JSON.stringify({id, version:authored.filter(x => x === id).length, criterion:'', covers:[]})};
      },
      hostCommand: async (_, {stdin}) => {
        stdins.push(stdin);
        if (stdins.length > gates.length) throw new Error('unexpected freeze');
        return gates[stdins.length - 1]();
      },
      pause: async (_, evidence) => { pauses.push(evidence); },
    }, {phase:'acceptance', prompt:() => 'author', author:ctx.authorAcceptanceEntries,
      capability:'freeze-acceptance', retryScopes:new Set(['candidate_artifact'])});
  } catch (e) { error = e.message; }
  return {authored, stdins, pauses, prompts, error};
}

async function outageAfterFirstPass() {
  const out = await run([outage, clean]);
  assert.equal(out.error, undefined);
  assert.deepEqual(out.authored, ['A', 'B', 'C'], 'the healthy first pass is not re-authored');
  assert.equal(out.stdins.length, 2, 'the freeze is retried');
  assert.equal(out.stdins[1], out.stdins[0], 'with the same candidate');
  assert.equal(out.pauses.length, 0);
}

async function outageAfterPartialRepair() {
  const out = await run([() => refute('B'), outage, clean]);
  assert.equal(out.error, undefined);
  assert.deepEqual(out.authored, ['A', 'B', 'C', 'B'], 'only the refuted entry was re-authored, once');
  assert.equal(out.stdins.length, 3);
  assert.equal(out.stdins[2], out.stdins[1], 'the repaired candidate is frozen again unchanged');
}

async function twoOutagesInARow() {
  const out = await run([outage, outage, clean]);
  assert.equal(out.error, undefined);
  assert.deepEqual(out.authored, ['A', 'B', 'C']);
  assert.equal(out.stdins.length, 3);
  assert.ok(out.stdins.every(stdin => stdin === out.stdins[0]));
  assert.equal(out.pauses.length, 0, 'two outages stay inside the no-progress window');
}

// A persisting outage pauses with the operational reason; the resumed run
// retries the freeze, still without an author call. The first outage credits
// the round that added the missing entries; two further consecutive outages
// fill the independent operational window.
async function persistingOutagePausesThenResumes() {
  const out = await run([outage, outage, outage, outage, clean]);
  assert.equal(out.error, undefined);
  assert.deepEqual(out.authored, ['A', 'B', 'C']);
  assert.equal(out.stdins.length, 5);
  assert.deepEqual(Array.from(out.pauses[0].progress_history, step => step.progress), [true, false, false]);
  assert.equal(out.pauses.length, 1);
  assert.equal(out.pauses[0].reason, 'operational_no_progress');
}

// C4: the contract changed after the staged freeze was prepared (a newer
// per-check repair was published). The host returns the current contract;
// the loop rebuilds its candidate from it -- the newer published entry wins --
// re-authors nothing, and freezes the rebuilt candidate at once.
const stale = (current) => () => ({postcondition:{satisfied:false}, gateEnvelope:{
  report:{gate_report:'prepared', current_acceptance_contract:current},
  policy_findings:[{subject:'acceptance', remediation_scope:'candidate_artifact',
    text:'the acceptance contract changed after this freeze was prepared; refresh the current contract and re-run the acceptance freeze',
    deterministic_defect:{code:'stale_acceptance_contract', subject:'acceptance', provenance:'host'}}]}});
const published = {acceptance:[{id:'B', version:'published', criterion:'b'}], supplementary:[]};

async function staleContractRebuildsFromTheCurrentContract() {
  const out = await run([stale(published), clean]);
  assert.equal(out.error, undefined);
  assert.deepEqual(out.authored, ['A', 'B', 'C'], 'a stale freeze re-authors nothing');
  assert.equal(out.stdins.length, 2, 'the rebuilt candidate is frozen again');
  const [first, retried] = out.stdins.map(stdin => JSON.parse(stdin));
  assert.deepEqual(retried.entries.map(entry => entry.id), ['A', 'B', 'C']);
  assert.equal(retried.entries[1].version, 'published', 'the newer published entry wins');
  assert.deepEqual(retried.entries[0], first.entries[0]);
  assert.deepEqual(retried.entries[2], first.entries[2]);
  assert.equal(out.pauses.length, 0);
}

// A finding of the stale outcome judged the older candidate: it sends back
// only an entry the current contract does not supply.
async function staleContractReauthorsOnlyUnsuppliedFlaggedEntries() {
  const refutedBoth = () => {
    const outcome = stale(published)();
    outcome.gateEnvelope.policy_findings.push(...['B', 'C'].map(id => ({
      text:`check '${id}' was refuted by the host judge; reason: "repair"`, subject:id,
      remediation_scope:'candidate_artifact'})));
    return outcome;
  };
  const out = await run([refutedBoth, clean]);
  assert.equal(out.error, undefined);
  assert.deepEqual(out.authored, ['A', 'B', 'C', 'C'], 'only the unsupplied flagged entry is re-authored');
  assert.equal(JSON.parse(out.stdins[1]).entries[1].version, 'published');
}

// A contract that keeps changing under the freeze counts in the ordinary
// no-progress window: the loop pauses with evidence, never retries forever.
async function persistingStaleContractPausesOnNoProgress() {
  const out = await run([stale(published), stale(published), stale(published), stale(published), clean]);
  assert.equal(out.error, undefined);
  assert.deepEqual(out.authored, ['A', 'B', 'C']);
  assert.equal(out.pauses.length, 1);
  assert.equal(out.pauses[0].reason, 'no_progress');
}

// B5: the stale contract changed elsewhere (B), not the entry this round
// re-authored (A): the round's unpublished repair of A is kept, never the
// refuted bytes it repaired.
const withB = (a) => ({acceptance:[a, {id:'B', version:'published', criterion:'b',
  check:{kind:'command', command:'fixed elsewhere', cwd:'project_root'}}], supplementary:[]});
async function staleContractKeepsTheRoundsUnpublishedRepair() {
  const out = await run([() => refute('A'), stale(withB({id:'A', version:1, criterion:'a'})), clean]);
  assert.equal(out.error, undefined);
  assert.deepEqual(out.authored, ['A', 'B', 'C', 'A'], 'nothing is re-authored after the stale freeze');
  assert.equal(out.stdins.length, 3);
  const rebuilt = JSON.parse(out.stdins[2]);
  assert.equal(rebuilt.entries[0].version, 2, "this round's repair of A is kept");
  assert.equal(rebuilt.entries[1].version, 'published', 'the newer published B wins');
}

// A published entry that changed since the base the round's draft repaired
// wins; the draft is shown to that entry's next author.
async function stalePublishedChangeWinsAndShowsTheDraft() {
  const changed = {id:'A', version:'published', criterion:'a',
    check:{kind:'command', command:'repaired elsewhere', cwd:'project_root'}};
  const out = await run([() => refute('A'), stale(withB(changed)), () => refute('A'), clean]);
  assert.equal(out.error, undefined);
  assert.equal(JSON.parse(out.stdins[2]).entries[0].version, 'published');
  assert.deepEqual(out.authored, ['A', 'B', 'C', 'A', 'A']);
  assert.match(out.prompts[4], /A newer published version of this entry replaced your previous draft/);
  assert.match(out.prompts[4], /"version":2/);
  assert.doesNotMatch(out.prompts[3], /replaced your previous draft/);
}

// H1: a check the host could not prove (an operational finding) beside a
// check its author can repair stops nothing: nothing is published, the
// author repairs its own check, and the next freeze probes the unproven one
// again. Alone, an operational finding still stops the phase.
const unproven = {text:"check 'A': operational: the host could not prove it", subject:'A',
  remediation_scope:'operational'};
const unprovenBeside = id => () => ({postcondition:{satisfied:false}, gateEnvelope:{policy_findings:[
  unproven, {text:`check '${id}': its own run gave no verdict; the check must change`, subject:id,
    remediation_scope:'candidate_artifact'}]}});
async function anUnprovenCheckBesideAnAuthoredFindingStopsNothing() {
  const out = await run([unprovenBeside('B'), clean]);
  assert.equal(out.error, undefined);
  assert.deepEqual(out.authored, ['A', 'B', 'C', 'B'], 'only the authored finding is re-authored');
  assert.equal(out.stdins.length, 2, 'the next freeze probes the unproven check again');
}
// #554: an unproven check alone no longer stops the phase (it used to be a
// terminal stop): the same candidate is frozen again, and the repeat on the
// unchanged entry goes to that entry's author.
async function anUnprovenCheckAloneIsFrozenAgainThenGoesToItsAuthor() {
  const alone = () => ({postcondition:{satisfied:false}, gateEnvelope:{policy_findings:[unproven]}});
  const out = await run([alone, alone, clean]);
  assert.equal(out.error, undefined, 'never a terminal stop');
  assert.equal(out.stdins[1], out.stdins[0], 'the first is frozen again with the same candidate');
  assert.deepEqual(out.authored, ['A', 'B', 'C', 'A'], 'the repeat goes to its own author only');
  assert.match(out.prompts.at(-1), /two freezes of its unchanged text/);
}

// #552: an authored judge failure that still reaches the loop names its
// checks as data; only those entries are re-authored, never every entry.
const judgeFailure = (subjects, text) => () => ({gateEnvelope:{operational_error:{
  kind:'authored_judge_reply', text, ...(subjects ? {subjects} : {})}}});
async function anAuthoredJudgeFailureReauthorsOnlyItsNamedEntries() {
  const out = await run([judgeFailure(['B'], "check 'B': 4 replies; no usable verdict"), clean]);
  assert.equal(out.error, undefined);
  assert.deepEqual(out.authored, ['A', 'B', 'C', 'B'], 'only the named entry is re-authored');
}
// An envelope saved before the field existed keeps the old text routing.
async function aSavedAuthoredJudgeFailureWithoutSubjectsKeepsItsTextRouting() {
  const text = 'freeze incomplete: 1 check(s) still have no judge verdict (C). Retry the freeze';
  const out = await run([judgeFailure(null, text), clean]);
  assert.equal(out.error, undefined);
  assert.deepEqual(out.authored, ['A', 'B', 'C', 'C']);
}

// #551: an entry's prompt lists as current only its findings open in the
// latest gate result; earlier ones are listed as ones not to regress, and a
// finding that reopens is current again.
const finding = (id, text) => () => ({postcondition:{satisfied:false}, gateEnvelope:{policy_findings:[
  {text:`check '${id}': ${text}`, subject:id, remediation_scope:'candidate_artifact'}]}});
const currentOf = prompt => (/open in the latest gate result:\n((?:- [^\n]*\n?)*)/.exec(prompt) || [, ''])[1];
const earlierOf = prompt => (/Do not regress them:\n((?:- [^\n]*\n?)*)/.exec(prompt) || [, ''])[1];
async function anEntryPromptListsOnlyItsLatestGateFindingsAsCurrent() {
  const out = await run([finding('A', 'names a path its site lacks'), finding('A', 'already holds before implementation'),
    finding('A', 'names a path its site lacks'), clean]);
  assert.equal(out.error, undefined);
  assert.deepEqual(out.authored, ['A', 'B', 'C', 'A', 'A', 'A']);
  const [first, second, third] = out.prompts.slice(3);
  assert.match(currentOf(first), /names a path its site lacks/);
  assert.equal(earlierOf(first), '');
  assert.match(currentOf(second), /already holds before implementation/);
  assert.doesNotMatch(currentOf(second), /names a path its site lacks/, 'a resolved finding is not current');
  assert.match(earlierOf(second), /names a path its site lacks/, 'a resolved finding is not to regress');
  assert.match(currentOf(third), /names a path its site lacks/, 'a reopened finding is current again');
  assert.doesNotMatch(currentOf(third), /already holds/);
  assert.match(earlierOf(third), /already holds before implementation/);
  assert.doesNotMatch(earlierOf(third), /names a path its site lacks/);
}

// #552 item 6: a round whose freeze judged nothing keeps the open findings
// of the last gate that did judge as current, and the history of earlier
// ones; its note goes to the entry it names. #555: they are the entry's own,
// in its own section, never a shared repair list.
async function aRoundThatJudgedNothingKeepsTheLastJudgedFindings_555() {
  const out = await run([finding('A', 'requirement R1 is covered by no check'),
    judgeFailure(['B'], "check 'B': 4 replies; no usable verdict"), clean]);
  assert.equal(out.error, undefined);
  assert.deepEqual(out.authored, ['A', 'B', 'C', 'A', 'B']);
  const after = out.prompts[4];
  assert.doesNotMatch(after, /requirement R1 is covered by no check/, "B never reads A's open finding");
  assert.match(after, /check 'B': 4 replies; no usable verdict/, 'the named entry gets the note');
  // The open finding stays current for its own entry when that is re-authored.
  const own = await run([finding('A', 'requirement R1 is covered by no check'),
    judgeFailure(['A'], "check 'A': 4 replies; no usable verdict"), clean]);
  assert.equal(own.error, undefined);
  assert.deepEqual(own.authored, ['A', 'B', 'C', 'A', 'A']);
  assert.match(own.prompts[4], /open in the latest gate result:\n- check 'A': requirement R1 is covered by no check/,
    'the last judged gate\'s open finding stays current');
  assert.match(own.prompts[4], /check 'A': 4 replies; no usable verdict/, 'the named entry gets the note');
}
async function aRungResetAfterARoundThatJudgedNothingKeepsTheHistory_555() {
  const out = await run([finding('A', 'first finding'), finding('A', 'second finding'),
    finding('A', 'third finding'), judgeFailure(['A'], "check 'A': 4 replies; no usable verdict"), clean]);
  assert.equal(out.error, undefined);
  const after = out.prompts.at(-1);
  assert.match(after, /open in the latest gate result:\n- check 'A': third finding/, 'the last judged findings are restored');
  assert.match(after, /Do not regress them:\n(?:- [^\n]*\n)*- check 'A': second finding/, 'the history is kept');
  assert.match(after, /check 'A': 4 replies; no usable verdict/);
}

// #552 item 7: a check the host did not judge (#549: its files are too large;
// #552: the judge could not decide it) is never published, and its finding
// routes by its structured subject to that entry only.
const unpublished = (id, text) => () => ({postcondition:{satisfied:false}, gateEnvelope:{policy_findings:[
  {text, subject:id, remediation_scope:'candidate_artifact',
    source_path:'tasks/acceptance-contract.json'}]}});
async function aNotJudgedFindingReauthorsOnlyItsEntry() {
  for (const text of [
    "for check 'B', the host found 300000 bytes of text source among the words of the check (big 300000 bytes), more than the judge can read (262144 bytes); use only file words whose content decides the criterion",
    "the judge returned no usable verdict for check 'B': 4 replies; the last: no JSON; make the check smaller or split it so that one verdict decides it",
  ]) {
    const out = await run([unpublished('B', text), clean]);
    assert.equal(out.error, undefined);
    assert.deepEqual(out.authored, ['A', 'B', 'C', 'B'], `only the named entry is re-authored: ${text}`);
  }
}

// #552 review: a later stage of the freeze ended incomplete (B's passability
// judge), but C was already known not judged: both re-author, each with its
// own note, never another check's text.
async function aKnownFindingTravelsWithAnIncompleteFreeze() {
  const out = await run([() => ({gateEnvelope:{
    operational_error:{kind:'authored_judge_reply', subjects:['B', 'C'],
      text:"freeze incomplete: 1 check(s) still have no judge verdict on its pre-implementation output (B)"},
    policy_findings:[{subject:'C', remediation_scope:'candidate_artifact',
      text:"the judge returned no usable verdict for check 'C': 4 replies; make the check smaller or split it"}]}}), clean]);
  assert.equal(out.error, undefined);
  assert.deepEqual(out.authored, ['A', 'B', 'C', 'B', 'C']);
  const [b, c] = out.prompts.slice(3);
  assert.match(b, /could not use this entry's last check:\n- freeze judge reply unusable: [^\n]*\(B\)/);
  assert.match(c, /could not use this entry's last check:\n- the judge returned no usable verdict for check 'C'/);
  assert.doesNotMatch(c, /could not use this entry's last check:\n- freeze judge reply unusable/, 'C never gets B\'s note');
}

const tests = [
  ['a known finding travels with an incomplete freeze', aKnownFindingTravelsWithAnIncompleteFreeze],
  ['a not-judged finding re-authors only its entry', aNotJudgedFindingReauthorsOnlyItsEntry],
  ['a round that judged nothing keeps the last judged findings (#555)', aRoundThatJudgedNothingKeepsTheLastJudgedFindings_555],
  ['a rung reset after a round that judged nothing keeps the history (#555)', aRungResetAfterARoundThatJudgedNothingKeepsTheHistory_555],
  ['an authored judge failure re-authors only its named entries', anAuthoredJudgeFailureReauthorsOnlyItsNamedEntries],
  ['a saved authored judge failure without subjects keeps its text routing', aSavedAuthoredJudgeFailureWithoutSubjectsKeepsItsTextRouting],
  ['an entry prompt lists only its latest gate findings as current', anEntryPromptListsOnlyItsLatestGateFindingsAsCurrent],
  ['an unproven check beside an authored finding stops nothing', anUnprovenCheckBesideAnAuthoredFindingStopsNothing],
  ['an unproven check alone is frozen again, then goes to its author', anUnprovenCheckAloneIsFrozenAgainThenGoesToItsAuthor],
  ['a stale freeze keeps the round\'s unpublished repair', staleContractKeepsTheRoundsUnpublishedRepair],
  ['a published change since the draft base wins and shows the draft', stalePublishedChangeWinsAndShowsTheDraft],
  ['a stale freeze rebuilds from the current contract', staleContractRebuildsFromTheCurrentContract],
  ['a stale freeze re-authors only flagged entries it does not supply', staleContractReauthorsOnlyUnsuppliedFlaggedEntries],
  ['a persisting stale contract pauses on no progress', persistingStaleContractPausesOnNoProgress],
  ['outage right after the first pass', outageAfterFirstPass],
  ['outage after a partial repair', outageAfterPartialRepair],
  ['two outages in a row', twoOutagesInARow],
  ['a persisting outage pauses, then resumes without re-authoring', persistingOutagePausesThenResumes],
];
module.exports = tests;
if (require.main === module) (async () => {
  let failed = 0;
  for (const [name, test] of tests) {
    try { await test(); console.log(`PASS ${name}`); }
    catch (e) { failed++; console.error(`FAIL ${name}: ${e.message.slice(0, 600)}`); }
  }
  process.exitCode = failed ? 1 : 0;
})();
