// Issue 357 round 7: an entry that beats its own best is progress in every
// kind of failed round (a sibling refused, unparseable or never answered);
// each entry's note stays in its own slot and the shared repair list stays
// the gate's findings; stalled authored windows advance the ladder.
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js', 'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js']
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
  const logs = [], rungs = [], rounds = [];
  const ctx = {args:{acceptanceCriteria:Object.fromEntries(ids.map(id => [id, id])), prdRequirementTexts:{'REQ-1':'must hold REQ-1'}, authorMaxParallelism:cap, gateMode:mode},
    console:{log: line => {
      logs.push(String(line));
      rungs.push({line:String(line), calls, gate, trace:JSON.parse(JSON.stringify(ctx.progressTrace))});
    }},
    __archonHost: async (_, raw) => { throw new Error(JSON.parse(raw).reason); },
    JSON: Object.assign(Object.create(JSON), {parse(text, ...args) {
      if (text === 'opaque parser fixture') throw new SyntaxError('new provider parser wording');
      return JSON.parse(text, ...args);
    }}),
    __archonValidateAcceptanceEntry: (id, serialized) => {
      const value = decided.get(`${id}@${JSON.parse(serialized).version}`);
      return JSON.stringify(typeof value === 'number' ? fields.slice(0, value).map(field => shape(id, field)) : []);
    }};
  vm.createContext(withAuthorContext(ctx)); vm.runInContext(source, ctx);
  vm.runInContext(`
    globalThis.progressTrace = [];
    for (const name of ['recordRepairs', 'recordAnswered', 'recordOperational', 'recordAttempt']) {
      const original = globalThis[name];
      globalThis[name] = (progress, ...args) => {
        const result = original(progress, ...args);
        progressTrace.push(JSON.parse(JSON.stringify({entry:progress.history.at(-1),
          stalled:progress.stalled, floor:progress.repair.stalled})));
        return result;
      };
    }
  `, ctx);
  const originalRoundReport = ctx.authorRoundReport;
  let previousAdded = 0;
  ctx.authorRoundReport = (policy, state) => {
    const report = originalRoundReport(policy, state);
    rounds.push(JSON.parse(JSON.stringify({...report, added:(state.added || 0) - previousAdded})));
    previousAdded = state.added || 0;
    return report;
  };
  let calls = 0, gate = 0, error, result;
  const versions = new Map(), pauses = [], prompts = [];
  try {
    result = await ctx.authorCandidate({ checkpoint: async (_id, options) => { const line = options.task; logs.push(line); rungs.push({line, calls, gate, trace:JSON.parse(JSON.stringify(ctx.progressTrace))}); },
      agent: async (_, options) => {
        if (++calls > 60) throw new Error('bounded test exhausted');
        const id = options.task.match(/Author ONLY entry ([^:]+):/)[1];
        const version = (versions.get(id) || 0) + 1; versions.set(id, version);
        prompts.push({id, version, task:options.task});
        const value = version >= 13 ? 'ok' : at(id, version, options.task);
        decided.set(`${id}@${version}`, value);
        if (value === 'down') return {status:'failed', summary:'transport'};
        if (value === 'bad') return {status:'accepted', stopReason:'end_turn', content:'not json'};
        if (value === 'badUnknown') return {status:'accepted', stopReason:'end_turn', content:'opaque parser fixture'};
        if (value === 'stopUnknown') return {status:'accepted', stopReason:version % 2 ? 'provider_stop_a' : 'provider_stop_b', content:JSON.stringify({id, version})};
        if (value === 'bad2') return {status:'accepted', stopReason:'end_turn', content:'{"id":'};
        if (value === 'noend') return {status:'accepted', stopReason:'max_tokens', content:JSON.stringify({id, version, criterion:''})};
        return {status:'accepted', stopReason:'end_turn', content:JSON.stringify({id:value === 'wrong' ? 'OTHER' : id, version, criterion:''})};
      },
      hostCommand: async () => {
        if (++gate > 9) return clean();
        return gates[Math.min(gate, gates.length) - 1]();
      },
      pause: async (_, evidence) => { pauses.push(evidence); throw new Error('paused'); },
    }, {phase:'acceptance', prompt:() => 'author', author:ctx.authorAcceptanceEntries,
      capability:'freeze-acceptance', retryScopes:new Set(['candidate_artifact'])});
  } catch (e) { error = e.message; }
  const trace = JSON.parse(JSON.stringify(ctx.progressTrace));
  const history = trace.map(step => step.entry);
  const prompt = (id, version) => (prompts.find(p => p.id === id && p.version === version) || {}).task || '';
  return {calls, gate, error, result, pauses, prompts, prompt, history, flags:history.map(step => step.progress), logs, rungs, trace, rounds};
}

const rungEvents = out => out.logs.filter(line => line.includes('repair rung'));
const assertAuthoredLadderContinued = out => {
  assert.equal(out.error, undefined, out.error);
  assert.ok(rungEvents(out).some(line => line.includes('r2')), `recorded calls advanced to r2: ${JSON.stringify(rungEvents(out))}`);
  assert.deepEqual(out.pauses, []);
};

function assertFirstRung(out, calls, flags) {
  assertAuthoredLadderContinued(out);
  assert.equal(out.rungs[0].calls, calls, 'the original progress window determines the first strategy change');
  assert.deepEqual(out.rungs[0].trace.map(step => step.entry.progress), flags);
  assert.equal(out.rungs[0].trace.at(-1).stalled, 3);
}
const credited = step => Array.from(step.entries || [], e => [e.subject, e.progress]);

// The judge refutes A and B; both repairs are refused at one defect; then A
// passes while B fails in the given way, and B then stays at one defect.
async function passBesideAdvancesAfterSiblingCredit(kind) {
  const out = await run({A:['ok', 1, 'ok'], B:['ok', 1, kind, 1]}, [refute(['A', 'B']), clean]);
  assertFirstRung(out, 9, [true, true, true, false, false, false]);
  assert.ok(credited(out.history[2]).some(([id, progress]) => id === 'A' && progress), 'A passing is credited beside every sibling failure kind');
  assert.equal(out.prompts.filter(prompt => prompt.id === 'A').length, 3, 'the accepted sibling stays settled');
}
const passBesideUnparseable = () => passBesideAdvancesAfterSiblingCredit('bad');
const passBesideTransport = () => passBesideAdvancesAfterSiblingCredit('down');
const passBesideRefusal = () => passBesideAdvancesAfterSiblingCredit(1);

// Entry credit restores the episode floor; the earlier judged repeat stays spent.
async function creditRestoresFloorBeforeLadderTransition() {
  const out = await run({A:['ok', 'ok', 1, 'ok'], B:['ok', 'ok', 1, 'bad', 1]}, [refute(['A', 'B']), refute(['A', 'B']), clean]);
  assertFirstRung(out, 10, [true, false, true, true, false, false]);
  assert.deepEqual(out.rungs[0].trace.map(step => step.stalled), [0, 1, 1, 1, 2, 3]);
  assert.equal(out.rungs[0].trace[3].floor, 1, 'credit cannot forgive the preceding judged repeat');
}

async function passCreditedOnceBeforeLadderTransition() {
  const out = await run({A:['ok', 1, 'ok'], B:['ok', 1, 'bad']}, [refute(['A', 'B']), clean]);
  assertFirstRung(out, 9, [true, true, true, false, false, false]);
  assert.deepEqual(credited(out.history[2]), [['B', true], ['A', true]]);
  for (const step of out.history.slice(3)) {
    assert.ok(!(step.entries || []).some(entry => entry.subject === 'A' && entry.progress), 'A earns no second pass credit');
  }
}

async function knownMalformedAlternationAdvancesWithoutNewCredit() {
  const out = await run({A:['ok', 'bad', 'bad', 'wrong', 'wrong', 'bad', 'wrong', 'bad']},
    [refute(['A']), clean], {cap:1});
  assertFirstRung(out, 7, [true, true, false, true, false, false, false]);
  const malformed = out.rungs[0].trace.slice(1).map(step => step.entry.entries[0]);
  assert.deepEqual(malformed.map(entry => entry.malformed_class),
    ['parse-error:unexpected-token:location-unknown', 'parse-error:unexpected-token:location-unknown',
      'wrong-id', 'wrong-id', 'parse-error:unexpected-token:location-unknown', 'wrong-id']);
  assert.deepEqual(malformed.map(entry => entry.progress), [true, false, true, false, false, false]);
}

async function unknownParserClassAdvancesWithoutRepeatCredit() {
  const out = await run({A:['badUnknown']}, [refute(['A']), clean], {cap:1});
  assertFirstRung(out, 4, [true, false, false, false]);
  assert.deepEqual(out.rungs[0].trace.map(step => step.entry.entries[0].malformed_class),
    Array(4).fill('parse-error:other:location-unknown'));
}

async function unknownStopReasonAdvancesWithoutRepeatCredit() {
  const out = await run({A:['stopUnknown']}, [refute(['A']), clean], {cap:1});
  assertFirstRung(out, 4, [true, false, false, false]);
  assert.deepEqual(out.rungs[0].trace.map(step => step.entry.entries[0].malformed_class), Array(4).fill('stop-other'));
}

async function mixedRefusalsRetainBothCreditsBeforeLadderTransition() {
  const out = await run({A:['bad'], B:[1]}, [refute(['A', 'B']), clean], {cap:2});
  assertFirstRung(out, 8, [true, false, false, false]);
  assert.deepEqual(credited(out.history[0]), [['A', true], ['B', true]]);
  assert.equal(out.history[0].entries[0].malformed_class, 'parse-error:unexpected-token:location-unknown');
  assert.equal(out.history[0].entries[1].findings, 1);
}

// A rewrite unmeasured by the entry validator is novelty, not progress.
async function unmeasuredRewriteAdvancesWithoutCredit(kind) {
  const out = await run({A:['ok'], B:['ok', kind]}, [refute(['A', 'B']), clean]);
  if (kind === 'bad') assertFirstRung(out, 7, [true, true, false, false, false]);
  else {
    assert.equal(out.error, 'paused');
    assert.equal(out.pauses[0].reason, 'operational_no_progress');
    assert.deepEqual(out.flags, [true, false, false, false]);
    assert.deepEqual(out.trace.map(step => step.stalled), [0, 0, 0, 0]);
    assert.equal(out.rungs.length, 0);
    assert.equal(out.pauses.length, 1);
  }
  assert.ok(!out.history.slice(1).some(step => credited(step).some(([id, progress]) => id === 'A' && progress)));
  if (kind === 'bad') assert.ok(credited(out.history[1]).some(([id, progress]) => id === 'B' && progress));
  else assert.equal(out.history[1].entries, undefined);
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

// A clean author pass stays retained when the next gate reports a host fault.
async function cleanPassSurvivesHostRetries() {
  const out = await run({A:['ok', 'ok', 1, 'ok']}, [refute(['A']), refute(['A']), outage, outage, clean]);
  assert.equal(out.error, undefined);
  assert.equal(out.gate, 5);
  assert.ok(out.history.some(step => step.kind === 'operational' && step.progress), 'the clean pass earns credit before the retry');
  assert.equal(out.pauses.length, 0);
}
// F2 control: the same pass beside a sibling's transport failure.
async function siblingPassIsCreditedBeforeHostRetry() {
  const out = await run({A:['ok', 'ok', 1, 'ok'], Z:['ok', 'ok', 1, 'down', 'ok']},
    [refute(['A', 'Z']), refute(['A', 'Z']), outage, clean]);
  assert.equal(out.error, undefined);
  assert.ok(out.rounds.some(round => round.passed.includes('A') && round.outage));
  assert.ok(out.history.some(step => (step.outage || step.kind === 'operational') && credited(step).some(([id, progress]) => id === 'A' && progress)));
  assert.equal(out.pauses.length, 0);
}
// The judged path credits a clean pass too, but a judged repeat still adds
// one to the next floor: A's pass forgives its stalled refusal (window 1 -> 0)
// and the refutation then counts (0 -> 1), so A gets one more stalled round.
async function cleanPassBeforeJudgedRepeatAdvancesTheLadder() {
  const out = await run({A:['ok', 1, 1, 'ok', 1]}, [refute(['A'])]);
  assertFirstRung(out, 7, [true, true, false, false, true, false, false]);
  assert.deepEqual(credited(out.history[3]), [['A', true]]);
  assert.equal(out.history[3].progress, false, 'the judged repeat remains no progress despite the pass');
}
// A judge that refutes every candidate advances the ladder until the fixture
// supplies a complete accepted candidate.
async function endlessRefutationAdvancesTheLadder() {
  const out = await run({A:(v) => v % 2 ? 'ok' : 1}, [refute(['A'])]);
  assert.equal(out.rungs[0].calls, 7, 'each judged repeat raises the next episode floor');
  assertAuthoredLadderContinued(out);
}

// A transport failure hidden behind an authored refusal stays operational.
async function hiddenOutageRetainsItsClassificationBeforeLadderTransition(kind) {
  const out = await run({A:['ok', 1, kind, 1], B:['ok', 1, 'down', 1]}, [refute(['A', 'B'])], {mode:'observe'});
  assertAuthoredLadderContinued(out);
  assert.equal(out.history[2].outage, true, 'a sibling transport failure remains visible behind the first refusal');
  assert.equal(out.trace[2].stalled, 0, 'the mixed operational round spends no authored attempt');
}
// The control without a transport failure spends the authored window.
async function noOutageAdvancesTheLadderToo() {
  const out = await run({A:['ok', 1], B:['ok', 1]}, [refute(['A', 'B'])], {mode:'observe'});
  assertAuthoredLadderContinued(out);
  assertFirstRung(out, 10, [true, true, false, false, false]);
  assert.notEqual(out.history[2].outage, true);
}

// A non-transient gate fault stops with its reported detail.
async function hostRetriesUseTheirOwnWindow() {
  const out = await run({A:['ok']}, [refute(['A']), refute(['A']), outage, outage, clean], {mode:'observe'});
  assert.equal(out.error, undefined);
  assert.equal(out.gate, 5);
  assert.deepEqual(out.pauses, []);
}
// A successful gate resets operational attempts; author findings before it do not.
async function successfulGateResetsTheHostRetryWindow() {
  const out = await run({A:['ok']}, [refute(['A']), refute(['A']), outage, refute(['A']), clean], {mode:'observe'});
  assert.equal(out.error, undefined);
  assert.equal(out.gate, 5);
  assert.deepEqual(out.pauses, []);
}
// Judged repeats advance observe through the same authored ladder.
async function observeJudgedRepeatsAdvanceTheLadder() {
  const out = await run({A:['ok']}, [refute(['A'])], {mode:'observe'});
  assertAuthoredLadderContinued(out);
  assertFirstRung(out, 4, [true, false, false, false]);
  assert.deepEqual(out.rungs[0].trace.map(step => step.entry.kind), Array(4).fill('judged'));
  assert.ok(out.prompts.some(prompt => /check 'A'/.test(prompt.task)), 'the judge finding remains in the author call');
}

// N1: an owed entry completed in a clean round is progress even when the
// freeze then has an outage (the round added a previously missing entry).
async function newEntryIsCreditedBeforeHostRetries() {
  const out = await run({A:['ok']}, [refute(['A']), owe('REQ-1'), outage, outage, clean]);
  assert.equal(out.error, undefined);
  assert.equal(out.gate, 5);
  assert.ok(out.rounds.some(round => round.added === 1 && round.passed.includes('SUP-REQ-1')));
  assert.ok(out.history.some(step => step.kind === 'operational' && step.progress));
  assert.deepEqual(out.pauses, []);
}
// A round that adds no entry and passes nothing is not progress at an outage.
async function unchangedEntryEarnsNoCreditBeforeHostWindow() {
  const out = await run({A:['ok']}, [refute(['A']), refute(['A']), outage, outage, outage, clean]);
  assert.equal(out.error, 'paused');
  assert.deepEqual(out.history.map(step => step.kind), ['judged','judged','operational','operational','operational']);
  assert.deepEqual(out.flags, [true, false, false, false, false]);
  assert.equal(out.pauses[0].reason, 'operational_no_progress');
  assert.doesNotMatch(JSON.stringify(out.pauses), /resume/i);
}
// The added entry remains present and is authored only once.
async function supplementaryEntryEarnsCreditOnceBeforeHostWindow() {
  const out = await run({A:['ok']}, [refute(['A']), owe('REQ-1'), outage, outage, outage, outage, clean]);
  assert.equal(out.error, 'paused');
  assert.deepEqual(out.flags, [true, false, true, false, false]);
  assert.equal(out.prompts.filter(prompt => prompt.id === 'SUP-REQ-1').length, 1);
  assert.equal(out.pauses[0].reason, 'operational_no_progress');
}

// A terminal gate fault does not create an authored progress measurement.
async function hostWindowSpendsNoAuthoredAttempt() {
  const out = await run({A:['ok']}, [outage, outage, outage]);
  assert.equal(out.error, 'paused');
  assert.equal(out.calls, 1);
  assert.equal(out.gate, 3);
  assert.deepEqual(out.history.map(step => step.kind), ['operational','operational','operational']);
  assert.deepEqual(out.trace.map(step => step.stalled), [0,0,0]);
  assert.equal(out.pauses[0].reason, 'operational_no_progress');
  assert.match(out.pauses[0].recovery, /freeze-acceptance.*judge down.*retries/);
  assert.doesNotMatch(out.pauses[0].recovery, /resume/i);
}

async function operationalRoundLeavesLadderWindowIntact() {
  const out = await run({A:['down', 1, 1, 1, 1, 1]}, [clean]);
  assertFirstRung(out, 5, [false, true, false, false, false]);
  assert.deepEqual(out.rungs[0].trace.map(step => step.entry.kind), ['operational', 'refused', 'refused', 'refused', 'refused']);
  assert.deepEqual(out.rungs[0].trace.map(step => step.stalled), [0, 0, 1, 2, 3]);
}

async function authorProgressDoesNotResetTheHostWindow() {
  const out = await run({A:['down', 1, 'ok', 'ok']}, [outage, outage, outage]);
  assert.equal(out.error, 'paused');
  assert.equal(out.pauses[0].reason, 'operational_no_progress');
  assert.deepEqual(out.history.map(step => step.kind), ['operational', 'refused', 'operational', 'operational']);
  assert.deepEqual(out.flags, [false,true,true,false]);
}

async function twoAuthorRefusalsAfterProviderFailureComplete() {
  const out = await run({A:['down', 1, 1, 'ok']}, [clean]);
  assert.equal(out.error, undefined, JSON.stringify(out.pauses));
  assert.deepEqual(out.pauses, []);
}

const tests = [
  ['A passes beside an unparseable sibling before the ladder advances', passBesideUnparseable],
  ['A passes beside a transport fault before the ladder advances', passBesideTransport],
  ['A passes beside a refused sibling before the ladder advances', passBesideRefusal],
  ['entry credit restores the episode floor before the ladder advances', creditRestoresFloorBeforeLadderTransition],
  ['a pass earns credit once across ladder transitions', passCreditedOnceBeforeLadderTransition],
  ['known malformed alternation advances the ladder without new credit', knownMalformedAlternationAdvancesWithoutNewCredit],
  ['unknown parser classes advance the ladder without repeat credit', unknownParserClassAdvancesWithoutRepeatCredit],
  ['unknown stop reasons advance the ladder without repeat credit', unknownStopReasonAdvancesWithoutRepeatCredit],
  ['mixed refusals retain both progress credits before the ladder advances', mixedRefusalsRetainBothCreditsBeforeLadderTransition],
  ['an unmeasured rewrite gets no credit before the ladder advances', () => unmeasuredRewriteAdvancesWithoutCredit('bad')],
  ['an unmeasured rewrite gets no credit before the host retry window', () => unmeasuredRewriteAdvancesWithoutCredit('down')],
  ['an unparseable reply is noted for its own entry only', malformedNoteIsPerEntry],
  ['a refusal is replaced by an unparseable note', () => refusalAfterOtherFailure('bad')],
  ['a refusal survives a transport failure', () => refusalAfterOtherFailure('down')],
  ['F1 probe: a repair after an outage still reads its refusal', refusalSurvivesOutage],
  ['F2: a clean pass remains retained when the freeze reports a host fault', cleanPassSurvivesHostRetries],
  ['F2 control: a pass beside a sibling outage retains its credit before the host fault', siblingPassIsCreditedBeforeHostRetry],
  ['a clean pass before a judged repeat advances the ladder', cleanPassBeforeJudgedRepeatAdvancesTheLadder],
  ['an endless refutation advances through repairs', endlessRefutationAdvancesTheLadder],
  ['F3: a hidden outage behind a refusal preserves the authored window', () => hiddenOutageRetainsItsClassificationBeforeLadderTransition(1)],
  ['F3: a hidden outage behind an unparseable reply preserves the authored window', () => hiddenOutageRetainsItsClassificationBeforeLadderTransition('bad')],
  ['F3 control: the window without an outage advances the ladder', noOutageAdvancesTheLadderToo],
  ['host retries use their own window', hostRetriesUseTheirOwnWindow],
  ['successful gate resets the host retry window', successfulGateResetsTheHostRetryWindow],
  ['observe judged repeats advance the ladder', observeJudgedRepeatsAdvanceTheLadder],
  ['N1: the added entry remains retained before host retries', newEntryIsCreditedBeforeHostRetries],
  ['N1: no new entry means no progress before the host retry window', unchangedEntryEarnsNoCreditBeforeHostWindow],
  ['N1: the supplementary entry is added once before the host retry window', supplementaryEntryEarnsCreditOnceBeforeHostWindow],
  ['the host retry window spends no authored attempt', hostWindowSpendsNoAuthoredAttempt],
  ['an operational round leaves the authored ladder window intact', operationalRoundLeavesLadderWindowIntact],
  ['author progress does not reset the host retry window', authorProgressDoesNotResetTheHostWindow],
  ['one provider failure and two author refusals still complete', twoAuthorRefusalsAfterProviderFailureComplete],
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
