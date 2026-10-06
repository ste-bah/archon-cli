// Issue 357 round 2: author shape repairs use the freeze progress measure.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['workflow_decompose_v1.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_progress.js']
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
  const ctx = {args:{acceptanceCriteria:siblings ? {A:'a',B:'b'} : {A:'a'}, authorMaxParallelism:1, gateMode:'enforce'},
    __archonValidateAcceptanceEntry: serialized => {
      const entry = JSON.parse(serialized);
      return JSON.stringify(entry.id === 'A' && supplementary ? [] : sequence(entry.version));
    }};
  vm.createContext(ctx); vm.runInContext(source, ctx);
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
  assert.equal(out.calls, 4, 'transport and incomplete replies share the no-progress window');
  assert.equal(out.pauses[0].answered_attempts, 3);
  assert.deepEqual(Array.from(out.pauses[0].progress_history, step => step.progress), [true,false,false,false]);
}

// A native shape refusal must not credit a lower tier after the freeze reached
// a judge. This is the same tier ordering as the parent freeze path.
async function afterJudge() {
  const ctx = {args:{acceptanceCriteria:{A:'a'},authorMaxParallelism:1,gateMode:'enforce'}};
  let calls = 0, gates = 0, paused;
  ctx.__archonValidateAcceptanceEntry = () => JSON.stringify(calls === 1 ? [] : defects(7 - calls));
  vm.createContext(ctx); vm.runInContext(source, ctx);
  await assert.rejects(ctx.authorCandidate({
    agent: async () => {calls++; return accepted({id:'A',criterion:''});},
    hostCommand: async () => {gates++; return {...clean(),gateEnvelope:{policy_findings:[{
      text:"check 'A': repair",subject:'A',remediation_scope:'candidate_artifact'}]}};},
    pause: async (_, evidence) => {paused = evidence; throw new Error('paused');},
  }, {phase:'acceptance',prompt:()=> 'author',author:ctx.authorAcceptanceEntries,
    capability:'freeze-acceptance',retryScopes:new Set(['candidate_artifact'])}), /paused/);
  assert.equal(calls, 4);
  assert.equal(gates, 1);
  assert.deepEqual(Array.from(paused.progress_history, step => [step.kind,step.stage,step.progress]),
    [['judged','passed',true],['refused','shape',false],['refused','shape',false],['refused','shape',false]]);
}

const tests = [
  ['shape repairs decrease 5 to 0',()=>shrinking()],
  ['wrapped shape repairs decrease 5 to 0',()=>shrinking({wrapper:true})],
  ['supplementary repairs decrease with retained sibling',()=>shrinking({supplementary:true})],
  ['shape repairs continue across missing siblings',siblings],
  ...['same','reworded','duplicates'].map(kind => [`unchanged shape ${kind} pauses`,()=>unchanged(kind)]),
  ['shape oscillation preserves best',oscillating],['shape resume preserves best',resume],
  ['shape and operational failures share a window',mixed],['shape cannot regress judged tier',afterJudge],
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
