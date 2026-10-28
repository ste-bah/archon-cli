const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { spawnSync } = require('node:child_process');
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
if (!process.argv[2]) {
  for (const scenario of ['body', 'author', 'set-gates', 'set-gates-reply']) {
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
  let calls = 0;
  const w = {
    agent: async () => ({status: 'accepted', stopReason: 'end_turn', content: '{}'}),
    checkpoint: async () => ({}),
    pause: async id => {
      const owner = scenario === 'body' ? 'authored-pending' : scenario === 'author' ? 'subject' : 'set-gates';
      assert.equal(id, `pause-${owner}-1`, 'the stalled owning loop must pause');
      console.log(`${scenario}: paused after ${calls} host calls`);
      throw new Error('progress pause');
    },
    hostCommand: async capability => {
      assert(++calls <= 40, 'host calls exceeded the existing progress windows without a pause');
      if (scenario === 'set-gates-reply' && capability === 'task-set-lint') {
        return {gateEnvelope: {operational_error: {kind: 'authored_judge_reply', text: 'repair assertion'}}};
      }
      return {publicationReceipt: {}, postcondition: {satisfied: true}, subjects: [subject], gateEnvelope: {policy_findings:
        capability === 'requirements-trace' || (scenario.startsWith('set-gates') && capability !== 'task-set-lint') ? [] : [{subject: subject.taskId,
          text: 'repair assertion', remediation_scope: scenario === 'author' ? 'candidate_artifact' : 'body'}]}};
    }
  };
  const action = scenario === 'body'
    ? context.authorBodies(w, [[subject, []]], new Map())
    : scenario === 'author' ? context.authorCandidate(w, {phase: 'subject', capability: 'validate',
      prompt: () => 'author', retryScopes: new Set(['candidate_artifact'])})
      : context.runSetGateLoop(w, {outcome: {}, subjects: [subject]}, new Map());
  assert.rejects(action, /progress pause/).catch(error => { console.error(error); process.exitCode = 1; });
}
