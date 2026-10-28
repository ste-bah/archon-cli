// Issue 357 rounds 5-6: a round that refuses several entries measures each
// one against its own best and feeds each one its own refusal. Progress is at
// least one entry beating its own best (a first pass counts); an entry worse
// than its best does not cancel it. Bests only improve over a finite measure,
// so the loop stays bounded. The two-entry cases are the round-4 review probe.
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js', 'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');
const fields = ['check/command', 'check/cwd', 'check/kind', 'gap_permitted', 'covers/0'];
const shape = (id, field) => ({text:`acceptance entry '${id}' was refused: ${field} invalid`,
  deterministic_defect:{provenance:'host_validator', code:'invalid_candidate_shape',
    subject:`entries/0/${field}`, location:'shape', stage:'shape'}});
const clean = () => ({publicationReceipt:{call_id:'freeze'}, postcondition:{satisfied:true},
  gateEnvelope:{policy_findings:[]}});

// Version 1 of every entry is shape-valid and the judge refutes all of them;
// `seq[id](n)` is the number of shape defects of that entry's repair n.
async function run(seq, cap) {
  const ids = Object.keys(seq);
  const ctx = {args:{acceptanceCriteria:Object.fromEntries(ids.map(id => [id, id])), authorMaxParallelism:cap, gateMode:'enforce'},
    __archonValidateAcceptanceEntry: (id, serialized) => {
      const entry = JSON.parse(serialized);
      const n = entry.version === 1 || entry.version > 13 ? 0 : seq[id](entry.version - 1);
      return JSON.stringify(fields.slice(0, n).map(field => shape(id, field)));
    }};
  vm.createContext(withAuthorContext(ctx)); vm.runInContext(source, ctx);
  let calls = 0, gates = 0, error;
  const versions = new Map(), pauses = [], prompts = [];
  try {
    await ctx.authorCandidate({ checkpoint: async () => {},
      agent: async (_, options) => {
        if (++calls > 60) throw new Error('bounded test exhausted');
        const id = options.task.match(/Author ONLY entry ([^:]+):/)[1];
        const version = (versions.get(id) || 0) + 1; versions.set(id, version);
        prompts.push({id, version, task:options.task});
        return {status:'accepted', stopReason:'end_turn', content:JSON.stringify({id, version, criterion:''})};
      },
      hostCommand: async () => {
        if (++gates > 1) return clean();
        return {...clean(), gateEnvelope:{policy_findings:ids.map(id => ({
          text:`check '${id}' was refuted: repair`, subject:id, remediation_scope:'candidate_artifact'}))}};
      },
      pause: async (_, evidence) => { pauses.push(evidence); throw new Error('paused'); },
    }, {phase:'acceptance', prompt:() => 'author', author:ctx.authorAcceptanceEntries,
      capability:'freeze-acceptance', retryScopes:new Set(['candidate_artifact'])});
  } catch (e) { error = e.message; }
  const history = pauses[0] ? Array.from(pauses[0].progress_history) : [];
  return {calls, gates, error, pauses, prompts, history, flags:history.map(step => step.progress)};
}

// Every repair prompt holds its own entry's refusal and no sibling's.
function ownRefusalsOnly(out, ids) {
  for (const {id, version, task} of out.prompts.filter(p => p.version > 2)) {
    assert.match(task, new RegExp(`acceptance entry '${id}' was refused`), `${id} v${version} reads its own refusal`);
    for (const other of ids.filter(other => other !== id)) {
      assert.doesNotMatch(task, new RegExp(`acceptance entry '${other}' was refused`), `${id} v${version} reads ${other}'s refusal`);
    }
  }
}

// Probe scenario 1: A stays at 1 defect while B shrinks 3 -> 2 -> 1 -> 0.
// A then receives the remaining repair strategies until it clears.
async function stuckBesideShrinking() {
  const out = await run({A:() => 1, B:n => Math.max(4 - n, 0)}, 2);
  assert.equal(out.error, undefined, 'A continues through the ladder');
  assert.equal(out.pauses.length, 0);
  assert.ok(out.calls > 13);
  ownRefusalsOnly(out, ['A', 'B']);
}

// Probe scenario 2: in the round A is fixed, B regresses 1 -> 4. That round
// is progress exactly because A beat its best (it passed); B's regression is
// recorded as worse and credits nothing. The rounds after it are B alone at 4.
async function regressionBesideFix() {
  const out = await run({A:n => n >= 2 ? 0 : 1, B:n => n >= 2 ? 4 : 1}, 2);
  assert.equal(out.error, undefined);
  assert.equal(out.pauses.length, 0);
  ownRefusalsOnly(out, ['A', 'B']);
}

// The same regression with A stuck at its best: nothing beat a best, so the
// round where B goes 1 -> 4 is no progress.
async function regressionBesideStuck() {
  const out = await run({A:() => 1, B:n => n >= 2 ? 4 : 1}, 2);
  assert.equal(out.error, undefined);
  assert.equal(out.pauses.length, 0);
}

// Three entries in one window: A 2 -> 1 -> 0, B 1 -> 0, C 1 -> 2 -> 2. C getting
// worse cancels nothing: the rounds where A or B beat their bests are
// progress; C alone then receives a fresh repair strategy.
async function threeEntries() {
  const out = await run({A:n => Math.max(3 - n, 0), B:n => n >= 2 ? 0 : 1, C:n => n >= 2 ? 2 : 1}, 3);
  assert.equal(out.error, undefined);
  assert.equal(out.pauses.length, 0);
  ownRefusalsOnly(out, ['A', 'B', 'C']);
}

// Bound: two entries trading defects forever (A 1,2,1,2... and B 2,1,2,1...)
// make progress only while a best still improves. Once both bests are 1 no
// round beats a best, so the loop advances repair strategies.
async function oscillatingPairAdvancesTheLadder() {
  const out = await run({A:n => n % 2 ? 1 : 2, B:n => n % 2 ? 2 : 1}, 2);
  assert.equal(out.error, undefined, 'an oscillating pair advances strategy until clean');
  assert.equal(out.pauses.length, 0);
}

const tests = [
  ['stuck entry beside a shrinking one: ladder continues', stuckBesideShrinking],
  ['regression beside a fix: progress only because A beat its best', regressionBesideFix],
  ['regression beside a stuck entry is not progress', regressionBesideStuck],
  ['three entries: a worse entry cancels no progress', threeEntries],
  ['an oscillating pair advances the ladder', oscillatingPairAdvancesTheLadder],
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
