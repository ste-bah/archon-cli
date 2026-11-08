// Issue-247: author fan-out is a bounded pool, not barrier batches. Time is a
// virtual clock (no wall-clock assertion): a call "takes" its duration in
// clock units, and the clock jumps to the next timer once every microtask has
// drained. Proves: makespan follows the pool schedule, the in-flight count
// never exceeds the cap, a prompt depends only on its id and pre-round state,
// and an operational failure stops new starts but lets started
// calls settle.
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const dir = process.env.ARCHON_DECOMPOSE_SCRIPT_DIR || __dirname;
const scriptSource = () => ['workflow_decompose_v1.js', 'workflow_decompose_v1_body.js','workflow_decompose_reply_blocks.js','workflow_decompose_v1_acceptance.js','workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js', "workflow_decompose_v1_prompt.js",'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_failing_set.js', 'workflow_decompose_v1_context.js'].map(f=>fs.readFileSync(dir+'/'+f,'utf8')).join('\n');

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
  vm.createContext(withAuthorContext(ctx)); vm.runInContext(scriptSource(),ctx); return ctx;
}

// Pool 110 < barrier batches 130 for these durations, cap 2.
async function makespanFollowsTheSchedule() {
 const durations=[30,60,10,10,60,10];
 {
  const c=clock(); const ctx=context(); let active=0,peak=0; const starts=[];
  const {settled,started}=await c.drive(ctx.runBounded(durations.length,2,async index=>{
   starts[index]=c.now(); active++; peak=Math.max(peak,active);
   await c.sleep(durations[index]); active--; return index;
  }));
  assert.equal(c.now(),110,'pool makespan');
  assert.equal(peak,2,'never more than cap in flight');
  assert.equal(started,durations.length);
  assert.deepEqual(Array.from(settled,s=>s.value),[0,1,2,3,4,5],'results in input order');
  assert.deepEqual(starts,[...starts].sort((a,b)=>a-b),'indices start in order');
 }
}

// With cap 4, entry 4 uses the first freed slot while entry 0 is still running.
async function slowFirstCallDoesNotBlockTheFifthEntry() {
 const criteria={A:'a',B:'b',C:'c',D:'d',E:'e',F:'f'};
 const r=acceptanceRun(1,4,criteria,null,{durations:{A:100,B:10,C:20,D:30,E:5,F:5}});
 assert.equal((await r.run).status,'accepted');
 assert.equal(r.starts[4].id,'E');
 assert.equal(r.starts[4].at,10,'index 4 starts as soon as index 1 frees its slot');
 assert.equal(r.peak(),4);
 assert.equal(r.active(),0);
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
  if([].concat(options.fails).includes(id)&&(options.round||1)===1) return {status:'failed',summary:'transport'};
  if(options.malformed?.includes(id)) return {status:'accepted',stopReason:'end_turn',content:'{"id":'};
  return {status:'accepted',stopReason:'end_turn',content:JSON.stringify({id,seed,covers:[]})};
 }};
 const run=c.drive(ctx.authorAcceptanceEntries(w,'author',options.round||1,state||{entries:new Map(),retryIds:null}));
 return {run,prompts,ends,starts,files:ctx.__archonAuthorContext.files,peak:()=>peak,active:()=>active,now:c.now};
}

// Issue 288: one record line per completed entry, its id and digest first.
const priorIds=task=>task.split('Previously completed entries')[1].split('\n').slice(1).filter(line=>line.startsWith('- ')).map(line=>line.match(/^- (\S+) sha256:[0-9a-f]{64} /)[1]);

// No entry sees same-round results: different completion orders, exact prompts.
async function promptsDependOnlyOnPreRoundState() {
 const criteria=Object.fromEntries(Array.from({length:9},(_,i)=>[`AC-${i+1}`,`criterion ${i+1}`]));
 const ids=Object.keys(criteria).sort(); const cap=3;
 const runs=[];
 for(const seed of [7,1234]) {
  const r=acceptanceRun(seed,cap,criteria); const out=await r.run;
  assert.equal(out.status,'accepted');
  assert(r.peak()<=cap,'in-flight count never exceeds cap'); assert.equal(r.peak(),cap);
  ids.forEach(id=>assert.deepEqual(priorIds(r.prompts[id]),[],`prior of ${id}`));
  runs.push(r);
 }
 assert.notDeepEqual(runs[0].ends.map(e=>e.id),runs[1].ends.map(e=>e.id),'the seeds must finish in different orders');
 assert.deepEqual(runs[0].prompts,runs[1].prompts,'prompts identical across completion orders');
}

async function rewritePriorKeepsOtherPreviousBestVersions() {
 const criteria={A:'a',B:'b',C:'c',D:'d',E:'e'};
 const ids=Object.keys(criteria), pending=['A','C','E'];
 const runs=[];
 for(const seed of [7,1234]) {
  const entries=new Map([...ids].reverse().map(id=>[id,{id,old:true,covers:[]}]));
  const state={entries,retryIds:new Set(pending)};
  const r=acceptanceRun(seed,2,criteria,state);
  assert.equal((await r.run).status,'accepted');
  assert.deepEqual(Object.keys(r.prompts).sort(),pending);
  for(const id of pending) {
   assert.deepEqual(priorIds(r.prompts[id]),ids.filter(other=>other!==id),'prior follows entry order and excludes self');
   const records=r.prompts[id].split('Previously completed entries')[1].split('\n').filter(line=>line.startsWith('- '));
   for(const record of records) {
    const path=`/run/author-context/${record.match(/sha256:([0-9a-f]{64})/)[1]}.json`;
    const entry=JSON.parse(r.files.get(path));
    assert.equal(entry.old,true,'other pending entries use their previous best');
    assert.notEqual(entry.id,id,'own previous version is excluded');
    assert.equal(record.includes('under rewrite this round: do not copy its check; keep only names consistent'),pending.includes(entry.id),'each pending sibling is marked, stable siblings are not');
   }
   assert(r.prompts[id].includes('Records marked under rewrite this round are previous versions'));
  }
  runs.push(r);
 }
 assert.notDeepEqual(runs[0].ends.map(e=>e.id),runs[1].ends.map(e=>e.id));
 assert.deepEqual(runs[0].prompts,runs[1].prompts,'rewrite prompts identical across completion orders');
}

// An outage stops new starts when it settles. The cap-3 B-at-10/A-at-20
// schedule and its mirror start different siblings. Keeping every success (the
// previous implementation) fails this next-round pending/prompt equality proof.
async function failureStopsNewStartsAndSettlesSiblings() {
 const criteria={A:'a',B:'b',C:'c',D:'d',E:'e',F:'f'};
 const runs=[],prompts=[];
 for(const aTakes of [5,20]) {
  const durations={A:aTakes,B:10,C:30,D:5,E:5,F:5};
  const state={entries:new Map(),retryIds:null};
  const r=acceptanceRun(1,3,criteria,state,{durations,fails:'B'});
  const out=await r.run;
  assert.equal(out.status,'failed');
  const early=aTakes===5;
  assert.deepEqual(r.starts.map(s=>s.id),early?['A','B','C','D']:['A','B','C']);
  assert(r.starts.every(s=>s.at<10),'no new start after the outage settles');
  assert.equal(r.active(),0,'started siblings settled before return');
  assert.equal(r.now(),30,'returned only after the slowest started sibling');
  assert.deepEqual(Array.from(state.entries.keys()),['A'],'only successes before the lowest operational failure are kept');
  assert.equal(state.added,1);
  assert.equal(state.replaced,0);
  assert.deepEqual(Array.from(state.roundPassed),['A']);
  const retry=['B','C','D','E','F'];
  assert.deepEqual(Array.from(state.retryIds),retry,'all indices from the failure onward are retried');
  assert.equal(state.roundOutage,true);
  const next=acceptanceRun(2,3,criteria,state,{durations,round:2});
  assert.equal((await next.run).status,'accepted');
  assert.deepEqual(Object.keys(next.prompts).sort(),retry);
  runs.push(r); prompts.push(next.prompts);
 }
 for(const id of Object.keys(runs[0].prompts).filter(id=>id in runs[1].prompts))
  assert.equal(runs[0].prompts[id],runs[1].prompts[id],`first-round prompt bytes for ${id}`);
 assert.deepEqual(prompts[0],prompts[1],'every next-round prompt is byte-identical');
}

// The first failure to settle is C, but B is the lowest failed index. Neither
// a refusal nor a successful rewrite after B may change retained state.
async function lowestOperationalFailureIgnoresLaterResults() {
 const criteria={A:'a',B:'b',C:'c',D:'d',E:'e',F:'f',G:'g'};
 const entries=new Map(['D','E'].map(id=>[id,{id,old:true}]));
 const refusals=new Map([['D',['old D refusal']],['E',['old E refusal']]]);
 const state={entries,refusals,retryIds:null};
 const validator=serialized=>JSON.stringify(JSON.parse(serialized).id==='D'?[{text:'new D refusal'}]:[]);
 const r=acceptanceRun(1,3,criteria,state,{fails:['B','C'],malformed:['A'],validator,
  durations:{A:5,B:20,C:10,D:1,E:1,F:30}});
 const out=await r.run;
 assert.equal(out.status,'failed');
 assert.deepEqual(r.starts.map(s=>s.id),['A','B','C','D','E','F']);
 assert.deepEqual(Array.from(out.refusals,x=>x.entryId),['A'],'later refusals are not reported');
 assert.deepEqual([...state.entries.values()],[{id:'D',old:true},{id:'E',old:true}]);
 assert.deepEqual(state.refusals.get('D'),['old D refusal'],'later refusal cannot set a note');
 assert.deepEqual(state.refusals.get('E'),['old E refusal'],'later success cannot clear a note');
 assert(state.refusals.has('A'),'earlier malformed reply still sets its own note');
 assert.equal(state.added,0); assert.equal(state.replaced,0);
 assert.deepEqual(Array.from(state.roundPassed),[]);
 assert.deepEqual(Array.from(state.retryIds),Object.keys(criteria));
}

// Issue 357 round 4 (F3): a shape refusal of one entry keeps the sibling that
// passed beside it, whichever finished first. Refusals do not stop later
// entries; every refusal is returned and only refused entries are retried.
async function refusalKeepsPassedSiblings() {
 const criteria={A:'a',B:'b',C:'c',D:'d'};
 const refusal=id=>[{text:`acceptance entry '${id}' was refused: criterion is missing`,
  deterministic_defect:{provenance:'host_validator',code:'invalid_candidate_shape',subject:'entries/0/criterion',location:'shape',stage:'shape'}}];
 for(const [cap,refused,starts,kept,retry] of [
  [1,['A'],['A','B','C','D'],['B','C','D'],['A']],
  [2,['A'],['A','B','C','D'],['B','C','D'],['A']],
  [3,['A','C'],['A','B','C','D'],['B','D'],['A','C']],
  [3,['B'],['A','B','C','D'],['A','C','D'],['B']],
 ]) {
  const prompts=[];
  for(const bTakes of [5,40]) {
   const state={entries:new Map(),retryIds:null};
   const validator=(serialized)=>JSON.stringify(refused.includes(JSON.parse(serialized).id)?refusal(JSON.parse(serialized).id):[]);
   const r=acceptanceRun(1,cap,criteria,state,{durations:{A:20,B:bTakes,C:10,D:5},validator});
   const out=await r.run;
   assert.equal(out.status,'failed');
   assert.equal(state.roundOutage,false,'shape refusals are author attempts, not outages');
   assert.deepEqual(Array.from(out.refusals,r=>r.entryId),refused,'every refusal of the round is returned, in input order');
   assert.deepEqual(r.starts.map(s=>s.id),starts,`cap ${cap} starts`);
   assert.deepEqual(Array.from(state.entries.keys()),kept,`cap ${cap} refused ${refused} keeps passed siblings`);
   assert.deepEqual(Array.from(state.retryIds),retry,`cap ${cap} refused ${refused} retries only the unfinished`);
   const next=acceptanceRun(2,cap,criteria,state,{durations:{A:20,B:bTakes,C:10,D:5}});
   assert.equal((await next.run).status,'accepted');
   assert.deepEqual(Object.keys(next.prompts).sort(),retry,'a passed sibling is not re-authored');
   prompts.push(next.prompts);
  }
  assert.deepEqual(prompts[0],prompts[1],'retained siblings do not depend on timing');
 }
}

// Refusals never change a sibling's pre-round context or stop new starts.
async function repairableRepliesContinueWithFixedPrompts() {
 const criteria={A:'a',B:'b',C:'c',D:'d',E:'e',F:'f',G:'g'};
 for(const cap of [1,3]) for(const kind of ['refusal','malformed']) {
  const runs=[];
  for(const reverse of [false,true]) {
   const state={entries:new Map([['A',{id:'A',old:true}]]),retryIds:new Set(['B','C','D','E','F','G'])};
   const refused=['B','D'];
   const validator=serialized=>JSON.stringify(refused.includes(JSON.parse(serialized).id)?[{text:'invalid check'}]:[]);
   const durations=reverse?{B:40,C:5,D:20,E:5,F:5,G:5}:{B:5,C:40,D:20,E:5,F:5,G:5};
   const r=acceptanceRun(1,cap,criteria,state,{durations,...(kind==='refusal'?{validator}:{malformed:refused})});
   assert.equal((await r.run).status,'failed');
   const pending=['B','C','D','E','F','G'];
   assert.deepEqual(r.starts.map(s=>s.id),pending);
   assert.deepEqual([...state.entries.keys()],['A','C','E','F','G']);
   assert.deepEqual([...state.retryIds],refused);
   pending.forEach(id=>assert.deepEqual(priorIds(r.prompts[id]),['A'],`${kind} cap ${cap} prior of ${id}`));
   assert.equal(r.active(),0);
   assert(r.peak()<=cap);
   runs.push(r);
  }
  if(cap>1) assert.notDeepEqual(runs[0].ends.map(e=>e.id),runs[1].ends.map(e=>e.id));
  assert.deepEqual(runs[0].prompts,runs[1].prompts,`${kind} cap ${cap}: prompts identical across opposite completion orders`);
 }
}

// A repairable refusal before an outage continues filling available slots.
async function refusalBeforeOperationalFailureKeepsDeterministicState() {
 const criteria={A:'a',B:'b',C:'c',D:'d',E:'e',F:'f',G:'g',H:'h'};
 const runs=[],prompts=[];
 for(const aTakes of [5,40]) {
  const state={entries:new Map(),retryIds:null};
  const validator=serialized=>JSON.stringify(JSON.parse(serialized).id==='A'?[{text:'invalid check'}]:[]);
  const r=acceptanceRun(1,3,criteria,state,{validator,fails:'C',durations:{A:aTakes,B:10,C:20,D:5,E:5,F:5}});
  assert.equal((await r.run).status,'failed');
  const early=aTakes===5;
  assert.deepEqual(r.starts.map(s=>s.id),early?['A','B','C','D','E','F','G','H']:['A','B','C','D','E']);
  assert(r.starts.every(s=>s.at<20),'nothing starts after the outage settles');
  assert.deepEqual([...state.entries.keys()],['B']);
  assert.deepEqual([...state.retryIds],['A','C','D','E','F','G','H']);
  assert.equal(state.roundOutage,true);
  assert.equal(r.active(),0);
  const next=acceptanceRun(2,3,criteria,state,{round:2});
  assert.equal((await next.run).status,'accepted');
  runs.push(r); prompts.push(next.prompts);
 }
 for(const id of Object.keys(runs[0].prompts).filter(id=>id in runs[1].prompts))
  assert.equal(runs[0].prompts[id],runs[1].prompts[id],`first-round prompt bytes for ${id}`);
 assert.deepEqual(prompts[0],prompts[1],'refusal before outage preserves next-round prompt bytes');
}

// A stop raised inside one call (a pause or cancel the host observed) is
// rethrown only after every started sibling has settled: none is abandoned.
async function rejectionSettlesSiblingsBeforeRaising() {
 const criteria={A:'a',B:'b',C:'c',D:'d'};
 const state={entries:new Map(),retryIds:null};
 const r=acceptanceRun(1,3,criteria,state,{durations:{A:50,B:10,C:30,D:5},throws:'B'});
 await assert.rejects(r.run,/stopped at B/);
 assert.equal(r.active(),0,'no sibling abandoned mid-call');
 assert.equal(r.now(),50);
 assert.deepEqual(r.starts.map(s=>s.id),['A','B','C'],'nothing starts after the stop');
 assert.deepEqual([...state.entries.keys()],['A','C'],'successes kept before rethrowing');
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

makespanFollowsTheSchedule().then(slowFirstCallDoesNotBlockTheFifthEntry).then(promptsDependOnlyOnPreRoundState).then(rewritePriorKeepsOtherPreviousBestVersions)
 .then(failureStopsNewStartsAndSettlesSiblings).then(lowestOperationalFailureIgnoresLaterResults).then(refusalKeepsPassedSiblings).then(repairableRepliesContinueWithFixedPrompts).then(refusalBeforeOperationalFailureKeepsDeterministicState).then(rejectionSettlesSiblingsBeforeRaising).then(rejectionOutranksAFailedReply)
 .then(()=>console.log('bounded author pool passed')).catch(e=>{console.error(e);process.exitCode=1});
