const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['v1', 'v1_body', 'reply_blocks', 'v1_acceptance', 'v1_set_gate',
  'v1_frozen', 'v1_progress', 'v1_failing_set', 'v1_seed', 'v1_context']
  .map(name => fs.readFileSync(`${root}/workflow_decompose_${name}.js`, 'utf8')).join('\n');
const subject = { taskId: 'TASK-SAMPLE-001', fileName: 'TASK-SAMPLE-001.md' };
const phase = `body-${subject.taskId}`;
const reply = content => ({ status: 'accepted', stopReason: 'end_turn', content });
const findings = (count, label) => Array.from({ length: count }, (_, i) => ({
  text: `${label} finding ${i}`, remediation_scope: 'body', subject: subject.taskId,
  deterministic_defect: { provenance: 'host_validator', code: 'repository_claim', location: `${i}` }
}));
const outcome = found => ({ publicationReceipt: { call_id: 'published' },
  postcondition: { satisfied: true }, gateEnvelope: { policy_findings: found } });
function context() {
  const ctx = withAuthorContext({ args: { gateMode: 'enforce', taskRoot: '/tasks',
    projectRoot: '/project', repositoryRoot: '/repo', prdPath: '/input', acceptanceCriteria: {} } });
  vm.createContext(ctx); vm.runInContext(source, ctx);
  return ctx;
}
function policy(ctx) { return ctx.bodyPolicy(subject, []); }
function retained(ctx) { return vm.runInContext(`BODY_CANDIDATES.get('${phase}')`, ctx); }
function details(prompt) {
  const json = prompt.split('Exact gate findings (JSON):\n')[1]?.split('\n')[0];
  assert.ok(json, 'repair contains structured findings');
  const body = prompt.split('Retained candidate (raw text):\n')[1]?.split('\nEnd retained candidate.')[0];
  return { body, ...JSON.parse(json) };
}

// Replay is the production persistence protocol: requests and results are in
// the existing run result store. A fresh VM reconstructs all indexes from it.
async function trajectory(recorded = [], interrupt = false) {
  const ctx = context(), journal = [], prompts = [];
  let cursor = 0, call = 0;
  async function dispatch(method, id, options, result) {
    const request = JSON.parse(JSON.stringify({ method, id, options }));
    if (cursor < recorded.length) {
      const saved = recorded[cursor++];
      assert.deepEqual(request, saved.request, 'resumed requests preserve exact bytes');
      journal.push(saved);
      return JSON.parse(JSON.stringify(saved.result));
    }
    const saved = { request, result };
    journal.push(saved);
    return result;
  }
  const w = {
    checkpoint: (id, options) => dispatch('checkpoint', id, options, {}),
    agent: async (id, options) => {
      call++;
      if (interrupt && call === 5) throw new Error('interrupted');
      prompts.push(options.task);
      return dispatch('agent', id, options, reply(`body-${call}\nunchanged line\n`));
    },
    hostCommand: (id, options) => dispatch('hostCommand', id, options,
      outcome(findings([39, 59, 5, 33, 0][call - 1], `attempt-${call}`)))
  };
  if (interrupt) await assert.rejects(ctx.authorCandidate(w, policy(ctx)), /interrupted/);
  else await ctx.authorCandidate(w, policy(ctx));
  return { ctx, journal, prompts };
}

test('39 -> 59 -> 5 -> 33 repairs from the five-finding body and accepts only the clean gate', async () => {
  const run = await trajectory();
  assert.equal(run.prompts.length, 5, 'findings block acceptance even with a publication receipt');
  assert.equal(details(run.prompts[2]).body, 'body-1\nunchanged line\n', '59 cannot replace 39');
  const best = details(run.prompts[4]);
  assert.equal(best.body, 'body-3\nunchanged line\n');
  assert.deepEqual(best.findings, findings(5, 'attempt-3'));
  assert.equal(best.blocking_findings, 5);
  assert.match(run.prompts[1], /Change only what its findings require and keep every other line exactly/);
  assert.match(run.prompts[2], /Change only what its findings require/);
  assert.doesNotMatch(run.prompts[1] + run.prompts[2], /fresh context/);
  assert.match(run.prompts[3], /fresh context.*Previously seen defects/);
  assert.doesNotMatch(run.prompts[3], /Retained candidate|unchanged line|bounded-edit attempts failed/);
  assert.match(run.prompts[2], /Prioritize this group first/);
});

test('resume reconstructs the best rejected bytes, findings and score from durable calls', async () => {
  const interrupted = await trajectory([], true);
  assert.equal(retained(interrupted.ctx).score, 5);
  const resumed = await trajectory(interrupted.journal);
  const continuous = await trajectory();
  assert.deepEqual(resumed.journal, continuous.journal);
  assert.equal(details(resumed.prompts[4]).body, 'body-3\nunchanged line\n');
});

test('ties prefer newer bytes; no score bypasses an unsatisfied gate postcondition', async () => {
  const ctx = context();
  ctx.retainBodyCandidate(phase, 'older', findings(5, 'older'));
  ctx.retainBodyCandidate(phase, 'newer', findings(5, 'newer'));
  assert.equal(retained(ctx).body, 'newer');
  let calls = 0;
  await assert.rejects(ctx.authorCandidate({
    agent: async () => { calls++; return reply('candidate'); },
    hostCommand: async () => ({ ...outcome([]), postcondition: { satisfied: false } })
  }, policy(ctx)), /authoritative postcondition is not satisfied/);
  assert.equal(calls, 1);
  assert.equal(retained(ctx).body, 'newer', 'failed publication cannot replace retained bytes');
});

test('large candidate uses exact host-written context; a failed write pauses and logs', async () => {
  const ctx = context(), body = 'unchanged\n'.repeat(4000), found = findings(5, 'large');
  ctx.retainBodyCandidate(phase, body, found);
  let prompt;
  await ctx.authorCandidate({
    agent: async (_, opts) => { prompt = opts.task; return reply('corrected'); },
    hostCommand: async () => outcome([])
  }, policy(ctx));
  const path = prompt.match(/at (\/run\/author-context\/[^ ]+)/)[1];
  assert.equal(ctx.__archonAuthorContext.files.get(path), body);
  assert.deepEqual(details(prompt).findings, found);
  assert.ok(Buffer.byteLength(prompt) < 20000);
  const broken = context(), logs = [];
  broken.retainBodyCandidate(phase, body + 'different', found);
  broken.__archonAuthorContext = () => { throw new Error('disk unavailable'); };
  await assert.rejects(broken.authorCandidate({
    checkpoint: async (_, opts) => logs.push(opts.task),
    agent: async () => { throw new Error('must not dispatch'); },
    pause: async (_, evidence) => {
      assert.equal(evidence.reason, 'operational_no_progress');
      throw new Error('paused context outage');
    }
  }, policy(broken)), /paused context outage/);
  assert.match(logs[0], /retained body candidate context failed.*disk unavailable/);
});

test('set-gate repair starts from the published candidate with exact new findings', async () => {
  const ctx = context(), prompts = [];
  ctx.publishBodyCandidate(phase, 'published body\n');
  const setFindings = findings(5, 'set');
  let round = 0;
  await ctx.runSetGateLoop({
    checkpoint: async () => ({}),
    agent: async (_, opts) => { prompts.push(opts.task); return reply('corrected body'); },
    hostCommand: async cap => {
      if (cap === 'task-set-lint') return outcome(++round === 1 ? setFindings : []);
      return outcome([]);
    }
  }, { subjects: [subject] }, new Map());
  assert.equal(prompts.length, 1);
  assert.deepEqual(details(prompts[0]), { body: 'published body\n', findings: setFindings, blocking_findings: 5 });
});

test('skeleton reauthor drops changed subjects and preserves the candidate of unchanged subjects', async () => {
  const ctx = context();
  const other = { taskId: 'TASK-SAMPLE-002', fileName: 'TASK-SAMPLE-002.md' };
  ctx.changedSubject = subject; ctx.otherSubject = other;
  vm.runInContext(`
    globalThis.priorOutcome = {};
    globalThis.nextOutcome = { subjects: [changedSubject, otherSubject] };
    SKELETON_TASKS.set(priorOutcome, new Map([[changedSubject.taskId, 'old'], [otherSubject.taskId, 'same']]));
    SKELETON_TASKS.set(nextOutcome, new Map([[changedSubject.taskId, 'changed'], [otherSubject.taskId, 'same']]));
    authorCandidate = async () => nextOutcome;
    changedDependencySubjects = () => new Set();
    globalThis.work = [];
    authorBodies = async (_, entries) => { work.push(...entries); };
  `, ctx);
  ctx.retainBodyCandidate(phase, 'old contract body', findings(5, 'old'));
  ctx.retainBodyCandidate(`body-${other.taskId}`, 'unchanged contract body', findings(5, 'other'));
  const bodies = new Map([[subject.fileName, outcome([])], [other.fileName, outcome([])]]);
  await ctx.reauthorSkeleton({}, { outcome: ctx.priorOutcome, subjects: [subject, other] }, bodies, [], [], 2);
  assert.equal(retained(ctx), undefined);
  assert.equal(vm.runInContext(`BODY_CANDIDATES.get('body-${other.taskId}').body`, ctx), 'unchanged contract body');
  assert.equal(ctx.work.length, 1);
  assert.equal(ctx.work[0][0].taskId, subject.taskId);
});

test('Rust declaration-slicing harness still runs the body repair driver with the new dependencies', async () => {
  const names = [...fs.readFileSync(`${root}/workflow_decompose_repair_decls.rs`, 'utf8')
    .matchAll(/^    "([^"]+)",/gm)].map(match => match[1]);
  const sliced = names.map(name => {
    const marker = [`async function ${name}(`, `function ${name}(`, `const ${name} =`]
      .find(marker => source.includes(marker));
    assert.ok(marker, `Rust test declaration ${name} exists`);
    const rest = source.slice(source.indexOf(marker));
    return rest.slice(0, marker.includes('function') ? rest.indexOf('\n}') + 2 : rest.indexOf(';\n') + 1);
  }).join('\n');
  const rust = fs.readFileSync(`${root}/workflow_decompose_body_repair_tests.rs`, 'utf8');
  const tail = rust.slice(rust.indexOf('fn issue551_'));
  const driver = tail.split('let driver = r##"')[1].split('"##;')[0];
  let report;
  const done = new Promise(resolve => { report = resolve; });
  const ctx = vm.createContext({ console: { log: text => report(JSON.parse(text)) } });
  vm.runInContext(sliced + '\n' + driver, ctx);
  const result = await done;
  assert.equal(result.error, undefined);
  assert.equal(result.prompts.length, 5);
  assert.match(result.prompts[3], /Re-author a complete replacement from fresh context/);
  assert.match(result.prompts[3], /finding OPEN-GAMMA \(a repeat: seen in 2 earlier attempts/);
  for (const index of [2, 4]) {
    const current = result.prompts[index].split('Repair these exact authoritative findings:')[1];
    const [open, earlier] = current.split('Earlier attempts in this phase raised the following');
    assert.match(open, /finding OPEN-GAMMA/);
    assert.doesNotMatch(open, /finding RESOLVED-ALPHA|finding RESOLVED-BETA/);
    assert.match(earlier, /finding RESOLVED-ALPHA/);
    assert.match(earlier, /finding RESOLVED-BETA/);
  }
});


test('set-gate reentry drops an older draft and uses the published judged bytes', () => {
  const ctx = context();
  ctx.publishBodyCandidate(phase, 'published');
  ctx.retainSetBodyCandidate(subject, findings(39, 'set'));
  ctx.retainBodyCandidate(phase, 'five-finding draft', findings(5, 'best'));
  ctx.retainSetBodyCandidate(subject, findings(33, 'later set'));
  assert.equal(retained(ctx).body, 'published');
  assert.equal(retained(ctx).score, 33);
  assert.deepEqual(JSON.parse(JSON.stringify(retained(ctx).findings)), findings(33, 'later set'));
});


test('body ranking passes lint before comparing deduped fidelity findings', () => {
  const ctx = context();
  const lint = { ...findings(1, 'lint')[0], text: 'candidate artifact was refused: lint',
    deterministic_defect: { provenance: 'host_validator', code: 'tool_obligation', stage: 'tools' } };
  const fidelity = findings(5, 'fidelity').map(({ deterministic_defect, ...finding }) => finding);
  ctx.retainBodyCandidate(phase, 'lint refusal', [lint]);
  ctx.retainBodyCandidate(phase, 'mechanically clean', fidelity);
  ctx.retainBodyCandidate(phase, 'lint regression', [lint]);
  ctx.retainBodyCandidate(phase, 'judged lint regression', [{ ...lint, text: 'lint defect' }]);
  assert.equal(retained(ctx).body, 'mechanically clean');
  assert.equal(retained(ctx).score, 5);
  ctx.retainBodyCandidate(phase, 'deduped', [fidelity[0], fidelity[0]]);
  assert.equal(retained(ctx).body, 'deduped');
  assert.equal(retained(ctx).score, 1);
  const foreign = ['skeleton', 'inherited_predecessor', 'prd_input', 'operational', 'unknown']
    .map(remediation_scope => ({ text: 'foreign finding', remediation_scope }));
  ctx.retainBodyCandidate(phase, 'only body findings count', [fidelity[0], ...foreign]);
  assert.equal(retained(ctx).score, 1);
  assert.equal(retained(ctx).findings.length, 1);
});

test('shadow return excludes inherited findings and re-entry delivers new grouped G', async () => {
  const ctx = context(), prompts = [];
  const body = findings(1, 'body');
  const excluded = ['skeleton', 'inherited_predecessor'].map(remediation_scope => ({
    text: `excluded ${remediation_scope}`, remediation_scope
  }));
  await ctx.authorCandidate({ agent: async () => reply('retained shadow body'),
    hostCommand: async () => outcome([...body, ...excluded]) }, policy(ctx));
  assert.equal(retained(ctx).score, 1);
  assert.equal(retained(ctx).findings.length, 1);
  ctx.publishBodyCandidate(phase, 'published shadow body');
  const grouped = findings(1, 'G');
  ctx.retainSetBodyCandidate(subject, grouped);
  await ctx.authorCandidate({
    agent: async (_, opts) => { prompts.push(opts.task); return reply('fixed'); },
    hostCommand: async () => outcome([])
  }, ctx.bodyPolicy(subject, ['new grouped finding G']));
  assert.match(prompts[0], /new grouped finding G/);
  assert.match(prompts[0], /G finding 0/);
  assert.doesNotMatch(prompts[0], /excluded skeleton|excluded inherited_predecessor|body finding 0/);
  assert.equal(details(prompts[0]).body, 'published shadow body');
  assert.deepEqual(details(prompts[0]).findings, grouped);
});

test('fresh rung on direct re-entry has findings without retained bytes or a failed-edit claim', async () => {
  const ctx = context();
  ctx.retainBodyCandidate(phase, 'SECRET RETAINED BODY', findings(1, 'previous'));
  let prompt;
  await ctx.authorCandidate({ agent: async (_, opts) => { prompt = opts.task; return reply('new'); },
    hostCommand: async () => outcome([]) }, { ...policy(ctx), initialRung: 4 });
  assert.match(prompt, /Previously seen defects:.*\n- previous finding 0/);
  assert.doesNotMatch(prompt, /SECRET RETAINED BODY|bounded.edit.*failed|Retained candidate/);
});

test('upgrade seed recovers the best journaled judgment, or the latest for older seeds', () => {
  for (const journaled of [true, false]) {
    const ctx = context();
    const seed = { kind: 'artifact', candidate: 'latest worse', gate: { findings: findings(33, 'latest') },
      ...(journaled ? { journal_results: [
        { body: 'best journaled', findings: findings(5, 'best') },
        { body: 'latest worse', findings: findings(33, 'latest') }
      ] } : {}) };
    ctx.seedArtifact(seed, policy(ctx));
    assert.equal(retained(ctx).body, journaled ? 'best journaled' : 'latest worse');
  }
});

test('workflow seeds on-disk published body for set-gate repair', async () => {
  const ctx = context();
  ctx.args.frozenChain = { acceptance: true, skeleton: true, subjects: [subject],
    bodies: [subject.fileName] };
  ctx.subject = subject;
  vm.runInContext(`
    requireFixedArgs = () => {};
    prepareAuthorStages = async () => ({ acceptance: {}, skeleton: { subjects: [subject] }, changedChecks: new Set() });
    reconcile = () => {};
    runSetGateLoop = async w => {
      retainSetBodyCandidate(subject, [{ text: 'set defect', remediation_scope: 'body' }]);
      await authorCandidate(w, bodyPolicy(subject, ['set defect']));
      return { taskSetLint: {}, requirementsTrace: {} };
    };
  `, ctx);
  let prompt;
  await ctx.workflow({ agent: async (_, opts) => { prompt = opts.task; return reply('repaired disk body'); },
    hostCommand: async () => outcome([]), finalReport: async () => ({}) });
  assert.match(prompt, /Read the retained candidate as raw text at \/tasks\/TASK-SAMPLE-001.md/);
  assert.deepEqual(details(prompt).findings, [{ text: 'set defect', remediation_scope: 'body' }]);
});


test('empty or non-body judgments never retain or anchor a body candidate', async () => {
  for (const found of [[], [{ text: 'predecessor defect', remediation_scope: 'inherited_predecessor' }]]) {
    const ctx = context();
    ctx.retainBodyCandidate(phase, 'unusable anchor', found);
    assert.equal(retained(ctx), undefined);
    assert.equal(ctx.bodyRepairContext(policy(ctx), 2), null);
  }
  const ctx = context();
  await ctx.authorCandidate({ agent: async () => reply('unusable anchor'),
    hostCommand: async () => outcome([{ text: 'predecessor defect', remediation_scope: 'inherited_predecessor' }]) }, policy(ctx));
  assert.equal(retained(ctx), undefined);
  ctx.publishBodyCandidate(phase, 'published');
  ctx.retainSetBodyCandidate(subject, []);
  assert.equal(retained(ctx), undefined);
});

test('set-gate task findings with fatal or inherited scopes reach the body author intact', async () => {
  for (const scope of ['prd_input', 'operational', 'unknown', 'inherited_predecessor']) {
    const ctx = context(), prompts = [];
    ctx.publishBodyCandidate(phase, 'published body');
    const found = { text: `exact ${scope} defect`, remediation_scope: scope,
      source_path: `/tasks/${subject.fileName}`, subject: subject.taskId };
    let round = 0;
    await ctx.runSetGateLoop({ checkpoint: async () => ({}),
      agent: async (_, opts) => { prompts.push(opts.task); return reply('corrected'); },
      hostCommand: async cap => outcome(cap === 'task-set-lint' && ++round === 1 ? [found] : [])
    }, { subjects: [subject] }, new Map());
    assert.equal(prompts.length, 1);
    assert.equal(details(prompts[0]).body, 'published body');
    assert.deepEqual(details(prompts[0]).findings, [{ ...found, remediation_scope: 'body' }]);
  }
});

test('latest unjudged seed goes to the gate before selecting a judged repair draft', async () => {
  const ctx = context();
  const seed = { candidate: 'latest unjudged', gate: null, journal_results: [
    { body: 'earlier judged', findings: findings(1, 'earlier') }
  ] };
  const seeded = ctx.seedArtifact(seed, policy(ctx));
  assert.equal(seeded.carried, 'latest unjudged');
  assert.equal(seeded.previousCandidate, 'latest unjudged');
  ctx.args.phaseSeed = { subjects: { [phase]: { kind: 'artifact', ...seed } } };
  let gated;
  await ctx.authorCandidate({ agent: async () => { throw new Error('must gate the carried reply first'); },
    hostCommand: async (_, opts) => { gated = opts.stdin; return outcome([]); } }, policy(ctx));
  assert.equal(gated, 'latest unjudged');
});

test('frozen arguments and verification envelopes exclude cached body text', () => {
  const frozen = fs.readFileSync(`${root}/workflow_decompose_frozen_chain.rs`, 'utf8');
  const argument = frozen.split('pub(crate) fn to_argument')[1].split('/// Read and verify')[0];
  assert.doesNotMatch(argument, /body_contents/);
  const stage = fs.readFileSync(`${root}/workflow_decompose_frozen_chain_stage.rs`, 'utf8');
  assert.match(stage, /snapshot.to_argument\(\)/);
  const launch = fs.readFileSync(`${root}/workflow_decompose_launch_repair.rs`, 'utf8');
  assert.equal([...launch.matchAll(/launched\.bodies.clear\(\);\s*launched\.body_contents.clear\(\);/g)].length, 2);
});
