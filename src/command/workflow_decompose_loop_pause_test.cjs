const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { spawnSync } = require('node:child_process');
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
if (!process.argv[2]) {
  for (const scenario of ['body', 'author', 'set-gates', 'set-gates-reply', 'changed-bytes', 'changing_unowned_findings_pause_at_the_existing_attempt_window', 'changed-body-bytes', 'normalized-findings', 'oscillating-findings']) {
    const child = spawnSync(process.execPath, [__filename, scenario], {timeout: 5000, encoding: 'utf8'});
    assert.equal(child.status, 0, `${scenario}: ${child.error || child.stderr}`);
    console.log(`PASS ${scenario}`);
  }
} else {
  const scenario = process.argv[2];
  const context = withAuthorContext({args: {gateMode: 'enforce', taskRoot: '/subjects'}});
  vm.createContext(context);
  const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
  vm.runInContext(['v1', 'reply_blocks', 'v1_acceptance', 'v1_set_gate', 'v1_frozen', 'v1_progress', 'v1_seed', 'v1_context']
    .map(name => fs.readFileSync(`${root}/workflow_decompose_${name}.js`, 'utf8')).join('\n'), context);
  const subject = {taskId: 'TASK-X-001', fileName: 'TASK-X-001.md'};
  let calls = 0, authors = 0;
  const rungs = [], prompts = [];
  const progressing = false;
  const w = {
    agent: async (_id, options) => { prompts.push(options.task); return {status: 'accepted', stopReason: 'end_turn', content: scenario.endsWith('bytes') ? String(++authors) : '{}'}; },
    checkpoint: async (_id, options) => { rungs.push(options.task); },
    pause: async id => {
      const owner = (scenario === 'body' || scenario === 'changed-body-bytes') ? 'authored-pending' : (scenario === 'author' || !scenario.startsWith('set-gates')) ? 'subject' : 'set-gates';
      assert.equal(progressing, false, "progress during a cycle must never pause");
      if (scenario !== "set-gates-reply") assert(rungs.some(text => text.includes("rung r4")) || prompts.some(text => text.includes("Repair operation r4")), "repair rungs must fit inside the attempt window");
      if (scenario === "author" || scenario === "changing_unowned_findings_pause_at_the_existing_attempt_window") assert.equal(calls, 4, "the existing attempt window must pause");
      assert.equal(id, `pause-${owner}-1`, 'the stalled owning loop must pause');
      console.log(`${scenario}: paused after ${calls} host calls`);
      throw new Error('progress pause');
    },
    hostCommand: async capability => {
      assert(++calls <= 100, 'host calls exceeded the existing progress windows without a pause');
      if (scenario === 'set-gates-reply' && capability === 'task-set-lint') {
        return {gateEnvelope: {operational_error: {kind: 'authored_judge_reply', text: 'repair assertion'}}};
      }
      if (progressing && calls > 30) return {publicationReceipt: {}, postcondition: {satisfied: true}, gateEnvelope: {policy_findings: []}};
      return {publicationReceipt: {}, postcondition: {satisfied: true}, subjects: [subject], gateEnvelope: {policy_findings:
        capability === 'requirements-trace' || (scenario.startsWith('set-gates') && capability !== 'task-set-lint') ? [] : [{subject: scenario === "changing_unowned_findings_pause_at_the_existing_attempt_window" ? undefined : subject.taskId,
          text: scenario === 'changing_unowned_findings_pause_at_the_existing_attempt_window' ? `assertion ${String.fromCharCode(97 + Math.floor(calls / 26), 97 + calls % 26)}` : scenario === 'normalized-findings' ? `ASSERTION: TASK-X-${calls}   absent!` : scenario === 'oscillating-findings' ? `assertion ${Math.floor(calls / 9) % 2 ? 'alpha' : 'beta'}` : 'repair assertion', remediation_scope: (!scenario.startsWith('set-gates') && scenario !== 'body' && scenario !== 'changed-body-bytes') ? 'candidate_artifact' : 'body'}]}};
    }
  };
  const action = (scenario === 'body' || scenario === 'changed-body-bytes')
    ? context.authorBodies(w, [[subject, []]], new Map())
    : (!scenario.startsWith('set-gates')) ? context.authorCandidate(w, {phase: 'subject', capability: 'validate',
      prompt: () => 'author', retryScopes: new Set(['candidate_artifact'])})
      : context.runSetGateLoop(w, {outcome: {}, subjects: [subject]}, new Map());
  (progressing ? action.then(() => assert.equal(calls, 31)) : assert.rejects(action, /progress pause/)).catch(error => { console.error(error); process.exitCode = 1; });
}
