// Body authoring fans out in a bounded pool of authorMaxParallelism (Issue-56
// addition, pool since Issue-247): N subjects produce N body author calls with
// at most the cap in flight, results keyed by frozen file name in skeleton
// order, frozen bodies skipped, and a failing body surfaces only after its
// started siblings settle, with nothing started after it failed.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const scriptSource = () => ['workflow_decompose_v1.js','workflow_decompose_v1_acceptance.js','workflow_decompose_v1_set_gate.js'].map(f=>fs.readFileSync(__dirname+'/'+f,'utf8')).join('\n');
const SUBJECTS = Array.from({length:7},(_,i)=>({taskId:`TASK-X-${i+1}`,fileName:`TASK-X-${i+1}.md`}));

function harness(cap, options = {}) {
 const context = {args:{projectRoot:'/p',repositoryRoot:'/r',prdPath:'/p/prd',prdDigest:'x',taskRoot:'/p/tasks',gateMode:'observe',acceptanceCriteria:{'AC-X-1':'c'},authorMaxParallelism:cap,frozenChain:options.frozenChain}, console};
 vm.createContext(context);
 vm.runInContext(scriptSource(),context);
 let active=0,peak=0,failedAt=null,firstEndedAfterFourthStarted=null; const bodyCalls=[],landed=[];
 const w={
  agent:async(id,opts)=>{
   if(!id.startsWith('body-')) return {status:'accepted',stopReason:'end_turn',content:JSON.stringify({id:'AC-X-1'})};
   const taskId=opts.task.match(/host-frozen task_id (TASK-X-\d+)/)[1];
   bodyCalls.push({id,taskId,peakAtStart:active+1,afterFailure:failedAt!==null}); active++;peak=Math.max(peak,active);
   if(options.holdFirst&&taskId==='TASK-X-1') {
    // Hold the first body until the fourth starts: a pool starts it in the
    // slot a sibling frees; batches and a prefix window hold it behind this
    // call. The bound only ends a wrong scheduler's wait, it is not a timing.
    for(let i=0;i<200&&!bodyCalls.some(c=>c.taskId==='TASK-X-4');i++) await new Promise(resolve=>setTimeout(resolve,1));
    firstEndedAfterFourthStarted=bodyCalls.some(c=>c.taskId==='TASK-X-4');
   } else await new Promise(resolve=>setTimeout(resolve,options.delay?.[taskId] ?? 5));
   active--;
   if([].concat(options.failing).includes(taskId)) { failedAt=bodyCalls.length; throw new Error(`author of ${taskId} exhausted`); }
   return {status:'accepted',stopReason:'end_turn',content:`body of ${taskId}`};
  },
  hostCommand:async(cap,opts)=>{
   if(cap==='land-task-body') landed.push(opts.stdin);
   return {publicationReceipt:{call_id:cap},postcondition:{satisfied:true},result:{data:{publicationReceipt:{call_id:cap}}},gateEnvelope:{policy_findings:[]},subjects:SUBJECTS};
  },
  finalReport:async(_,report)=>report
 };
 return {context,w,bodyCalls,landed,peak:()=>peak,active:()=>active,fourthBeforeFirstEnded:()=>firstEndedAfterFourthStarted};
}

async function boundedPool() {
 const h=harness(3,{holdFirst:true});
 const report=await h.context.workflow(h.w);
 assert.equal(h.bodyCalls.length,SUBJECTS.length,'one author call per subject');
 assert.equal(h.peak(),3,'at most the cap in flight, and the pool is full');
 assert.equal(h.active(),0,'every started call settled');
 assert.equal(h.fourthBeforeFirstEnded(),true,'a freed slot starts the next body before the slowest one ends');
 assert.deepEqual(h.bodyCalls.map(c=>c.taskId),SUBJECTS.map(s=>s.taskId),'bodies start in skeleton order');
 assert.equal(h.landed.length,SUBJECTS.length);
 // Evidence keeps skeleton order: the body inputs follow acceptance and skeleton, one per subject in order.
 const bodyInputs=report.inputs.slice(2,2+SUBJECTS.length);
 assert.equal(bodyInputs.length,SUBJECTS.length,'one evidence entry per body');
}

async function sequentialWhenCapIsOne() {
 const h=harness(1);
 await h.context.workflow(h.w);
 assert.equal(h.bodyCalls.length,SUBJECTS.length);
 assert.equal(h.peak(),1,'cap 1 is the old sequential loop');
}

async function frozenBodiesAreSkipped() {
 const frozen={acceptance:true,skeleton:true,subjects:SUBJECTS,bodies:['TASK-X-2.md','TASK-X-5.md']};
 const h=harness(4,{frozenChain:frozen});
 assert.equal(h.context.frozenChain().bodies.size,2,'fixture frozen chain is read');
 await h.context.workflow(h.w);
 assert.deepEqual(h.bodyCalls.map(c=>c.taskId).sort(),['TASK-X-1','TASK-X-3','TASK-X-4','TASK-X-6','TASK-X-7'],'frozen bodies are not re-authored');
 assert.equal(h.peak(),4);
}

async function failureSurfacesAfterSiblingsSettle() {
 const h=harness(3,{failing:'TASK-X-2'});
 await assert.rejects(()=>h.context.workflow(h.w),/author of TASK-X-2 exhausted/);
 assert.equal(h.active(),0,'no sibling agent abandoned mid-call');
 assert(h.bodyCalls.length>=3&&h.bodyCalls.length<SUBJECTS.length,'started calls ran to completion and the pool stopped');
 assert(h.bodyCalls.every(c=>!c.afterFailure),'nothing starts after a body fails');
}

// Two bodies fail; the later one by index fails first in time. The lowest
// index is raised: neither the last index nor the first to land.
async function lowestFailingBodyIsRaised() {
 const h=harness(3,{failing:['TASK-X-2','TASK-X-3'],delay:{'TASK-X-2':20,'TASK-X-3':1}});
 await assert.rejects(()=>h.context.workflow(h.w),/author of TASK-X-2 exhausted/);
 assert.equal(h.active(),0,'no sibling agent abandoned mid-call');
}

boundedPool().then(sequentialWhenCapIsOne).then(frozenBodiesAreSkipped).then(failureSurfacesAfterSiblingsSettle).then(lowestFailingBodyIsRaised)
 .then(()=>console.log('bounded body pool passed')).catch(e=>{console.error(e);process.exitCode=1});
