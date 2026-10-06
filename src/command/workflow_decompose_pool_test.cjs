// Issue-247: author fan-out is a bounded pool, not barrier batches. Time is a
// virtual clock (no wall-clock assertion): a call "takes" its duration in
// clock units, and the clock jumps to the next timer once every microtask has
// drained. Proves: makespan follows the pool / prefix-window schedule, the
// in-flight count never exceeds the cap, an acceptance prompt depends on its
// index alone, and a failure stops new starts but lets started calls settle.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const dir = process.env.ARCHON_DECOMPOSE_SCRIPT_DIR || __dirname;
const scriptSource = () => ['workflow_decompose_v1.js','workflow_decompose_v1_acceptance.js','workflow_decompose_v1_set_gate.js','workflow_decompose_v1_progress.js'].map(f=>fs.readFileSync(dir+'/'+f,'utf8')).join('\n');

function clock() {
 let now=0,seq=0; const timers=[];
 const sleep=ms=>new Promise(resolve=>timers.push({at:now+ms,seq:seq++,resolve}));
 async function drive(promise) {
  let done=false,value,error,failed=false;
  promise.then(v=>{done=true;value=v;},e=>{done=true;failed=true;error=e;});
  for(;;) {
   await new Promise(resolve=>setImmediate(resolve));
   if(done) break;
   if(timers.length===0) throw new Error('deadlock: nothing pending and not done');
   timers.sort((a,b)=>a.at-b.at||a.seq-b.seq);
   const timer=timers.shift(); now=timer.at; timer.resolve();
  }
  if(failed) throw error;
  return value;
 }
 return {sleep,drive,now:()=>now};
}

function context(args={}, validator=() => '[]') {
 const ctx={args,console}; ctx.__archonValidateAcceptanceEntry = (_, serialized) => validator(serialized);
  vm.createContext(ctx); vm.runInContext(scriptSource(),ctx); return ctx;
}

// Pool 110 < prefix window 120 < barrier batches 130 for these durations, cap 2.
async function makespanFollowsTheSchedule() {
 const durations=[30,60,10,10,60,10];
 for(const [window,expected] of [[false,110],[true,120]]) {
  const c=clock(); const ctx=context(); let active=0,peak=0; const starts=[];
  const {settled,started}=await c.drive(ctx.runBounded(durations.length,2,window,async index=>{
   starts[index]=c.now(); active++; peak=Math.max(peak,active);
   await c.sleep(durations[index]); active--; return index;
  }));
  assert.equal(c.now(),expected,`window=${window} makespan`);
  assert.equal(peak,2,'never more than cap in flight');
  assert.equal(started,durations.length);
  assert.deepEqual(Array.from(settled,s=>s.value),[0,1,2,3,4,5],'results in input order');
  assert.deepEqual(starts,[...starts].sort((a,b)=>a-b),'indices start in order');
 }
}

// A slow call 0 delays only the calls at index >= cap in the window; in the
// plain pool it delays nothing.
async function slowFirstCallBlocksOnlyCallsBehindIt() {
 const durations=[100,10,10,10,10,10];
 for(const [window,expected] of [[true,[0,0,0,100,100,100]],[false,[0,0,0,10,10,20]]]) {
  const c=clock(); const ctx=context(); const starts=[];
  await c.drive(ctx.runBounded(durations.length,3,window,async index=>{starts[index]=c.now(); await c.sleep(durations[index]);}));
  assert.deepEqual(starts,expected,`window=${window} start times`);
 }
}

function seeded(seed) { let s=seed; return ()=>{s=(s*1103515245+12345)%2147483648; return s/2147483648;}; }

function acceptanceRun(seed, cap, criteria, state, options={}) {
 const c=clock(); const ctx=context({acceptanceCriteria:criteria,authorMaxParallelism:cap},options.validator);
 const random=seeded(seed); const prompts={},ends=[],starts=[]; let active=0,peak=0;
 const w={agent:async(_,opts)=>{
  const id=opts.task.match(/Author ONLY entry ([^:]+):/)[1];
  prompts[id]=opts.task; starts.push({id,at:c.now()}); active++; peak=Math.max(peak,active);
  await c.sleep(options.durations?.[id] ?? 1+Math.floor(random()*50)); active--; ends.push({id,at:c.now(),active});
  if(options.throws===id) throw new Error(options.throwMessage||`stopped at ${id}`);
  if(options.fails===id&&(options.round||1)===1) return {status:'failed',summary:'transport'};
  return {status:'accepted',stopReason:'end_turn',content:JSON.stringify({id,seed})};
 }};
 const run=c.drive(ctx.authorAcceptanceEntries(w,'author',options.round||1,state||{entries:new Map(),retryIds:null}));
 return {run,prompts,ends,starts,peak:()=>peak,active:()=>active,now:c.now};
}

// Issue 288: one JSON line per completed entry, its id first.
const priorIds=task=>task.split('Previously completed entries')[1].split('\n').slice(1).filter(line=>line.startsWith('- ')).map(line=>line.match(/^- \{"id":"([^"]*)"/)[1]);

// Entry i sees exactly entries 0..i-cap of this round, however the calls
// happen to finish: two seeds, different completion orders, same prompts.
async function promptsDependOnIndexAlone() {
 const criteria=Object.fromEntries(Array.from({length:9},(_,i)=>[`AC-${i+1}`,`criterion ${i+1}`]));
 const ids=Object.keys(criteria).sort(); const cap=3;
 const runs=[];
 for(const seed of [7,1234]) {
  const r=acceptanceRun(seed,cap,criteria); const out=await r.run;
  assert.equal(out.status,'accepted');
  assert(r.peak()<=cap,'in-flight count never exceeds cap'); assert.equal(r.peak(),cap);
  ids.forEach((id,i)=>assert.deepEqual(priorIds(r.prompts[id]),ids.slice(0,Math.max(0,i-cap+1)),`prior of ${id}`));
  runs.push(r);
 }
 assert.notDeepEqual(runs[0].ends.map(e=>e.id),runs[1].ends.map(e=>e.id),'the seeds must finish in different orders');
 const strip=prompts=>Object.fromEntries(Object.entries(prompts).map(([k,v])=>[k,v.replace(/"seed":\d+/g,'')]));
 assert.deepEqual(strip(runs[0].prompts),strip(runs[1].prompts),'prompts identical across completion orders');
 // A retry round sees earlier-round entries first, then its own window.
 const state={entries:new Map(ids.map(id=>[id,{id,old:true}])),retryIds:new Set(['AC-2','AC-5','AC-7','AC-9'])};
 const r=acceptanceRun(99,2,criteria,state); await r.run;
 assert.deepEqual(Object.keys(r.prompts).sort(),['AC-2','AC-5','AC-7','AC-9']);
 const earlier=['AC-1','AC-3','AC-4','AC-6','AC-8'];
 assert.deepEqual(priorIds(r.prompts['AC-2']),earlier);
 assert.deepEqual(priorIds(r.prompts['AC-7']),[...earlier,'AC-2']);
 assert.deepEqual(priorIds(r.prompts['AC-9']),[...earlier,'AC-2','AC-5']);
}

// Issue 357 round 4: a failed entry limits new starts to the indices whose
// fixed prefix holds no failure, and every started sibling settles. Every
// entry that succeeded is kept, the ones after the failure too; the failure
// and the indices never started are retried. Two timings -- A ends before B
// fails, or after -- start the same calls and leave the same entries and
// retry set, so the next round's prompts are identical.
async function failureStopsNewStartsAndSettlesSiblings() {
 const criteria={A:'a',B:'b',C:'c',D:'d',E:'e',F:'f'};
 const outcomes=[];
 for(const aTakes of [5,20]) {
  const durations={A:aTakes,B:10,C:30,D:5,E:5,F:5};
  const state={entries:new Map(),retryIds:null};
  const r=acceptanceRun(1,3,criteria,state,{durations,fails:'B'});
  const out=await r.run;
  assert.equal(out.status,'failed');
  assert.deepEqual(r.starts.map(s=>s.id),['A','B','C','D'],'D (prefix A) starts in both timings; E (prefix A,B) never');
  assert.equal(r.active(),0,'started siblings settled before return');
  assert.equal(r.now(),30,'returned only after the slowest started sibling');
  assert.deepEqual(Array.from(state.entries.keys()),['A','C','D'],'every entry that succeeded is kept');
  assert.deepEqual(Array.from(state.retryIds),['B','E','F'],'only the failure and the unstarted indices are retried');
  const next=acceptanceRun(2,3,criteria,state,{durations,round:2});
  assert.equal((await next.run).status,'accepted');
  assert.deepEqual(Object.keys(next.prompts).sort(),['B','E','F']);
  outcomes.push(next.prompts);
 }
 assert.deepEqual(outcomes[0],outcomes[1],'the next round does not depend on the earlier timing');
}

// Issue 357 round 4 (F3): a shape refusal of one entry keeps the sibling that
// passed beside it, whichever finished first. Several failures in one round
// return the lowest index's failure and retry each of them.
async function refusalKeepsPassedSiblings() {
 const criteria={A:'a',B:'b',C:'c',D:'d'};
 const refusal=id=>[{text:`acceptance entry '${id}' was refused: criterion is missing`,
  deterministic_defect:{provenance:'host_validator',code:'invalid_candidate_shape',subject:'entries/0/criterion',location:'shape',stage:'shape'}}];
 for(const [cap,refused,starts,kept,retry] of [
  [2,['A'],['A','B'],['B'],['A','C','D']],
  [3,['A','C'],['A','B','C'],['B'],['A','C','D']],
  [3,['B'],['A','B','C','D'],['A','C','D'],['B']],
 ]) {
  const prompts=[];
  for(const bTakes of [5,40]) {
   const state={entries:new Map(),retryIds:null};
   const validator=(serialized)=>JSON.stringify(refused.includes(JSON.parse(serialized).id)?refusal(JSON.parse(serialized).id):[]);
   const r=acceptanceRun(1,cap,criteria,state,{durations:{A:20,B:bTakes,C:10,D:5},validator});
   const out=await r.run;
   assert.equal(out.status,'failed');
   assert.equal(out.entryId,refused[0],'the lowest refused index is returned');
   assert.deepEqual(r.starts.map(s=>s.id),starts,`cap ${cap} starts`);
   assert.deepEqual(Array.from(state.entries.keys()),kept,`cap ${cap} refused ${refused} keeps passed siblings`);
   assert.deepEqual(Array.from(state.retryIds),retry,`cap ${cap} refused ${refused} retries only the unfinished`);
   const next=acceptanceRun(2,cap,criteria,state,{durations:{A:20,B:bTakes,C:10,D:5}});
   assert.equal((await next.run).status,'accepted');
   assert.deepEqual(Object.keys(next.prompts).sort(),retry,'a passed sibling is not re-authored');
   prompts.push(Object.fromEntries(Object.entries(next.prompts).map(([k,v])=>[k,v.replace(/"seed":\d+/g,'')])));
  }
  assert.deepEqual(prompts[0],prompts[1],'retained siblings do not depend on timing');
 }
}

// A stop raised inside one call (a pause or cancel the host observed) is
// rethrown only after every started sibling has settled: none is abandoned.
async function rejectionSettlesSiblingsBeforeRaising() {
 const criteria={A:'a',B:'b',C:'c',D:'d'};
 const r=acceptanceRun(1,3,criteria,null,{durations:{A:50,B:10,C:30,D:5},throws:'B'});
 await assert.rejects(r.run,/stopped at B/);
 assert.equal(r.active(),0,'no sibling abandoned mid-call');
 assert.equal(r.now(),50);
 assert.deepEqual(r.starts.map(s=>s.id),['A','B','C'],'nothing starts after the stop');
}

// A stop thrown by a later call outranks a failed reply at a lower index:
// returned as a failed value it would be retried as an operational failure.
async function rejectionOutranksAFailedReply() {
 const criteria={A:'a',B:'b',C:'c',D:'d',E:'e'};
 const pause='workflow paused by run control: generation 2 observed before/after V2 call';
 const r=acceptanceRun(1,3,criteria,null,{durations:{A:5,B:10,C:30,D:7,E:5},fails:'B',throws:'D',throwMessage:pause});
 await assert.rejects(r.run,error=>error.message===pause);
 assert.deepEqual(r.starts.map(s=>s.id),['A','B','C','D'],'D started before B failed');
 assert.equal(r.active(),0,'no sibling abandoned mid-call');
}

makespanFollowsTheSchedule().then(slowFirstCallBlocksOnlyCallsBehindIt).then(promptsDependOnIndexAlone)
 .then(failureStopsNewStartsAndSettlesSiblings).then(refusalKeepsPassedSiblings).then(rejectionSettlesSiblingsBeforeRaising).then(rejectionOutranksAFailedReply)
 .then(()=>console.log('bounded author pool passed')).catch(e=>{console.error(e);process.exitCode=1});
