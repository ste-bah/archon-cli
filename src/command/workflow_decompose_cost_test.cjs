const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
async function run(globalFinding = false, structural = false) {
 const criteria = Object.fromEntries(Array.from({length:9},(_,i)=>[`AC-X-${i+1}`,`criterion ${i+1}`]));
 const context = {args:{projectRoot:'/p',prdPath:'/p/prd',prdDigest:'x',taskRoot:'/p/tasks',gateMode:'observe',acceptanceCriteria:criteria,authorMaxParallelism:4}, console};
 vm.createContext(context);
 vm.runInContext(fs.readFileSync(__dirname+'/workflow_decompose_v1.js','utf8'),context);
 let active=0,peak=0,round=0; const calls=[],assembled=[];
 const w={
  agent:async(id,options)=>{
   if(!id.startsWith('acceptance-author-')) return {status:'accepted',stopReason:'end_turn',content:'{}'};
   const key=options.task.match(/Author ONLY entry ([^:]+):/)[1];
   calls.push({key,round,task:options.task}); active++;peak=Math.max(peak,active);
   await new Promise(resolve=>setTimeout(resolve,5)); active--;
   return {status:'accepted',stopReason:'end_turn',content:JSON.stringify({id:key,version:round})};
  },
  hostCommand:async(cap,options)=>{
   const findings=[];
   if(cap==='freeze-acceptance') {
    assembled.push(JSON.parse(options.stdin));round++;
    if(round===1) findings.push({subject:globalFinding || structural?'acceptance-contract':'AC-X-5',text:structural ? "candidate artifact was refused: candidate artifact rejected: check 'AC-X-5': verifier ends with '; true'" : 'repair this check',remediation_scope:'candidate_artifact'});
   }
   return {publicationReceipt:structural && cap==='freeze-acceptance' && round===1 ? null : {call_id:cap},postcondition:{satisfied:true},result:{data:{publicationReceipt:{call_id:cap}}},gateEnvelope:{policy_findings:findings},subjects:[{taskId:'TASK-X-1',fileName:'TASK-X-1.md'}]};
  }, finalReport:async()=>({})
 };
 await context.workflow(w);
 assert.equal(peak,4,'bounded concurrent entry authoring');
 assert.equal(calls.filter(x=>x.round===1).length,globalFinding?9:1,'only faulted entries reauthored unless finding is global');
 if(!globalFinding) {
  assert.equal(calls.filter(x=>x.round===1)[0].key,'AC-X-5');
  assert.equal(assembled[1].entries.find(x=>x.id==='AC-X-1').version,0,'clean entry retained byte-for-byte');
 }
 assert(calls.find(x=>x.key==='AC-X-9'&&x.round===0).task.includes('"AC-X-1","version":0'),'later batch sees earlier entries');
}
async function failedBatch() {
 const context={args:{acceptanceCriteria:{A:'a',B:'b',C:'c',D:'d'},authorMaxParallelism:3},console};
 vm.createContext(context);vm.runInContext(fs.readFileSync(__dirname+'/workflow_decompose_v1.js','utf8'),context);
 const calls={};let fail=true;
 const w={agent:async(_,options)=>{
  const id=options.task.match(/Author ONLY entry ([^:]+):/)[1];calls[id]=(calls[id]||0)+1;
  if(id==='B'&&fail) {fail=false;return {status:'failed',summary:'transport'};}
  return {status:'accepted',stopReason:'end_turn',content:JSON.stringify({id})};
 }};
 const state={entries:new Map(),retryIds:null};
 assert.equal((await context.authorAcceptanceEntries(w,'author',1,state)).status,'failed');
 assert.equal((await context.authorAcceptanceEntries(w,'author',2,state)).status,'accepted');
 assert.deepEqual(calls,{A:1,B:2,C:1,D:1},'completed siblings must not repeat');
}
async function structuralRouting() {
 const ctx={};vm.createContext(ctx);vm.runInContext(fs.readFileSync(__dirname+'/workflow_decompose_v1.js','utf8'),ctx);
 const known=new Set(['A','B','C']);
 const route=text=>ctx.acceptanceRepairIds([{text,subject:'acceptance',remediation_scope:'candidate_artifact'}],known,false);
 assert.deepEqual([...route("candidate artifact was refused: candidate artifact rejected: check 'A': invalid; check 'B': invalid")],['A','B']);
 assert.equal(route("check 'UNKNOWN': invalid"),null);
 assert.equal(route("gap_policy disagrees; check 'A': invalid"),null);
 assert.equal(ctx.acceptanceRepairIds([{text:"check 'A': invalid"},{text:"missing acceptance id"}],known,false),null);
}
run().then(()=>run(true)).then(()=>run(false,true)).then(failedBatch).then(structuralRouting).then(()=>console.log('selective carry-forward and bounded batches passed')).catch(e=>{console.error(e);process.exitCode=1});
