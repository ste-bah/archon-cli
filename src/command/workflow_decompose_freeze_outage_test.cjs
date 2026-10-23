// Issue 362: a clean author round leaves nothing to re-author. A freeze that
// fails operationally (no findings) is retried with the same entries and no
// author call; a persisting outage pauses (resumable), never re-authors.
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');
const clean = () => ({publicationReceipt:{call_id:'freeze'}, postcondition:{satisfied:true},
  gateEnvelope:{policy_findings:[]}});
const outage = () => ({gateEnvelope:{operational_error:{text:'freeze backend unavailable'}}});
const refute = id => ({...clean(), gateEnvelope:{policy_findings:[{
  text:`check '${id}' was refuted: repair`, subject:id, remediation_scope:'candidate_artifact'}]}});

// `gates` is the freeze outcome sequence. A pause is resumed (returns).
async function run(gates) {
  const ctx = {args:{acceptanceCriteria:{A:'a', B:'b', C:'c'}, authorMaxParallelism:1, gateMode:'enforce'},
    __archonValidateAcceptanceEntry: () => '[]'};
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
          content:JSON.stringify({id, version:authored.filter(x => x === id).length, criterion:''})};
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

const tests = [
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
