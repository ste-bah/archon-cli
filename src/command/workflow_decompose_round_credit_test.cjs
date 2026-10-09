// Issue 357 round 7: an entry that beats its own best is progress in every
// kind of failed round (a sibling refused, unparseable or never answered);
// each entry's note stays in its own slot and the shared repair list stays
// the gate's findings; observe pauses on a window that holds an outage, and
// (Issue 288) on every other stalled window too.
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js']
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
// The judge names an owed supplementary entry for a PRD requirement.
const owe = req => () => ({gateEnvelope:{policy_findings:[{remediation_scope:'candidate_artifact', subject:'SUP',
  text:`check 'SUP-${req}': PRD requirement ${req} is covered by no acceptance check; author it: must hold ${req}`}]}});

// script[id][version - 1]: 'ok', a shape defect count, 'bad' (unparseable)
// or 'down' (the call fails in transport); the last value repeats. A
// function script[id](version, task) decides from the prompt it is shown.
async function run(script, gates, {cap = 2, mode = 'enforce'} = {}) {
  const ids = Object.keys(script);
  const decided = new Map();
  const at = (id, version, task) => typeof script[id] === 'function' ? script[id](version, task)
    : !script[id] ? 'ok' : script[id][Math.min(version, script[id].length) - 1];
  const ctx = {args:{acceptanceCriteria:Object.fromEntries(ids.map(id => [id, id])), authorMaxParallelism:cap, gateMode:mode},
    __archonValidateAcceptanceEntry: (id, serialized) => {
      const value = decided.get(`${id}@${JSON.parse(serialized).version}`);
      return JSON.stringify(typeof value === 'number' ? fields.slice(0, value).map(field => shape(id, field)) : []);
    }};
  vm.createContext(withAuthorContext(ctx)); vm.runInContext(source, ctx);
  let calls = 0, gate = 0, error, result;
  const versions = new Map(), pauses = [], prompts = [];
  try {
    result = await ctx.authorCandidate({
      agent: async (_, options) => {
        if (++calls > 60) throw new Error('bounded test exhausted');
        const id = options.task.match(/Author ONLY entry ([^:]+):/)[1];
        const version = (versions.get(id) || 0) + 1; versions.set(id, version);
        prompts.push({id, version, task:options.task});
        const value = at(id, version, options.task);
        decided.set(`${id}@${version}`, value);
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
  assert.deepEqual(credited(out.history[2]), [['B', false], ['A', true]]);
  for (const step of out.history.slice(3)) {
    assert.ok(!(step.entries || []).some(entry => entry.subject === 'A' && entry.progress), 'A is not credited again');
  }
}

// Issue 261 kept: a rewrite of a judge-refuted entry that passes the author
// step without any measured refusal in the episode is novelty, not progress,
// whatever the sibling's failure.
async function unmeasuredRewriteIsNotCredited(kind) {
  const out = await run({A:['ok'], B:['ok', kind]}, [refute(['A', 'B']), clean]);
  assert.equal(out.error, 'paused');
  const expected = kind === 'bad' ? [true, true, false, false, false] : [true, false, false, false];
  assert.deepEqual(out.flags, expected);
  if (kind === 'bad') assert.ok(credited(out.history[1]).some(([id, improved]) => id === 'B' && improved),
    'a new malformed-reply class is progress once for its entry');
  else assert.equal(out.history[1].entries, undefined, 'a rewrite beside transport failure is not measured');
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
  assert.match(b3, /refused this entry's last answered reply[\s\S]*acceptance entry B returned no complete entry/, 'B reads its own note');
  assert.match(b3, gate, 'B still reads the gate findings');
}

// An unparseable reply is an answered reply: its note replaces B's shape
// refusal. A call never answered leaves the refusal of B's last answered
// reply in place (review r357g F1).
async function refusalAfterOtherFailure(kind) {
  const out = await run({B:['ok', 1, kind, 'ok']}, [refute(['B']), clean], {cap:1});
  assert.equal(out.error, undefined, JSON.stringify(out.pauses));
  assert.match(out.prompt('B', 3), /check\/command invalid/, 'the refusal reaches the next reply');
  if (kind === 'bad') {
    assert.doesNotMatch(out.prompt('B', 4), /check\/command invalid/, 'the replaced refusal is gone');
    assert.match(out.prompt('B', 4), /acceptance entry B returned no complete entry/);
  } else {
    assert.match(out.prompt('B', 4), /refused this entry's last answered reply[\s\S]*check\/command invalid/, 'the refusal survives an outage');
  }
}

// Review r357g F1 probe: the author repairs the defect only when its prompt
// shows it. A transport failure between the refusal and the repair must not
// hide it, so the repair lands and the run completes.
async function refusalSurvivesOutage() {
  const reply = (v, task) => v <= 2 ? 'ok' : v === 3 ? 1 : v === 4 ? 'down' : (/check\/command invalid/.test(task) ? 'ok' : 1);
  const out = await run({A:reply}, [refute(['A']), refute(['A']), clean]);
  assert.equal(out.error, undefined, JSON.stringify(out.pauses));
  assert.equal(out.gate, 3);
}

// Review r357g F2 probe: A is repaired in a clean round and the freeze then
// has two outages. A's pass is progress whatever the gate does next, so the
// window is not spent and the third freeze completes.
async function cleanPassBeforeOutages() {
  const out = await run({A:['ok', 'ok', 1, 'ok']}, [refute(['A']), refute(['A']), outage, outage, clean]);
  assert.equal(out.error, undefined, JSON.stringify(out.pauses));
  assert.equal(out.gate, 5);
}
// F2 control: the same pass beside a sibling's transport failure.
async function cleanPassControl() {
  const out = await run({A:['ok', 'ok', 1, 'ok'], Z:['ok', 'ok', 'ok', 'down', 'ok']},
    [refute(['A', 'Z']), refute(['A', 'Z']), outage, clean]);
  assert.equal(out.error, undefined, JSON.stringify(out.pauses));
}
// The judged path credits a clean pass too, but a judged repeat still adds
// one to the next floor: A's pass forgives its stalled refusal (window 1 -> 0)
// and the refutation then counts (0 -> 1), so A gets one more stalled round.
async function cleanPassBeforeJudgedRepeat() {
  const out = await run({A:['ok', 1, 1, 'ok', 1]}, [refute(['A'])]);
  assert.equal(out.error, 'paused');
  assert.equal(out.calls, 7, 'judged, 1, 1, pass+judged, then 1 x3 from floor 1');
  assert.deepEqual(credited(out.history[3]), [['A', true]]);
  assert.equal(out.history[3].progress, false, 'the judged repeat is no progress');
}
// Bound: a judge that refutes every candidate, with a shape repair between,
// still pauses: each judged repeat raises the next floor.
async function endlessRefutationPauses() {
  const out = await run({A:(v) => v % 2 ? 'ok' : 1}, [refute(['A'])]);
  assert.equal(out.error, 'paused');
  assert.equal(out.calls, 7);
}

// Review r357g F3: an outage in a round whose first failure is a refusal or
// an unparseable reply is still an outage of the window, so observe pauses.
async function hiddenOutagePauses(kind) {
  const out = await run({A:['ok', 1, kind, 1], B:['ok', 1, 'down', 1]}, [refute(['A', 'B'])], {mode:'observe'});
  assert.equal(out.error, 'paused', `observe must pause, got ${JSON.stringify(out.result)}`);
  assert.equal(out.history[2].outage, true, JSON.stringify(out.history[2]));
}
// F3 control: the same window without the transport failure also pauses
// (Issue 288: a stall never returns an artifact), but as a measured stall.
async function noOutagePausesToo() {
  const out = await run({A:['ok', 1], B:['ok', 1]}, [refute(['A', 'B'])], {mode:'observe'});
  assert.equal(out.error, 'paused', `observe must pause, got ${JSON.stringify(out.result)}`);
  assert.equal(out.pauses.length, 1);
  assert.equal(out.pauses[0].reason, 'no_progress', JSON.stringify(out.pauses[0]));
  assert.notEqual(out.history[2].outage, true, JSON.stringify(out.history[2]));
}

// Observe: a window that ends on outages after a judged repeat pauses
// (resumable); it never returns the commit the outages left unjudged.
async function oneOperationalWindowDoesNotBorrowJudgedAttempts() {
  const out = await run({A:['ok']}, [refute(['A']), refute(['A']), outage, outage, clean], {mode:'observe'});
  assert.equal(out.error, undefined, `two operational rounds fit their own window: ${JSON.stringify(out.pauses)}`);
  assert.deepEqual(out.pauses, []);
}
// A successful gate resets operational attempts; author findings before it do not.
async function successfulGateResetsOperationalWindow() {
  const out = await run({A:['ok']}, [refute(['A']), refute(['A']), outage, refute(['A']), clean], {mode:'observe'});
  assert.equal(out.error, undefined, `the refutation was judged successfully and reset the window: ${JSON.stringify(out.pauses)}`);
  assert.deepEqual(out.pauses, []);
}
// Control: judged repeats alone pause observe too (Issue 288), with the
// open refutation as evidence; the committed artifact is not returned.
async function observeJudgedRepeatsPause() {
  const out = await run({A:['ok']}, [refute(['A'])], {mode:'observe'});
  assert.equal(out.error, 'paused', `observe must pause, got ${JSON.stringify(out.result)}`);
  assert.equal(out.gate, 4, 'first refutation, then 3 judged repeats');
  assert.deepEqual(out.history.map(step => step.kind), ['judged', 'judged', 'judged', 'judged']);
  assert.ok(Array.from(out.pauses[0].last_findings).some(text => text.includes("check 'A' was refuted")),
    JSON.stringify(out.pauses[0].last_findings));
}

// N1: an owed entry completed in a clean round is progress even when the
// freeze then has an outage (the round added a previously missing entry).
async function newEntryBeforeOutages() {
  const out = await run({A:['ok']}, [refute(['A']), owe('REQ-1'), outage, outage, clean]);
  assert.equal(out.error, undefined, `must return, got ${JSON.stringify(out.history)}`);
  assert.equal(out.gate, 5);
  assert.deepEqual(out.history, [], 'no pause');
}
// A round that adds no entry and passes nothing is not progress at an outage.
async function noNewEntryOutageIsNotProgress() {
  const out = await run({A:['ok']}, [refute(['A']), refute(['A']), outage, outage, outage, clean]);
  assert.equal(out.error, 'paused');
  assert.deepEqual(out.history.map(step => step.kind), ['judged', 'judged', 'operational', 'operational', 'operational']);
  assert.deepEqual(out.flags, [true, false, false, false, false]);
}
// The added entry is credited at the first outage only; later outages stall.
async function newEntryCreditedOnce() {
  const out = await run({A:['ok']}, [refute(['A']), owe('REQ-1'), outage, outage, outage, outage, clean]);
  assert.equal(out.error, 'paused', 'bounded: repeated outages after one credit pause');
  assert.deepEqual(out.history.map(step => step.kind),
    ['judged', 'judged', 'operational', 'operational', 'operational']);
  assert.deepEqual(out.flags, [true, false, true, false, false]);
}

// Gate and provider failures use their own consecutive window. They remain
// in progress_history as operational rounds and never spend author attempts.
async function operationalRoundsUseOwnWindow() {
  const out = await run({A:['ok']}, [outage, outage, outage]);
  assert.equal(out.error, 'paused');
  assert.equal(out.pauses[0].reason, 'operational_no_progress');
  assert.deepEqual(out.history.map(step => step.kind), ['operational', 'operational', 'operational']);
  assert.match(out.pauses[0].recovery, /host gate/);
}

async function operationalRoundDoesNotSpendAuthorWindow() {
  const out = await run({A:['down', 1, 1, 1, 1, 1]}, [clean]);
  assert.equal(out.error, 'paused');
  assert.equal(out.pauses[0].reason, 'no_progress', JSON.stringify(out.pauses[0]));
  assert.deepEqual(out.history.map(step => step.kind), ['operational', 'refused', 'refused', 'refused', 'refused']);
}

async function authorProgressDoesNotResetOperationalWindow() {
  const out = await run({A:['down', 1, 'ok', 'ok']}, [outage, outage, outage]);
  assert.equal(out.error, 'paused');
  assert.equal(out.pauses[0].reason, 'operational_no_progress');
  assert.deepEqual(out.history.map(step => step.kind), ['operational', 'refused', 'operational', 'operational']);
  assert.deepEqual(out.flags, [false, true, true, false]);
}

async function twoAuthorRefusalsAfterProviderFailureDoNotPause() {
  const out = await run({A:['down', 1, 1, 'ok']}, [clean]);
  assert.equal(out.error, undefined, JSON.stringify(out.pauses));
  assert.deepEqual(out.pauses, []);
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
  ['a refusal is replaced by an unparseable note', () => refusalAfterOtherFailure('bad')],
  ['a refusal survives a transport failure', () => refusalAfterOtherFailure('down')],
  ['F1 probe: a repair after an outage still reads its refusal', refusalSurvivesOutage],
  ['F2 probe: a clean pass before two freeze outages is progress', cleanPassBeforeOutages],
  ['F2 control: the pass beside a sibling outage', cleanPassControl],
  ['a clean pass before a judged repeat: credited, the repeat still counts', cleanPassBeforeJudgedRepeat],
  ['an endless refutation with repairs between pauses', endlessRefutationPauses],
  ['F3: an outage behind a refusal pauses observe', () => hiddenOutagePauses(1)],
  ['F3: an outage behind an unparseable reply pauses observe', () => hiddenOutagePauses('bad')],
  ['F3 control: the window without an outage pauses as a judged stall', noOutagePausesToo],
  ['two operational rounds do not borrow judged attempts', oneOperationalWindowDoesNotBorrowJudgedAttempts],
  ['a successful gate resets operational attempts', successfulGateResetsOperationalWindow],
  ['observe pauses on judged repeats (control)', observeJudgedRepeatsPause],
  ['N1 probe: a new entry in a clean round before two outages is progress', newEntryBeforeOutages],
  ['N1 edge: a round with no new entry before an outage is not progress', noNewEntryOutageIsNotProgress],
  ['N1 edge: a new entry before outages is credited once', newEntryCreditedOnce],
  ['operational rounds fill their own window and identify the gate', operationalRoundsUseOwnWindow],
  ['an operational round does not spend the author window', operationalRoundDoesNotSpendAuthorWindow],
  ['author progress does not reset operational attempts before a successful gate', authorProgressDoesNotResetOperationalWindow],
  ['one provider failure and two author refusals do not pause', twoAuthorRefusalsAfterProviderFailureDoNotPause],
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
