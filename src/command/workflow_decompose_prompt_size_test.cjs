// Issue 288: an author prompt grows by a bounded amount per new acceptance
// entry and per new attempt, never by the size of everything done before.
// Prints the measured prompt bytes for a fixture of 120 entries and 30
// attempts. Also pins that an inactivity cut in a round that kept new work
// does not consume the no-progress window, and that cuts without progress
// pause the loop (they never fail it).
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const dir = process.env.ARCHON_DECOMPOSE_SCRIPT_DIR || __dirname;
const FILES = ['workflow_decompose_v1.js', 'workflow_decompose_v1_acceptance.js', 'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_progress.js'];
const source = FILES.map((f) => fs.readFileSync(`${dir}/${f}`, 'utf8')).join('\n');

function context(args) {
  const ctx = { args, console };
  vm.createContext(ctx);
  vm.runInContext(source, ctx);
  return ctx;
}

const pad = (n) => String(n).padStart(3, '0');
const words = (seed, length) => {
  let text = '';
  for (let i = 0; text.length < length; i += 1) text += `${['the', 'check', 'drives', 'path', 'output', 'record', 'field', 'value'][(seed + i) % 8]} `;
  return text.slice(0, length);
};
const criteria = (count) => Object.fromEntries(Array.from({ length: count }, (_, i) => [`AC-${pad(i + 1)}`, `criterion ${i + 1}: ${words(i, 240)}`]));

// A completed entry the size of the live ones (about 9.9 KB of JSON each).
function entry(id, n) {
  return {
    id,
    criterion: words(n, 300),
    check: n % 2 === 0
      ? { kind: 'command', command: `scripts/run-check.sh --case case_${pad(n)} --fixture fixtures/shared_fixture_${n % 7}.json && test -s out/result_${pad(n)}.json`, cwd: 'project_root' }
      : { kind: 'floor', contract: { kind: `deliverable_${n % 5}`, artifact_path: `out/artifact_${pad(n)}.json`, artifact_format: 'json', required_true_fields: ['complete', 'verified'], typed_verifier_command: `scripts/verify.sh out/artifact_${pad(n)}.json` } },
    gap_permitted: false,
    covers: [`REQ-${pad(n)}`, `REQ-${pad(n + 1)}`],
    judgment: { verdict: 'accepted', counterexample: words(n + 1, 2400), reason: words(n + 2, 6800), host_call_id: `call-${n}` },
  };
}

// The prompt of the acceptance author for the entry after `done` completed ones.
async function acceptancePrompt(done, total) {
  const all = criteria(total);
  const ctx = context({ acceptanceCriteria: all, authorMaxParallelism: 1 });
  const ids = Object.keys(all).sort();
  const state = { entries: new Map(ids.slice(0, done).map((id, n) => [id, entry(id, n)])), retryIds: new Set() };
  const prompts = [];
  const w = { agent: async (_id, options) => { prompts.push(options.task); return { status: 'failed', summary: 'stop after one call' }; } };
  await ctx.authorAcceptanceEntries(w, 'author', 1, state);
  assert.equal(prompts.length, 1);
  assert.match(prompts[0], new RegExp(`Author ONLY entry ${ids[done]}:`));
  return prompts[0];
}

async function priorGrowsBoundedPerEntry() {
  const total = 121;
  const at100 = await acceptancePrompt(100, total);
  const at120 = await acceptancePrompt(120, total);
  const entryJson = JSON.stringify(entry('AC-001', 1)).length;
  const perEntry = (Buffer.byteLength(at120) - Buffer.byteLength(at100)) / 20;
  console.log(`acceptance author prompt: ${Buffer.byteLength(at100)} bytes after 100 entries, ${Buffer.byteLength(at120)} bytes after 120 entries, ${perEntry} bytes per completed entry (entry JSON ${entryJson} bytes)`);
  assert(perEntry <= 800, `prompt grows ${perEntry} bytes per completed entry; the bound is 800`);
  // What the author needs to stay consistent: every earlier id, its check
  // kind, what it covers, and the paths and fixtures it uses.
  for (const n of [0, 57, 119]) {
    const id = `AC-${pad(n + 1)}`;
    assert(at120.includes(id), `prior names ${id}`);
    assert(at120.includes(`REQ-${pad(n)}`), `prior keeps ${id}'s covers`);
    assert(at120.includes(n % 2 === 0 ? `fixtures/shared_fixture_${n % 7}.json` : `out/artifact_${pad(n)}.json`), `prior keeps ${id}'s fixture or artifact`);
  }
}

// Each attempt: three host findings that rotate (the oscillation the history
// exists to stop) and two judge findings worded anew every attempt.
const ROTATING = Array.from({ length: 6 }, (_, i) => `check 'AC-${pad(i + 1)}' floor ${words(i, 380)}`);
const findingsOf = (attempt) => [
  ROTATING[attempt % 6], ROTATING[(attempt + 1) % 6], ROTATING[(attempt + 2) % 6],
  `judge note ${pad(attempt)}a: ${words(attempt, 380)}`, `judge note ${pad(attempt)}b: ${words(attempt + 3, 380)}`,
];
function historyPrompt(ctx, attempts) {
  const history = Array.from({ length: attempts }, (_, i) => ({ attempt: i + 1, findings: findingsOf(i + 1) }));
  return ctx.authorPrompt('author', attempts + 1, findingsOf(attempts), history);
}

function historyGrowsBoundedPerAttempt() {
  const ctx = context({});
  const at30 = historyPrompt(ctx, 30);
  const at60 = historyPrompt(ctx, 60);
  const perAttempt = (Buffer.byteLength(at60) - Buffer.byteLength(at30)) / 30;
  console.log(`author repair prompt: ${Buffer.byteLength(at30)} bytes at attempt 31, ${Buffer.byteLength(at60)} bytes at attempt 61, ${perAttempt} bytes per attempt`);
  assert(perAttempt <= 64, `prompt grows ${perAttempt} bytes per attempt; the bound is 64`);
  // The newest findings are in full; every rotating finding is shown once.
  for (const finding of findingsOf(30)) assert(at30.includes(finding), 'newest finding in full');
  for (const finding of ROTATING) {
    const count = at30.split(finding.slice(0, 120)).length - 1;
    assert.equal(count, 1, `a repeated finding is shown once, not ${count} times`);
  }
  assert.match(at30, /older distinct findings/, 'findings not shown are counted');
}

// The finding a phase was opened to repair stays in every prompt.
function seededFindingStays() {
  const ctx = context({});
  const seed = 'the other task waives it';
  const history = [{ attempt: 0, findings: [seed] }, ...Array.from({ length: 40 }, (_, i) => ({ attempt: i + 1, findings: findingsOf(i + 1) }))];
  const prompt = ctx.authorPrompt('author', 42, findingsOf(40), history);
  assert(prompt.includes(`the set gate, before this body was sent back: ${seed}`), prompt.slice(-2000));
}

const CUT = "workflow v2 call 'acceptance-author-AC-002-1' failed: subagent inactivity timeout: no model output, tool call or tool result for 3600s (inactivity limit 3600s); the host ended the session inside its wall clock";

// Round 1 keeps AC-001 and is cut on AC-002: the round kept new work, so the
// cut consumes nothing. Three more cuts with nothing kept pause the loop; the
// resumed loop finishes. Nothing fails.
async function inactivityCutWithProgressKeepsTheWindow() {
  const ctx = context({ acceptanceCriteria: { 'AC-001': 'one', 'AC-002': 'two' }, authorMaxParallelism: 1, gateMode: 'enforce' });
  let cuts = 0;
  const pauses = [];
  const w = {
    agent: async (_id, options) => {
      const id = options.task.match(/Author ONLY entry ([^:]+):/)[1];
      if (id === 'AC-002' && cuts < 4) { cuts += 1; return { status: 'failed', summary: CUT }; }
      return { status: 'accepted', stopReason: 'end_turn', content: JSON.stringify({ id, check: { kind: 'command', command: 'true' } }) };
    },
    pause: async (id, evidence) => { pauses.push({ id, evidence, cuts }); return { resumed: true }; },
    hostCommand: async (cap) => ({ publicationReceipt: { call_id: cap }, postcondition: { satisfied: true }, gateEnvelope: { policy_findings: [] } }),
  };
  const outcome = await ctx.authorCandidate(w, { phase: 'acceptance', author: ctx.authorAcceptanceEntries, capability: 'freeze-acceptance', retryScopes: new Set(['candidate_artifact']), prompt: () => 'author' });
  assert.equal(outcome.publicationReceipt.call_id, 'freeze-acceptance');
  assert.equal(pauses.length, 1, JSON.stringify(pauses));
  assert.equal(pauses[0].cuts, 4, 'the cut in the round that kept AC-001 consumed nothing: the pause follows three cuts after it');
  assert.equal(pauses[0].evidence.reason, 'operational_no_progress');
}

(async () => {
  for (const test of [priorGrowsBoundedPerEntry, historyGrowsBoundedPerAttempt, seededFindingStays, inactivityCutWithProgressKeepsTheWindow]) {
    try { await test(); } catch (error) { console.error(`${test.name}: ${error.message}`); process.exitCode = 1; }
  }
})();
