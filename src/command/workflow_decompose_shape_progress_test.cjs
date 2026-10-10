// Issue 357 rounds 2-3: author shape repairs use the freeze progress measure,
// within the repair episode of the entry they repair.
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js']
  .map(name => fs.readFileSync(`${root}/${name}`, 'utf8')).join('\n');
const shape = (field, text = `${field} invalid`) => ({
  text: `candidate artifact was refused: ${text}`,
  deterministic_defect: {provenance:'host_validator', code:'invalid_candidate_shape',
    subject:`entries/0/${field}`, location:'shape', stage:'shape'},
});
const fields = ['criterion', 'check/command', 'check/cwd', 'gap_permitted', 'covers/0'];
const defects = n => fields.slice(0, n).map(field => shape(field));
const accepted = entry => ({status:'accepted', stopReason:'end_turn', content:JSON.stringify(entry)});
const clean = () => ({publicationReceipt:{call_id:'freeze'}, postcondition:{satisfied:true},
  gateEnvelope:{policy_findings:[]}});

async function run(sequence, {supplementary = false, wrapper = false, resumed = false, siblings = false} = {}) {
  const ctx = {args:{acceptanceCriteria:siblings ? {A:'a',B:'b'} : {A:'a'}, prdRequirementTexts:{'REQ-X':'x'}, authorMaxParallelism:1, gateMode:'enforce'},
    __archonValidateAcceptanceEntry: (_, serialized) => {
      const entry = JSON.parse(serialized);
      return JSON.stringify(entry.id === 'A' && supplementary ? [] : sequence(entry.version));
    }};
  vm.createContext(withAuthorContext(ctx)); vm.runInContext(source, ctx);
  if (supplementary) ctx.owedSupplementary().set('SUP-REQ-X', {requirement:'REQ-X',text:'x'});
  let calls = 0, freezes = 0;
  const versions = new Map();
  const pauses = [], prompts = [], rounds = [], ids = [];
  let error, result;
  try {
    result = await ctx.authorCandidate({
      agent: async (call, options) => {
        if (calls >= 40) throw new Error('bounded test exhausted');
        const id = options.task.match(/Author ONLY entry ([^:]+):/)[1];
        ids.push(id);
        if (id === 'A' && supplementary) return accepted({id,criterion:''});
        prompts.push(options.task); rounds.push(call); calls++;
        const version = (versions.get(id) || 0) + 1; versions.set(id, version);
        if (sequence(version) === 'transport') return {status:'failed',summary:'transport'};
        if (sequence(version) === 'incomplete') return {status:'accepted',stopReason:'end_turn',content:'malformed'};
        const entry = {id,version,criterion:'',check:{kind:'command',command:'test -f output',cwd:'project_root'}};
        return accepted(wrapper ? {acceptance:[entry]} : entry);
      },
      hostCommand: async () => { freezes++; return clean(); },
      pause: async (_, evidence) => {
        pauses.push(evidence);
        if (!resumed || pauses.length > 1) throw new Error('paused');
      },
    }, {phase:'acceptance',prompt:()=> 'author',author:ctx.authorAcceptanceEntries,
      capability:'freeze-acceptance',retryScopes:new Set(['candidate_artifact'])});
  } catch (e) { error = e.message; }
  return {calls,freezes,pauses,prompts,rounds,ids,error,result};
}

async function shrinking(options = {}) {
  const out = await run(n => defects(Math.max(6 - n, 0)), options);
  assert.equal(out.error, undefined, JSON.stringify(out.pauses));
  assert.equal(out.calls, 6, '5 -> 4 -> 3 -> 2 -> 1 -> 0 has no total-attempt cap');
  assert.equal(out.freezes, 1, 'only shape-valid entries reach freeze');
  assert.equal(out.pauses.length, 0);
  assert.match(out.prompts[1], /criterion invalid/, 'native diagnostic reaches repair prompt');
  if (options.supplementary) assert.equal(out.ids.filter(id => id === 'A').length, 1, 'retained sibling survives');
}

async function siblings() {
  const out = await run(n => defects(Math.max(6 - n, 0)), {siblings:true});
  assert.equal(out.error, undefined, JSON.stringify(out.pauses));
  assert.equal(out.calls, 12, 'each missing sibling can repair five shape defects');
  assert.equal(out.freezes, 1);
  assert.equal(out.ids.filter(id => id === 'A').length, 6, 'completed sibling remains retained');
}

async function unchanged(kind) {
  const out = await run(n => {
    const base = defects(2);
    if (kind === 'reworded') return fields.slice(0, 2).map(field => shape(field, `${field} invalid version ${n}`));
    if (kind === 'duplicates') return n === 1 ? [...base, ...base] : base;
    return base;
  });
  assert.equal(out.error, 'paused', 'stall pauses the run');
  assert.equal(out.calls, 4, 'one measured baseline then three unchanged attempts');
  assert.equal(out.freezes, 0, 'no false publication');
  const evidence = out.pauses[0];
  assert.equal(evidence.reason, 'no_progress');
  assert.equal(evidence.author_calls, 4);
  assert.equal(evidence.answered_attempts, 4, 'provider replies counted once');
  assert.deepEqual(Array.from(evidence.progress_history, step => [step.kind,step.stage,step.findings,step.progress]),
    [['refused','shape',2,true],['refused','shape',2,false],['refused','shape',2,false],['refused','shape',2,false]]);
}

async function oscillating() {
  const out = await run(n => defects(n % 2 ? 3 : 2));
  assert.equal(out.error, 'paused');
  assert.equal(out.calls, 5, 'returning to a prior best does not reset the window');
  assert.deepEqual(Array.from(out.pauses[0].progress_history, step => step.progress), [true,true,false,false,false]);
}

async function resume() {
  const out = await run(() => defects(2), {resumed:true});
  assert.equal(out.error, 'paused');
  assert.equal(out.calls, 7, 'resume opens only the stall window, preserving the best');
  assert.equal(out.pauses.length, 2);
  assert.deepEqual(Array.from(out.pauses[1].progress_history, step => step.progress), [true,false,false,false,false,false,false]);
}

async function mixed() {
  const out = await run(n => n === 2 ? 'transport' : n === 3 ? 'incomplete' : defects(2));
  assert.equal(out.error, 'paused');
  assert.equal(out.calls, 6, 'the first malformed class progresses once; its repeat and shape stalls then use the author window');
  assert.equal(out.pauses[0].reason, 'no_progress');
  assert.equal(out.pauses[0].answered_attempts, 5);
  assert.deepEqual(Array.from(out.pauses[0].progress_history, step => step.progress), [true,false,true,false,false,false]);
}

// Round 3: a judged refutation opens a repair episode for each refuted entry.
// Version 1 of every entry is shape-valid and reaches the judge; `sequence(n)`
// gives the shape defects of repair n. The judge refutes `ids` on the first
// freeze, or on every freeze when `always`.
async function refuted(sequence, {ids = ['A'], always = false, resumed = false} = {}) {
  const ctx = {args:{acceptanceCriteria:Object.fromEntries(ids.map(id => [id, id])),authorMaxParallelism:1,gateMode:'enforce'},
    __archonValidateAcceptanceEntry: (_, serialized) => {
      const entry = JSON.parse(serialized);
      return JSON.stringify(entry.version === 1 ? [] : sequence(entry.version - 1));
    }};
  vm.createContext(withAuthorContext(ctx)); vm.runInContext(source, ctx);
  let calls = 0, gates = 0, error;
  const versions = new Map(), pauses = [];
  try {
    await ctx.authorCandidate({
      agent: async (_, options) => {
        if (++calls > 60) throw new Error('bounded test exhausted');
        const id = options.task.match(/Author ONLY entry ([^:]+):/)[1];
        const version = (versions.get(id) || 0) + 1; versions.set(id, version);
        return accepted({id,version,criterion:'c'});
      },
      hostCommand: async () => {
        gates++;
        if (gates > 1 && !always) return clean();
        return {...clean(),gateEnvelope:{policy_findings:ids.map(id => ({
          text:`check '${id}' was refuted: repair`,subject:id,remediation_scope:'candidate_artifact'}))}};
      },
      pause: async (_, evidence) => {
        pauses.push(evidence);
        if (!resumed || pauses.length > 1) throw new Error('paused');
      },
    }, {phase:'acceptance',prompt:()=> 'author',author:ctx.authorAcceptanceEntries,
      capability:'freeze-acceptance',retryScopes:new Set(['candidate_artifact'])});
  } catch (e) { error = e.message; }
  const steps = n => Array.from(pauses[n].progress_history, step => [step.kind,step.stage,step.findings,step.progress]);
  return {calls,gates,error,pauses,steps};
}
const fiveToZero = n => defects(Math.max(6 - n, 0));
const judged = (progress) => ['judged','passed',0,progress];
const refused = (count, progress) => ['refused','shape',count,progress];

async function afterJudgeShrinks() {
  const out = await refuted(fiveToZero);
  assert.equal(out.error, undefined, JSON.stringify(out.pauses));
  assert.equal(out.calls, 7, 'one judged candidate, then 5 -> 4 -> 3 -> 2 -> 1 -> 0');
  assert.equal(out.gates, 2, 'only the shape-valid repair reaches the second freeze');
}

async function afterJudgeTwoEntries() {
  const out = await refuted(fiveToZero, {ids:['A','B']});
  assert.equal(out.error, undefined, JSON.stringify(out.pauses));
  assert.equal(out.calls, 14, 'each refuted entry has its own frontier: A then B repair 5 -> 0');
  assert.equal(out.gates, 2);
}

async function afterJudgeStalls(sequence, history, resumed = false) {
  const out = await refuted(sequence, {resumed});
  assert.equal(out.error, 'paused', 'a repair that stops improving pauses');
  assert.equal(out.gates, 1, 'no false publication');
  assert.equal(out.pauses[0].reason, 'no_progress');
  assert.deepEqual(out.steps(out.pauses.length - 1), history);
}

async function refutedLoop() {
  const out = await refuted(n => defects(2 - (n - 1) % 3), {always:true}); // each episode 2 -> 1 -> 0
  assert.equal(out.error, 'paused', 'repair credit never erases the repeated refutations');
  assert.equal(out.gates, 4);
  assert.equal(out.calls, 10);
  const episode = [refused(2,true),refused(1,true)];
  assert.deepEqual(out.steps(0), [judged(true),...episode,judged(false),...episode,judged(false),...episode,judged(false)]);
}

const tests = [
  ['shape repairs decrease 5 to 0',()=>shrinking()],
  ['wrapped shape repairs decrease 5 to 0',()=>shrinking({wrapper:true})],
  ['supplementary repairs decrease with retained sibling',()=>shrinking({supplementary:true})],
  ['shape repairs continue across missing siblings',siblings],
  ...['same','reworded','duplicates'].map(kind => [`unchanged shape ${kind} pauses`,()=>unchanged(kind)]),
  ['shape oscillation preserves best',oscillating],['shape resume preserves best',resume],
  ['shape and operational failures use separate windows',mixed],
  ['refuted entry shape repairs decrease 5 to 0',afterJudgeShrinks],
  ['two refuted entries each repair 5 to 0',afterJudgeTwoEntries],
  ['refuted entry unchanged shape pauses',()=>afterJudgeStalls(() => defects(2),
    [judged(true),refused(2,true),refused(2,false),refused(2,false),refused(2,false)])],
  ['refuted entry oscillating shape pauses',()=>afterJudgeStalls(n => defects(n % 2 ? 3 : 2),
    [judged(true),refused(3,true),refused(2,true),refused(3,false),refused(2,false),refused(3,false)])],
  ['refuted entry resume preserves its best',()=>afterJudgeStalls(() => defects(2),
    [judged(true),refused(2,true),...Array(6).fill(refused(2,false))], true)],
  ['repeated refutations pause despite shape repairs',refutedLoop],
];
module.exports = tests;
if (require.main === module) (async () => {
  let failed = 0;
  for (const [name,test] of tests) {
    try {await test(); console.log(`PASS ${name}`);}
    catch (e) {failed++; console.error(`FAIL ${name}: ${e.message.slice(0, 600)}`);}
  }
  process.exitCode = failed ? 1 : 0;
})();
