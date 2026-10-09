// Issue 362: a clean author round leaves nothing to re-author. A freeze that
// fails operationally (no findings) is retried with the same entries and no
// author call; a persisting outage pauses (resumable), never re-authors.
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_v1_acceptance.js',
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
  const authored = [], stdins = [], pauses = [];
  let error;
  try {
    await ctx.authorCandidate({
      agent: async (_, options) => {
        if (authored.length > 30) throw new Error('bounded test exhausted');
        const id = options.task.match(/Author ONLY entry ([^:]+):/)[1];
        authored.push(id);
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
  return {authored, stdins, pauses, error};
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

const tests = [
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
