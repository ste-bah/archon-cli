const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const root = process.env.ARCHON_TEST_SCRIPT_ROOT || __dirname;
const source = ['v1', 'v1_body', 'reply_blocks', 'v1_acceptance', 'v1_set_gate',
  'v1_frozen', 'v1_prompt', 'v1_progress', 'v1_failing_set', 'v1_seed', 'v1_context']
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
function details(prompt) {
  const json = prompt.split('Exact gate findings (JSON):\n')[1]?.split('\n')[0];
  assert.ok(json, 'repair contains structured findings');
  const body = prompt.split('Retained candidate (raw text):\n')[1]?.split('\nEnd retained candidate.')[0];
  return { body, ...JSON.parse(json) };
}

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
  assert.match(result.prompts[3], /Previously seen defects:\n- finding OPEN-GAMMA/);
  assert.doesNotMatch(result.prompts[3], /Retained candidate|# TASK-WS-002/);
  for (const index of [2, 4]) {
    const current = result.prompts[index].split('Repair these exact authoritative findings:')[1];
    const [open, earlier] = current.split('Earlier attempts in this phase raised the following');
    assert.match(open, /finding OPEN-GAMMA/);
    assert.doesNotMatch(open, /finding RESOLVED-ALPHA|finding RESOLVED-BETA/);
    assert.match(earlier, /finding RESOLVED-ALPHA/);
    assert.match(earlier, /finding RESOLVED-BETA/);
  }
});


test('ranking uses progress measures, deduplicates and never retains zero-actionable candidates', () => {
  const ctx = context(), p = policy(ctx);
  const retain = (prior, body, found) => ctx.retainBodyCandidate(prior, p, body, found);
  const lint = { ...findings(1, 'lint')[0], text: 'candidate artifact was refused: lint',
    deterministic_defect: { provenance: 'host_validator', code: 'tool_obligation', stage: 'tools' } };
  const fidelity = findings(5, 'fidelity').map(({ deterministic_defect, ...finding }) => finding);
  let best = retain(null, 'lint', [lint]);
  best = retain(best, 'cleaner', fidelity);
  assert.equal(retain(best, 'regression', [lint]).body, 'cleaner');
  best = retain(best, 'deduped', [fidelity[0], fidelity[0]]);
  assert.equal(best.score, 1);
  assert.equal(retain(best, 'newer tie', [fidelity[0]]).body, 'newer tie');
  for (const found of [[], [{ text: 'foreign', remediation_scope: 'inherited_predecessor' }]]) {
    assert.equal(retain(null, 'zero actionable', found), null);
    assert.equal(retain(best, 'zero actionable', found), best);
  }
});

test('owned fatal findings retain their exact original scope', async () => {
  for (const scope of ['prd_input', 'unknown']) {
  const ctx = context(), prompts = [];
  const found = { text: 'owned fatal defect', remediation_scope: scope,
    source_path: `/tasks/${subject.fileName}` };
  let call = 0;
  await ctx.authorCandidate({ agent: async (_, opts) => { prompts.push(opts.task); return reply(`body-${++call}`); },
    hostCommand: async () => outcome(call === 1 ? [found] : []) }, ctx.bodyPolicy(subject, ['initial set feedback']));
  assert.deepEqual(details(prompts[1]), { body: 'body-1', findings: [found], blocking_findings: 1 });
  assert.match(prompts[1], /initial set feedback/);
  }
});

test('owned inherited notes publish without retry; shadow findings return to the skeleton', async () => {
  for (const mode of ['observe', 'enforce']) for (const scope of ['inherited_predecessor', 'skeleton']) {
    const ctx = context();
    ctx.args.gateMode = mode;
    let calls = 0;
    const found = { text: 'predecessor freeze carries 1 policy finding(s)', remediation_scope: scope,
      source_path: `/tasks/${subject.fileName}` };
    const published = outcome([found]);
    const result = await ctx.authorCandidate({
      agent: async () => { assert.equal(++calls, 1, 'no body retry'); return reply('body'); },
      hostCommand: async () => published
    }, policy(ctx));
    assert.equal(result.publicationReceipt.call_id, 'published');
    if (scope === 'skeleton') {
      assert.equal(result.skeletonFindings.length, 1);
      assert.equal(result.skeletonFindings[0], found);
    }
    else assert.equal(result, published);
  }
});

test('large retained body uses host context; context failure logs and pauses', async () => {
  for (const broken of [false, true]) {
    const ctx = context(), body = 'unchanged\n'.repeat(4000), found = findings(5, 'large');
    let calls = 0, prompt;
    const logs = [];
    const run = ctx.authorCandidate({
      checkpoint: async (_, opts) => logs.push(opts.task),
      agent: async (_, opts) => {
        calls++;
        if (calls === 1) return reply(body);
        prompt = opts.task;
        return reply('fixed');
      },
      hostCommand: async () => {
        if (broken) ctx.__archonAuthorContext = () => { throw new Error('disk unavailable'); };
        return outcome(calls === 1 ? found : []);
      },
      pause: async (_, evidence) => {
        assert.equal(evidence.reason, 'operational_no_progress');
        throw new Error('paused context outage');
      }
    }, policy(ctx));
    if (broken) {
      await assert.rejects(run, /paused context outage/);
      assert.equal(calls, 1);
      assert.ok(logs.some(line => /retained body candidate context failed.*disk unavailable/.test(line)));
    } else {
      await run;
      const path = prompt.match(/at (\/run\/author-context\/[^ ]+)/)[1];
      assert.equal(ctx.__archonAuthorContext.files.get(path), body);
      assert.deepEqual(details(prompt).findings, found);
    }
  }
});

test('retained context host faults throw; write outages work without checkpoint', async () => {
  for (const fault of ['missing', 'no file', 'write']) {
    const ctx = context();
    let calls = 0;
    const run = ctx.authorCandidate({
      agent: async () => { assert.equal(++calls, 1); return reply('body'.repeat(5000)); },
      hostCommand: async () => {
        if (fault === 'missing') delete ctx.__archonAuthorContext;
        else ctx.__archonAuthorContext = fault === 'no file' ? () => '{}' : () => { throw new Error('disk unavailable'); };
        return outcome(findings(1, 'large'));
      },
      pause: async () => { throw new Error('paused context outage'); }
    }, policy(ctx));
    await assert.rejects(run, error => {
      assert.match(error.message, fault === 'write' ? /paused context outage/ : fault === 'missing' ? /binding is missing/ : /returned no file/);
      if (fault !== 'write') {
        assert.equal(error.retainedBodyContext, true);
        assert.notEqual(error.authorContext, true);
      }
      return true;
    });
  }
});

test('accepted, shadow, error and pause episode exits discard retained bytes on reentry', async () => {
  for (const exit of ['accepted', 'shadow', 'error', 'pause']) {
    const ctx = context();
    let calls = 0;
    const run = ctx.authorCandidate({
      checkpoint: async () => ({}),
      agent: async () => {
        calls++;
        if (calls > 1 && exit === 'error') throw new Error('episode error');
        return reply('SECRET OLD BODY');
      },
      hostCommand: async () => outcome(calls === 1 || exit === 'pause' ? findings(1, 'old')
        : exit === 'shadow' ? [{ text: 'skeleton defect', remediation_scope: 'skeleton' }] : []),
      pause: async () => { throw new Error('episode paused'); }
    }, policy(ctx));
    if (exit === 'error' || exit === 'pause') await assert.rejects(run, /episode (error|paused)/);
    else await run;
    let prompt;
    await ctx.authorCandidate({ agent: async (_, opts) => { prompt = opts.task; return reply('new body'); },
      hostCommand: async () => outcome([]) }, policy(ctx));
    assert.doesNotMatch(prompt, /SECRET OLD BODY|Retained candidate|old finding/);
  }
});

test('a resumed pause inside the call clears the retained body before dispatch', async () => {
  const ctx = context(), prompts = [];
  let paused = false;
  await ctx.authorCandidate({ checkpoint: async () => ({}),
    agent: async (_, opts) => { prompts.push(opts.task); return reply(paused ? 'fixed' : 'SECRET BEFORE PAUSE'); },
    hostCommand: async () => outcome(paused ? [] : findings(1, 'stall')),
    pause: async () => { paused = true; }
  }, policy(ctx));
  assert.ok(paused);
  assert.doesNotMatch(prompts.at(-1), /SECRET BEFORE PAUSE|Retained candidate/);
});
