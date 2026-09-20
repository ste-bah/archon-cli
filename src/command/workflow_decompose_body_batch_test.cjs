// Body authoring fans out in batches of authorMaxParallelism (Issue-56
// addition): N subjects produce N body author calls with at most batch-size
// in flight, results keyed by frozen file name in skeleton order, frozen
// bodies skipped, and a failing body surfaces only after its siblings settle.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const scriptSource = () => ['workflow_decompose_v1.js','workflow_decompose_v1_acceptance.js','workflow_decompose_v1_set_gate.js'].map(f=>fs.readFileSync(__dirname+'/'+f,'utf8')).join('\n');
const SUBJECTS = Array.from({length:7},(_,i)=>({taskId:`TASK-X-${i+1}`,fileName:`TASK-X-${i+1}.md`}));

function harness(cap, options = {}) {
 const context = {args:{projectRoot:'/p',repositoryRoot:'/r',prdPath:'/p/prd',prdDigest:'x',taskRoot:'/p/tasks',gateMode:'observe',acceptanceCriteria:{'AC-X-1':'c'},authorMaxParallelism:cap,frozenChain:options.frozenChain}, console};
 vm.createContext(context);
 vm.runInContext(scriptSource(),context);
 let active=0,peak=0; const bodyCalls=[],landed=[];
 const w={
  agent:async(id,opts)=>{
   if(!id.startsWith('body-')) return {status:'accepted',stopReason:'end_turn',content:JSON.stringify({id:'AC-X-1'})};
   const taskId=opts.task.match(/host-frozen task_id (TASK-X-\d+)/)[1];
   bodyCalls.push({id,taskId,peakAtStart:active+1}); active++;peak=Math.max(peak,active);
   await new Promise(resolve=>setTimeout(resolve,5)); active--;
   if(options.failing===taskId) throw new Error(`author of ${taskId} exhausted`);
   return {status:'accepted',stopReason:'end_turn',content:`body of ${taskId}`};
  },
  hostCommand:async(cap,opts)=>{
   if(cap==='land-task-body') landed.push(opts.stdin);
   return {publicationReceipt:{call_id:cap},postcondition:{satisfied:true},result:{data:{publicationReceipt:{call_id:cap}}},gateEnvelope:{policy_findings:[]},subjects:SUBJECTS};
  },
  finalReport:async(_,report)=>report
 };
 return {context,w,bodyCalls,landed,peak:()=>peak,active:()=>active};
}

async function boundedBatches() {
 const h=harness(3);
 const report=await h.context.workflow(h.w);
 assert.equal(h.bodyCalls.length,SUBJECTS.length,'one author call per subject');
 assert.equal(h.peak(),3,'at most batch-size bodies in flight, and the batch is full');
 assert.equal(h.active(),0,'every started call settled');
 assert.deepEqual(h.bodyCalls.slice(0,3).map(c=>c.taskId),['TASK-X-1','TASK-X-2','TASK-X-3'],'first batch is the first three subjects');
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
 assert.equal(h.bodyCalls.length,3,'the failing batch ran to completion and no later batch started');
}

boundedBatches().then(sequentialWhenCapIsOne).then(frozenBodiesAreSkipped).then(failureSurfacesAfterSiblingsSettle)
 .then(()=>console.log('bounded body batches passed')).catch(e=>{console.error(e);process.exitCode=1});
