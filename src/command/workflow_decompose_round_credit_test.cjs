// Issue 357 round 7: an entry that beats its own best is progress in every
// kind of failed round (a sibling refused, unparseable or never answered);
// each entry's note stays in its own slot and the shared repair list stays
// the gate's findings; observe pauses on a window that holds an outage.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_progress.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');
const fields = ['check/command', 'check/cwd', 'check/kind'];
const shape = (id, field) => ({text:`acceptance entry '${id}' was refused: ${field} invalid`,
  deterministic_defect:{provenance:'host_validator', code:'invalid_candidate_shape',
    subject:`entries/0/${field}`, location:'shape', stage:'shape'}});
const clean = () => ({publicationReceipt:{call_id:'freeze'}, postcondition:{satisfied:true},
  gateEnvelope:{policy_findings:[]}});
const outage = () => ({gateEnvelope:{operational_error:{text:'judge down'}}});
const refute = ids => () => ({...clean(), gateEnvelope:{policy_findings:ids.map(id => ({
  text:`check '${id}' was refuted: repair ${id}`, subject:id, remediation_scope:'candidate_artifact'}))}});

// script[id][version - 1]: 'ok', a shape defect count, 'bad' (unparseable)
// or 'down' (the call fails in transport); the last value repeats.
async function run(script, gates, {cap = 2, mode = 'enforce'} = {}) {
  const ids = Object.keys(script);
  const at = (id, version) => script[id][Math.min(version, script[id].length) - 1];
  const ctx = {args:{acceptanceCriteria:Object.fromEntries(ids.map(id => [id, id])), authorMaxParallelism:cap, gateMode:mode},
    __archonValidateAcceptanceEntry: (id, serialized) => {
      const value = at(id, JSON.parse(serialized).version);
      return JSON.stringify(typeof value === 'number' ? fields.slice(0, value).map(field => shape(id, field)) : []);
    }};
  vm.createContext(ctx); vm.runInContext(source, ctx);
  let calls = 0, gate = 0, error, result;
  const versions = new Map(), pauses = [], prompts = [];
  try {
    result = await ctx.authorCandidate({
      agent: async (_, options) => {
        if (++calls > 60) throw new Error('bounded test exhausted');
        const id = options.task.match(/Author ONLY entry ([^:]+):/)[1];
        const version = (versions.get(id) || 0) + 1; versions.set(id, version);
        prompts.push({id, version, task:options.task});
        const value = at(id, version);
        if (value === 'down') return {status:'failed', summary:'transport'};
        if (value === 'bad') return {status:'accepted', stopReason:'end_turn', content:'not json'};
        return {status:'accepted', stopReason:'end_turn', content:JSON.stringify({id, version, criterion:''})};
      },
      hostCommand: async () => {
        if (++gate > 40) throw new Error('gate exhausted');
        return gates[Math.min(gate, gates.length) - 1]();
      },
      pause: async (_, evidence) => { pauses.push(evidence); throw new Error('paused'); },
    }, {phase:'acceptance', prompt:() => 'author', author:ctx.authorAcceptanceEntries,
      capability:'freeze-acceptance', retryScopes:new Set(['candidate_artifact'])});
  } catch (e) { error = e.message; }
  const history = pauses[0] ? Array.from(pauses[0].progress_history) : [];
  const prompt = (id, version) => (prompts.find(p => p.id === id && p.version === version) || {}).task || '';
  return {calls, gate, error, result, pauses, prompts, prompt, history, flags:history.map(step => step.progress)};
}

const credited = step => Array.from(step.entries || [], e => [e.subject, e.progress]);

// The judge refutes A and B; both repairs are refused at one defect; then A
// passes while B fails in the given way, and B then stays at one defect.
async function passBeside(kind) {
  const out = await run({A:['ok', 1, 'ok'], B:['ok', 1, kind, 1]}, [refute(['A', 'B']), clean]);
  assert.equal(out.error, 'paused', 'B stuck alone still pauses');
  assert.equal(out.calls, 9, 'judged round, A+B twice, then 3 stalled rounds of B: A passing is progress');
  assert.deepEqual(out.flags, [true, true, true, false, false, false]);
  assert.ok(credited(out.history[2]).some(([id, p]) => id === 'A' && p), `A's pass is credited: ${JSON.stringify(out.history[2])}`);
  return out;
}
const passBesideUnparseable = () => passBeside('bad');
const passBesideTransport = () => passBeside('down');
const passBesideRefusal = () => passBeside(1);

// A credited round restores the window to the episode floor, never below it:
// the second refutation repeats the first (floor 1), so after A's credit
// only two more stalled rounds of B fit before the pause.
async function creditStopsAtTheFloor() {
  const out = await run({A:['ok', 'ok', 1, 'ok'], B:['ok', 'ok', 1, 'bad', 1]}, [refute(['A', 'B']), refute(['A', 'B']), clean]);
  assert.equal(out.error, 'paused');
  assert.deepEqual(out.flags, [true, false, true, true, false, false]);
  assert.equal(out.calls, 10, 'two judged rounds, A+B twice, then 2 stalled rounds of B');
}

// A pass is terminal: an entry is credited once. A passes beside B's
// unparseable reply (progress); B then stays unparseable and A, no longer
// pending, is not credited again.
async function passCreditedOnce() {
  const out = await run({A:['ok', 1, 'ok'], B:['ok', 1, 'bad']}, [refute(['A', 'B']), clean]);
  assert.equal(out.error, 'paused');
  assert.deepEqual(out.flags, [true, true, true, false, false, false]);
  assert.deepEqual(credited(out.history[2]), [['A', true]]);
  for (const step of out.history.slice(3)) assert.equal(step.entries, undefined, 'A is not credited again');
}

// Issue 261 kept: a rewrite of a judge-refuted entry that passes the author
// step without any measured refusal in the episode is novelty, not progress,
// whatever the sibling's failure.
async function unmeasuredRewriteIsNotCredited(kind) {
  const out = await run({A:['ok'], B:['ok', kind]}, [refute(['A', 'B']), clean]);
  assert.equal(out.error, 'paused');
  assert.deepEqual(out.flags, [true, false, false, false]);
  assert.equal(out.history[1].entries, undefined, 'A was never measured in the episode');
}

// B's unparseable reply is B's own note: C (whose call failed in the same
// round) keeps the gate's findings as its repair list and never reads B's note.
async function malformedNoteIsPerEntry() {
  const out = await run({B:['ok', 'bad', 'ok'], C:['ok', 'down', 'ok']}, [refute(['B', 'C']), clean]);
  assert.equal(out.error, undefined, JSON.stringify(out.pauses));
  const c3 = out.prompt('C', 3), b3 = out.prompt('B', 3);
  assert.doesNotMatch(c3, /acceptance entry B returned no complete entry/, 'C reads no sibling note');
  const gate = /Repair these exact authoritative findings:\n- check 'B' was refuted: repair B\n- check 'C' was refuted: repair C/;
  assert.match(c3, gate, 'the repair list is still the gate findings');
  assert.match(b3, /refused this entry's previous reply[\s\S]*acceptance entry B returned no complete entry/, 'B reads its own note');
  assert.match(b3, gate, 'B still reads the gate findings');
}

// B's shape refusal is replaced by its later note of another kind: an
// unparseable reply shows its own note, a transport failure clears it.
async function staleRefusalCleared(kind) {
  const out = await run({B:['ok', 1, kind, 'ok']}, [refute(['B']), clean], {cap:1});
  assert.equal(out.error, undefined, JSON.stringify(out.pauses));
  assert.match(out.prompt('B', 3), /check\/command invalid/, 'the refusal reaches the next reply');
  assert.doesNotMatch(out.prompt('B', 4), /check\/command invalid/, 'the stale refusal is gone');
  if (kind === 'bad') assert.match(out.prompt('B', 4), /acceptance entry B returned no complete entry/);
  else assert.doesNotMatch(out.prompt('B', 4), /refused this entry's previous reply/);
}

// Observe: a window that ends on outages after a judged repeat pauses
// (resumable); it never returns the commit the outages left unjudged.
async function observeOutagePauses() {
  const out = await run({A:['ok']}, [refute(['A']), refute(['A']), outage, outage, clean], {mode:'observe'});
  assert.equal(out.error, 'paused', `observe must pause, got ${JSON.stringify(out.result)}`);
  assert.deepEqual(out.history.map(step => step.kind), ['judged', 'judged', 'operational', 'operational']);
}
// One outage inside a judged window is enough: the window is not measured.
async function observeOneOutagePauses() {
  const out = await run({A:['ok']}, [refute(['A']), refute(['A']), outage, refute(['A']), clean], {mode:'observe'});
  assert.equal(out.error, 'paused', `observe must pause, got ${JSON.stringify(out.result)}`);
}
// Control: judged repeats alone still return observe's latest commit.
async function observeJudgedReturns() {
  const out = await run({A:['ok']}, [refute(['A'])], {mode:'observe'});
  assert.equal(out.error, undefined, JSON.stringify(out.pauses));
  assert.equal(out.result.publicationReceipt.call_id, 'freeze');
  assert.equal(out.gate, 4, 'first refutation, then 3 judged repeats');
}

const tests = [
  ['A passes while B is unparseable: progress', passBesideUnparseable],
  ['A passes while B is down in transport: progress', passBesideTransport],
  ['A passes while B is refused (control): progress', passBesideRefusal],
  ['a credited round restores the window to the episode floor', creditStopsAtTheFloor],
  ['a pass is credited once', passCreditedOnce],
  ['an unmeasured rewrite beside an unparseable reply is not credited', () => unmeasuredRewriteIsNotCredited('bad')],
  ['an unmeasured rewrite beside a transport failure is not credited', () => unmeasuredRewriteIsNotCredited('down')],
  ['an unparseable reply is noted for its own entry only', malformedNoteIsPerEntry],
  ['a refusal is replaced by an unparseable note', () => staleRefusalCleared('bad')],
  ['a refusal is cleared by a transport failure', () => staleRefusalCleared('down')],
  ['observe pauses on outages after a judged repeat', observeOutagePauses],
  ['observe pauses on one outage inside a judged window', observeOneOutagePauses],
  ['observe returns the latest commit on judged repeats (control)', observeJudgedReturns],
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
