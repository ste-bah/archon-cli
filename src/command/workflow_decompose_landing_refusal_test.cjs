const assert = require('node:assert/strict');
const { test } = require('node:test');
const fs = require('node:fs');
const vm = require('node:vm');
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const names = ['workflow_decompose_v1.js', 'workflow_decompose_v1_body.js',
  'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js',
  'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_frozen.js', "workflow_decompose_v1_prompt.js",
  'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_failing_set.js',
  'workflow_decompose_v1_seed.js', 'workflow_decompose_v1_context.js'];
const source = names.map(name => fs.readFileSync(`${__dirname}/${name}`, 'utf8')).join('\n');
const skeleton = repaired => JSON.stringify({ schema_version: 1, acceptance_digest: '',
  tasks: Array.from({ length: 19 }, (_, i) => ({ task_id: `TASK-X-${String(i+1).padStart(3,'0')}`,
    file_name: `TASK-X-${String(i+1).padStart(3,'0')}.md`, implements: ['REQ-1'],
    source_sections: [repaired && i < 14 ? '2' : '1'] })) });

for (const retained of [true, false]) {
  for (const refusal of ['Syntax at line 1', 'missing required field']) {
    test(`artifact refusal keeps authoritative findings and the prior judged candidate: ${refusal}`, async () => {
      const ctx = withAuthorContext({ args: { gateMode: 'enforce' } });
      vm.createContext(ctx);
      vm.runInContext(source, ctx);
      const findings = Array.from({ length: 9 }, (_, i) => `repair task ${i+1}`);
      const options = [];
      let freezes = 0;
      const candidate = skeleton(false), repaired = skeleton(true);
      const clean = { publicationReceipt: { call_id: 'freeze' }, postcondition: { satisfied: true },
        subjects: [], gateEnvelope: { policy_findings: [] } };
      await ctx.authorCandidate({
        agent: async (_id, opts) => {
          options.push(opts);
          return { status: 'accepted', stopReason: 'end_turn',
            retainedLandings: options.length !== 2 || retained,
            content: options.length === 1 ? candidate : options.length === 2 && !retained ? 'Unusable reply.' : repaired };
        },
        hostCommand: async (_capability, input) => {
          freezes++;
          if (freezes !== 2 || retained) assert.equal(JSON.parse(input.stdin).tasks.length, 19);
          else assert.throws(() => JSON.parse(input.stdin));
          if (freezes === 1) return { gateEnvelope: { policy_findings:
            findings.map(text => ({ text, remediation_scope: 'skeleton' })) } };
          if (freezes === 2) return { gateEnvelope: { policy_findings:
            [{ text: refusal, remediation_scope: 'candidate_artifact' }] } };
          return clean;
        },
        pause: async () => { throw Error('unexpected pause'); }
      }, { phase: 'skeleton', capability: 'freeze-skeleton', prompt: () => 'Author records',
        retryScopes: new Set(['skeleton', 'candidate_artifact']) });
      assert.equal(options.length, 3);
      assert.equal(options[2].previousSkeleton, candidate);
      for (const finding of findings) {
        assert.ok(options[2].task.includes(finding));
        assert.ok(options[2].openFindings.includes(finding));
      }
      assert.ok(options[2].task.includes(refusal));
      assert.ok(!options[2].task.includes('not one to fix now'));
    });
  }
}

test('invalid landing answers remain authored feedback and keep the gate findings', async () => {
  const ctx = withAuthorContext({ args: { gateMode: 'enforce' } });
  vm.createContext(ctx);
  vm.runInContext(source, ctx);
  const options = [];
  let freezes = 0;
  await ctx.authorCandidate({
    agent: async (_id, opts) => {
      options.push(opts);
      return options.length === 2
        ? { status: 'failed', invalid_answer: true, summary: 'repair has zero new landings' }
        : { status: 'accepted', stopReason: 'end_turn', retainedLandings: true, content: skeleton(false) };
    },
    hostCommand: async () => ++freezes === 1
      ? { gateEnvelope: { policy_findings: [{ text: 'repair task 1', remediation_scope: 'skeleton' }] } }
      : { publicationReceipt: {}, postcondition: { satisfied: true }, subjects: [], gateEnvelope: { policy_findings: [] } },
    pause: async () => { throw Error('authored malformed reply must not operationally pause'); }
  }, { phase: 'skeleton', capability: 'freeze-skeleton', prompt: () => 'Author records', retryScopes: new Set(['skeleton', 'candidate_artifact']) });
  assert.equal(options.length, 3);
  assert.ok(options[2].task.includes('repair has zero new landings'));
  assert.ok(options[2].openFindings.includes('repair task 1'));
});

test('retained landing provenance survives replay after a gate outage', async () => {
  const ctx = withAuthorContext({ args: { gateMode: 'enforce' } });
  vm.createContext(ctx);
  vm.runInContext(source, ctx);
  const options = [];
  let freezes = 0;
  const candidate = skeleton(true);
  await ctx.authorCandidate({
    agent: async (_id, opts) => {
      options.push(opts);
      return { status: 'accepted', stopReason: 'end_turn', content: candidate, retainedLandings: true };
    },
    hostCommand: async () => {
      freezes++;
      if (freezes === 1) return { gateEnvelope: { policy_findings: [], operational_error: { kind: 'provider', message: 'gate unavailable' } } };
      if (freezes === 2) return { gateEnvelope: { policy_findings: [{ text: 'Syntax refusal', remediation_scope: 'candidate_artifact' }] } };
      return { publicationReceipt: {}, postcondition: { satisfied: true }, subjects: [], gateEnvelope: { policy_findings: [] } };
    },
    pause: async () => { throw Error('unexpected pause'); }
  }, { phase: 'skeleton', capability: 'freeze-skeleton', prompt: () => 'Author records', retryScopes: new Set(['skeleton', 'candidate_artifact']) });
  assert.equal(options.length, 2);
  assert.equal(options[1].previousSkeleton, null);
});

test('resumed skeleton seed gives retained tasks and all nine findings to the next author', async () => {
  const findings = Array.from({ length: 9 }, (_, i) => ({ text: `repair task ${i+1}`, remediation_scope: 'skeleton' }));
  const candidate = skeleton(true);
  const ctx = withAuthorContext({ args: { gateMode: 'enforce', phaseSeed: { subjects: {
    skeleton: { kind: 'artifact', candidate, candidate_call: 'skeleton-author-4', gate: { findings } }
  } } } });
  vm.createContext(ctx);
  vm.runInContext(source, ctx);
  let options;
  await ctx.authorCandidate({
    agent: async (_id, opts) => {
      options = opts;
      return { status: 'accepted', stopReason: 'end_turn', content: candidate, retainedLandings: true };
    },
    hostCommand: async () => ({ publicationReceipt: {}, postcondition: { satisfied: true }, subjects: [], gateEnvelope: { policy_findings: [] } }),
    pause: async () => { throw Error('unexpected pause'); }
  }, { phase: 'skeleton', capability: 'freeze-skeleton', prompt: () => 'Author records', retryScopes: new Set(['skeleton', 'candidate_artifact']) });
  assert.equal(options.previousSkeleton, candidate);
  assert.equal(options.openFindings.length, 9);
  for (const finding of findings) assert.ok(options.task.includes(finding.text));
});

test('landing contract refusal carries retained tasks into the next author call', async () => {
  for (const summary of ['zero byte-changing landings', 'prose reply', 'bad receipt']) {
    const ctx = withAuthorContext({ args: { gateMode: 'enforce' } });
    vm.createContext(ctx);
    vm.runInContext(source, ctx);
    const options = [];
    const retained = skeleton(true);
    let freezes = 0;
    await ctx.authorCandidate({
      agent: async (_id, opts) => {
        options.push(opts);
        return options.length === 2
          ? { status: 'failed', invalid_answer: true, summary, content: retained, retainedLandings: true }
          : { status: 'accepted', stopReason: 'end_turn', retainedLandings: true, content: skeleton(false) };
      },
      hostCommand: async () => ++freezes === 1
        ? { gateEnvelope: { policy_findings: [{ text: 'repair task 1', remediation_scope: 'skeleton' }] } }
        : { publicationReceipt: {}, postcondition: { satisfied: true }, subjects: [], gateEnvelope: { policy_findings: [] } },
      pause: async () => { throw Error('unexpected pause'); }
    }, { phase: 'skeleton', capability: 'freeze-skeleton', prompt: () => 'Author records',
      retryScopes: new Set(['skeleton', 'candidate_artifact']) });
    assert.equal(options.length, 3);
    assert.equal(options[2].previousSkeleton, retained);
    assert.ok(options[2].task.includes(summary));
    assert.ok(options[2].openFindings.includes('repair task 1'));
    assert.equal(freezes, 2);
  }
});

test('resume context keeps authoritative findings alongside an artifact refusal', () => {
  const candidate = skeleton(true);
  const gate = { findings: [
    { text: 'repair task 1', remediation_scope: 'skeleton' },
    { text: 'bad receipt', remediation_scope: 'candidate_artifact' }
  ] };
  const ctx = withAuthorContext({ args: { phaseSeed: { subjects: {
    skeleton: { kind: 'artifact', candidate, gate }
  } } } });
  vm.createContext(ctx);
  vm.runInContext(source, ctx);
  assert.equal(ctx.skeletonPreviousCandidate(ctx.skeletonPolicy([]), candidate, {}), candidate);
  gate.findings.shift();
  assert.equal(ctx.skeletonPreviousCandidate(ctx.skeletonPolicy([]), candidate, {}), undefined);
});
