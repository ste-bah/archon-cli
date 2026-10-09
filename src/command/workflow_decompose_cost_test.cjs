const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const scriptRoot = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const scriptSource = () => ['workflow_decompose_v1.js','workflow_decompose_v1_acceptance.js','workflow_decompose_v1_set_gate.js','workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js'].map(f=>fs.readFileSync(scriptRoot+'/'+f,'utf8')).join('\n');
async function run(globalFinding = false, structural = false, refusal = null) {
 const criteria = Object.fromEntries(Array.from({length:9},(_,i)=>[`AC-X-${i+1}`,`criterion ${i+1}`]));
 const context = {args:{projectRoot:'/p',repositoryRoot:'/r',prdPath:'/p/prd',prdDigest:'x',taskRoot:'/p/tasks',gateMode:'observe',acceptanceCriteria:criteria,authorMaxParallelism:4}, console};
 context.__archonValidateAcceptanceEntry = () => '[]';
 vm.createContext(withAuthorContext(context));
 vm.runInContext(scriptSource(),context);
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
    if(round===1) findings.push({subject:globalFinding || structural?'acceptance-contract':'AC-X-5',text:refusal || (structural ? "candidate artifact was refused: candidate artifact rejected: check 'AC-X-5': verifier ends with '; true'" : 'repair this check'),remediation_scope:'candidate_artifact'});
   }
   return {publicationReceipt:(structural || refusal) && cap==='freeze-acceptance' && round===1 ? null : {call_id:cap},postcondition:{satisfied:true},result:{data:{publicationReceipt:{call_id:cap}}},gateEnvelope:{policy_findings:findings},subjects:[{taskId:'TASK-X-1',fileName:'TASK-X-1.md'}]};
  }, finalReport:async()=>({})
 };
 await context.workflow(w);
 assert.equal(peak,4,'bounded concurrent entry authoring (pool full)');
 assert.equal(calls.filter(x=>x.round===1).length,globalFinding?9:1,'only faulted entries reauthored unless finding is global');
 if(!globalFinding) {
  assert.equal(calls.filter(x=>x.round===1)[0].key,'AC-X-5');
  assert.equal(assembled[1].entries.find(x=>x.id==='AC-X-1').version,0,'clean entry retained byte-for-byte');
 }
 assert(/\n- AC-X-1 sha256:[0-9a-f]{64} /.test(calls.find(x=>x.key==='AC-X-9'&&x.round===0).task),'an entry past the window sees the entries before it (Issue 288: one record line each)');
}
async function failedEntry() {
 const context={args:{acceptanceCriteria:{A:'a',B:'b',C:'c',D:'d'},authorMaxParallelism:3},console};
 context.__archonValidateAcceptanceEntry = () => '[]';
 vm.createContext(withAuthorContext(context));vm.runInContext(scriptSource(),context);
 const calls={};let fail=true;
 const w={agent:async(_,options)=>{
  const id=options.task.match(/Author ONLY entry ([^:]+):/)[1];calls[id]=(calls[id]||0)+1;
  if(id==='B'&&fail) {fail=false;return {status:'failed',summary:'transport'};}
  return {status:'accepted',stopReason:'end_turn',content:JSON.stringify({id})};
 }};
 const state={entries:new Map(),retryIds:null};
 assert.equal((await context.authorAcceptanceEntries(w,'author',1,state)).status,'failed');
 assert.equal((await context.authorAcceptanceEntries(w,'author',2,state)).status,'accepted');
 // Every entry that succeeded is kept; only the failure is retried (Issue
 // 357). D's fixed prefix (A) holds no failure, so it starts whatever the
 // timing of B's failure, and its success is kept too.
 assert.deepEqual(calls,{A:1,B:2,C:1,D:1},'every entry that succeeded is kept');
}
async function structuralRouting() {
 const ctx={};vm.createContext(withAuthorContext(ctx));vm.runInContext(scriptSource(),ctx);
 const known=new Set(['A','B','C']);
 const route=text=>ctx.acceptanceRepairIds([{text,subject:'acceptance',remediation_scope:'candidate_artifact'}],known,false);
 assert.deepEqual([...route("candidate artifact was refused: candidate artifact rejected: check 'A': invalid; check 'B': invalid")],['A','B']);
 assert.equal(route("check 'UNKNOWN': invalid"),null);
 assert.equal(route("gap_policy disagrees; check 'A': invalid"),null);
 assert.equal(ctx.acceptanceRepairIds([{text:"check 'A': invalid"},{text:"missing acceptance id"}],known,false),null);
}
// Issue 357: the stub refuses what the native validator refuses for an
// author-owned field (check/command); the host-owned criterion is set first.
const commandRefusal=entry=>JSON.stringify(typeof entry.check?.command === 'string' ? [] : [{
 text:"acceptance entry 'A' was refused: check/command is missing or has an invalid type or value",
 deterministic_defect:{provenance:'host_validator',code:'invalid_candidate_shape',
  subject:'entries/0/check/command',location:'shape',stage:'shape'},
}]);
async function invalidAuthor(command, envelope = false) {
 const ctx={args:{acceptanceCriteria:{A:'a'},authorMaxParallelism:1,gateMode:'enforce'}};
 let validations=0;
 // Mock only the native boundary. The Rust shape corpus tests the validator;
 // these tests prove the author uses its refusal rather than accepting an id.
 ctx.__archonValidateAcceptanceEntry = (_, serialized) => {validations++; return commandRefusal(JSON.parse(serialized));};
 vm.createContext(withAuthorContext(ctx));vm.runInContext(scriptSource(),ctx);
 const state={entries:new Map(),retryIds:null};let calls=0;
 const check={kind:'command',cwd:'project_root',...command};
 const w={agent:async()=>{
  calls++;
  const entry={id:'A',criterion:'',check};
  return {status:'accepted',stopReason:'end_turn',content:JSON.stringify(envelope?{acceptance:[entry]}:entry)};
 }};
 const result=await ctx.authorAcceptanceEntries(w,'author',1,state);
 assert.equal(result.status,'failed','invalid shape is refused before freeze');
 assert.equal(result.malformed,true);
 assert.match(result.summary,/check\/command/);
 assert.equal(state.entries.size,0);
 assert.deepEqual([...state.retryIds],['A']);
 assert.equal(validations,1,'exactly one host shape validation at author acceptance');
 check.command='test -f output';
 assert.equal((await ctx.authorAcceptanceEntries(w,'repair',2,state)).status,'accepted');
 assert.equal(calls,2);
 const prompts=[];let freezes=0;
 await ctx.authorCandidate({agent:async(_,options)=>{
  prompts.push(options.task);
  return {status:'accepted',stopReason:'end_turn',content:JSON.stringify({id:'A',criterion:'',check:{kind:'command',cwd:'project_root',...(prompts.length===1?command:{command:'test -f output'})}})};
 },hostCommand:async()=>{
  freezes++;
  return {publicationReceipt:{call_id:'freeze'},postcondition:{satisfied:true},gateEnvelope:{policy_findings:[]}};
 }}, {phase:'acceptance',author:ctx.authorAcceptanceEntries,capability:'freeze-acceptance',prompt:()=> 'author',retryScopes:new Set(['candidate_artifact'])});
 assert.equal(prompts.length,2,'repair stays at its author call');
 assert.match(prompts[1],/check\/command is missing or has an invalid type or value/);
 assert.equal(freezes,1,'only the repaired entry reaches freeze');
 // A clean round leaves nothing to author (Issue 362); name A to re-author it.
 state.retryIds=new Set(['A']);
 ctx.__archonValidateAcceptanceEntry = () => {throw new Error('validator fault');};
 await assert.rejects(ctx.authorAcceptanceEntries(w,'repair',3,state),/validator fault/);
 delete ctx.__archonValidateAcceptanceEntry;
 await assert.rejects(ctx.authorAcceptanceEntries(w,'repair',4,state),/__archonValidateAcceptanceEntry is not defined/);
}
// Issue 357 round 5 (P3-A): the host owns the criterion and sets it before
// validation, so a missing or mistyped model criterion costs no repair call.
async function hostCriterion(value, envelope = false) {
 const ctx={args:{acceptanceCriteria:{A:'the host criterion'},authorMaxParallelism:1,gateMode:'enforce'}};
 const seen=[];
 ctx.__archonValidateAcceptanceEntry = (_, serialized) => {
  const entry=JSON.parse(serialized);seen.push(entry.criterion);
  return JSON.stringify(typeof entry.criterion === 'string' ? [] : [{text:'criterion invalid',
   deterministic_defect:{provenance:'host_validator',code:'invalid_candidate_shape',subject:'entries/0/criterion',location:'shape',stage:'shape'}}]);
 };
 vm.createContext(withAuthorContext(ctx));vm.runInContext(scriptSource(),ctx);
 const state={entries:new Map(),retryIds:null};let calls=0;
 const w={agent:async()=>{
  calls++;
  const entry={id:'A',check:{kind:'command',command:'test -f output',cwd:'project_root'},...value};
  return {status:'accepted',stopReason:'end_turn',content:JSON.stringify(envelope?{acceptance:[entry]}:entry)};
 }};
 assert.equal((await ctx.authorAcceptanceEntries(w,'author',1,state)).status,'accepted');
 assert.equal(calls,1,'no repair call for a host-owned field');
 assert.deepEqual(seen,['the host criterion'],'the validator judges the host criterion');
 assert.equal(state.entries.get('A').criterion,'the host criterion');
}
async function supplementaryPointer() {
 const ctx={args:{acceptanceCriteria:{A:'a'},prdRequirementTexts:{'REQ-X':'x'},authorMaxParallelism:1},__archonValidateAcceptanceEntry:()=> '[]'};
 vm.createContext(withAuthorContext(ctx));vm.runInContext(scriptSource(),ctx);
 ctx.owedSupplementary().set('SUP-REQ-X',{requirement:'REQ-X',text:'x'});
 const candidate={entries:[{id:'A'}],supplementary:[{id:'SUP-REQ-X'}]};
 const route=texts=>ctx.acceptanceRepairIds(texts.map(text=>({text})),new Set(['A']),false,candidate);
 const state={entries:new Map([['A',{id:'A'}],['SUP-REQ-X',{id:'SUP-REQ-X'}]]),retryIds:null};
 state.retryIds=ctx.acceptanceRepairIds([{text:'candidate artifact was refused: supplementary/0/criterion is missing or has an invalid type or value'}],new Set(['A']),false,candidate);
 const retained=state.entries.get('A');const calls=[];
 await ctx.authorAcceptanceEntries({agent:async(_,options)=>{
  const id=options.task.match(/Author ONLY entry ([^:]+):/)[1];calls.push(id);
  return {status:'accepted',stopReason:'end_turn',content:JSON.stringify({id})};
 }},'repair',2,state);
 assert.deepEqual(calls,['SUP-REQ-X']);
 assert.equal(state.entries.get('A'),retained,'healthy sibling retained unchanged');
 assert.deepEqual([...route(['entries/0/check/command invalid','supplementary/0/criterion invalid'])],['A','SUP-REQ-X']);
 for(const text of ['invalid document','entries/99/criterion invalid','entries/-1/criterion invalid','entries/0suffix invalid']) {
  assert.equal(route([text]),null,'unresolved refusal keeps full retry');
 }
 assert.equal(route(['supplementary/0/criterion invalid','gap_policy invalid']),null,'global defect still requires full retry');
}
const tests=[
 ['existing selective retry',()=>run()],['existing global retry',()=>run(true)],
 ['existing structural retry',()=>run(false,true)],['failed entry',failedEntry],['structural routing',structuralRouting],
 ['author missing command',()=>invalidAuthor({})],['author null command',()=>invalidAuthor({command:null})],
 ['author numeric command in wrapper',()=>invalidAuthor({command:42},true)],
 ['host criterion: missing',()=>hostCriterion({})],['host criterion: null',()=>hostCriterion({criterion:null})],
 ['host criterion: numeric in wrapper',()=>hostCriterion({criterion:42},true)],
 ['host criterion: model text replaced',()=>hostCriterion({criterion:'model text'})],
 ['entries pointer retry and unlocated full retry',async()=>{await run(false,false,'candidate artifact was refused: entries/4/criterion is missing or has an invalid type or value');await run(true,false,'candidate artifact was refused: invalid document');}],
 ['nested entries pointer retry',()=>run(false,false,'candidate artifact was refused: /entries/4/check/cwd is invalid')],
 ['supplementary pointer retry',supplementaryPointer],
 ...require('./workflow_decompose_shape_progress_test.cjs'),
];
(async()=>{
 let failed=0;
 for(const [name,test] of tests) {
  try {await test();console.log(`PASS ${name}`);}
  catch(e) {failed++;console.error(`FAIL ${name}: ${e.message}`);}
 }
 process.exitCode=failed?1:0;
})();
