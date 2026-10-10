// Issue 288: an author prompt is bounded whatever the number of completed
// acceptance entries and earlier attempts: what it repeats from earlier work
// shares one byte budget, and the exact bytes are host-written files it names.
// Prints the measured prompt bytes for a fixture of 120 entries and 60
// attempts. Also pins that an inactivity cut in a round that kept new work
// does not consume the no-progress window, and that cuts without progress
// pause the loop (they never fail it).
const { authorContext, withAuthorContext } = require('./workflow_decompose_context_stub.cjs');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const dir = process.env.ARCHON_DECOMPOSE_SCRIPT_DIR || __dirname;
const FILES = ['workflow_decompose_v1.js', 'workflow_decompose_reply_blocks.js', 'workflow_decompose_v1_acceptance.js', 'workflow_decompose_v1_set_gate.js', 'workflow_decompose_v1_progress.js', 'workflow_decompose_v1_context.js'];
const source = FILES.map((f) => fs.readFileSync(`${dir}/${f}`, 'utf8')).join('\n');

function context(args, binding) {
  const ctx = { args, console };
  if (binding) ctx.__archonAuthorContext = binding;
  ctx.__archonValidateAcceptanceEntry = () => '[]';
  vm.createContext(withAuthorContext(ctx));
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

// Check sizes (JSON characters) sampled evenly from 165 checks in 15 real
// acceptance contracts (median 2,072, p90 7,225, max 15,501): commands are
// multi-line scripts that name the shared paths, keys and formats.
const CHECK_SIZES = [128, 195, 386, 445, 629, 789, 1193, 1295, 1414, 1485, 1882, 2040, 2129, 2271, 2810, 2973, 3103, 3542, 3824, 5090, 6521, 7554, 10103, 15501];

// A command of about `size` characters ending in the interface it fixes.
function command(n, size) {
  const head = `python3 -c "\nimport json\nspec=json.load(open('out/spec_${pad(n)}.json'))\nreg=json.load(open('out/registry.json'))\n`;
  const tail = `assert reg['datasets'][spec['id']+':'+spec['version']]['format']=='parquet', 'registry key id:version ${pad(n)}'\n"`;
  let body = '';
  for (let i = 0; head.length + body.length + tail.length < size - 60; i += 1) body += `assert spec.get('field_${i}') is not None, 'field_${i} missing'\n`;
  return head + body + tail;
}

// A completed entry shaped like the real ones: the check, a short criterion
// and an ~830-character judgment (the real median and mean).
function entry(id, n) {
  return {
    id,
    criterion: words(n, 90),
    check: { kind: 'command', command: command(n, CHECK_SIZES[n % CHECK_SIZES.length]), cwd: 'project_root' },
    gap_permitted: false,
    covers: [`REQ-${pad(n)}`, `REQ-${pad(n + 1)}`],
    judgment: { verdict: 'accepted', counterexample: words(n + 1, 300), reason: words(n + 2, 480), host_call_id: `call-${n}` },
  };
}

// The prompt of the acceptance author for the entry after `done` completed ones.
async function acceptancePrompt(done, total, binding) {
  const all = criteria(total);
  const ctx = context({ acceptanceCriteria: all, authorMaxParallelism: 1 }, binding);
  const ids = Object.keys(all).sort();
  const state = { entries: new Map(ids.slice(0, done).map((id, n) => [id, entry(id, n)])), retryIds: new Set() };
  const prompts = [];
  const w = { agent: async (_id, options) => { prompts.push(options.task); return { status: 'failed', summary: 'stop after one call' }; } };
  await ctx.authorAcceptanceEntries(w, 'author', 1, state);
  assert.equal(prompts.length, 1);
  assert.match(prompts[0], new RegExp(`Author ONLY entry ${ids[done]}:`));
  return prompts[0];
}

// Before: every completed entry's whole check, about 4 KB each (359,698 /
// 439,291 bytes after 100 / 120 entries). Now one record line per entry in a
// fixed share: the prompt after 120 entries is within that share of the one
// after the first entry, and entries past the share add nothing.
async function priorIsBoundedInAggregate() {
  const total = 121;
  const files = new Map();
  const sizes = {};
  const prompts = {};
  for (const done of [1, 70, 100, 120]) {
    prompts[done] = await acceptancePrompt(done, total, authorContext(files));
    sizes[done] = Buffer.byteLength(prompts[done]);
  }
  console.log(`acceptance author prompt bytes by completed entries: ${JSON.stringify(sizes)}`);
  assert(sizes[120] - sizes[1] <= 14336 + 1024, `prompt grew ${sizes[120] - sizes[1]} bytes from 1 to 120 entries`);
  assert(sizes[120] - sizes[100] <= 64, `prompt grew ${sizes[120] - sizes[100]} bytes from 100 to 120 entries`);
  assert(sizes[120] <= 48 * 1024, `prompt after 120 entries is ${sizes[120]} bytes`);
  assert(!prompts[120].includes('host_call_id') && !prompts[120].includes('field_3 missing'), 'no judgment and no whole check is inlined');
  // Every listed record names the sha256 of the entry's exact JSON, and the
  // file it names holds exactly those bytes.
  const at120 = prompts[120];
  const listed = [...at120.matchAll(/\n- (AC-\d+) sha256:([0-9a-f]{64}) covers=/g)];
  assert(listed.length > 10 && listed.length < 120, `${listed.length} records shown`);
  for (const [, id, sha] of listed) {
    const n = Number(id.slice(3)) - 1;
    assert.equal(files.get(`/run/author-context/${sha}.json`), JSON.stringify(entry(id, n)), `${id} resolves to its exact bytes`);
  }
  // The omitted ones are counted, and the index lists every one exactly.
  const omitted = at120.match(/; (\d+) more are not, and every one is listed in (\S+)\./);
  assert(omitted, at120.slice(-3000));
  assert.equal(Number(omitted[1]) + listed.length, 120);
  const index = files.get(omitted[2]).trim().split('\n').map((line) => JSON.parse(line));
  assert.equal(index.length, 120);
  for (const [n, record] of index.entries()) assert.equal(files.get(record.path), JSON.stringify(entry(`AC-${pad(n + 1)}`, n)));
  // The nearest entries are the ones shown: AC-120 (just before AC-121) is.
  assert(listed.some(([, id]) => id === 'AC-120') && !listed.some(([, id]) => id === 'AC-001'), 'nearest records shown');
}

// A record line is cut at a character boundary by bytes, not characters: a
// multibyte check does not buy a longer line, and the cut is marked.
async function multibyteRecordsAreCutByBytes() {
  const files = new Map();
  const ctx = context({}, authorContext(files));
  const wide = { id: 'AC-W', covers: ['REQ-1'], check: { kind: 'command', cwd: 'project_root', command: `echo ${'東京🚀'.repeat(400)}` } };
  const text = ctx.priorText([wide], 'AC-X', ['AC-W', 'AC-X']);
  const line = text.split('\n- ')[1];
  assert(Buffer.byteLength(line) <= 320, `record is ${Buffer.byteLength(line)} bytes`);
  assert(line.endsWith(' [cut]'), line);
  assert(!line.includes('�') && Buffer.from(line, 'utf8').toString('utf8') === line, 'no character is split');
  const sha = line.match(/sha256:([0-9a-f]{64})/)[1];
  assert.equal(files.get(`/run/author-context/${sha}.json`), JSON.stringify(wide), 'the exact bytes are kept whole');
  // A catalogue over its share is cut by bytes too and names the exact catalogue.
  const catalogue = Object.fromEntries(Array.from({ length: 80 }, (_, i) => [`AC-${pad(i + 1)}`, '基準'.repeat(100)]));
  const listed = ctx.catalogueText(catalogue, 'AC-040', Object.keys(catalogue));
  assert(Buffer.byteLength(listed) <= 10240 + 400, `catalogue is ${Buffer.byteLength(listed)} bytes`);
  const file = listed.match(/the exact catalogue is (\S+) \(sha256/)[1];
  assert.equal(files.get(file), JSON.stringify(catalogue));
  assert(listed.includes('\n- AC-040: '), 'the entry\'s own neighbourhood is listed');
}

// A host that cannot write the context never yields a prompt that names
// bytes that are not there: the entry's call is an outage, no agent is
// called, and outages pause the loop. A missing binding is a host fault.
async function unwritableContextIsAnOutageAndAMissingBindingThrows() {
  const all = { 'AC-001': 'one', 'AC-002': 'two' };
  const failing = () => { throw new Error('disk full'); };
  const ctx = context({ acceptanceCriteria: all, authorMaxParallelism: 1, gateMode: 'enforce' }, failing);
  const agents = [];
  const w = { agent: async (id) => { agents.push(id); return { status: 'failed', summary: 'unreached' }; } };
  const kept = { id: 'AC-001', check: { kind: 'command', command: 'true' } };
  const state = { entries: new Map([['AC-001', kept]]), retryIds: new Set(['AC-002']) };
  const out = await ctx.authorAcceptanceEntries(w, 'author', 2, state);
  assert.equal(agents.length, 0, 'no provider call');
  assert.equal(out.status, 'failed');
  assert.notEqual(out.malformed, true, 'an outage, not a refusal');
  assert.match(out.summary, /AC-002: author context could not be written: disk full/);
  assert.equal(state.roundOutage, true);
  assert.equal(state.roundCalls, 0);
  const bare = context({ acceptanceCriteria: all, authorMaxParallelism: 1 });
  delete bare.__archonAuthorContext;
  await assert.rejects(bare.authorAcceptanceEntries(w, 'author', 2, { entries: new Map([['AC-001', kept]]), retryIds: new Set(['AC-002']) }), /author-context binding is missing/);
}

// The current findings are never cut, whatever their size; a prompt whose
// mandatory text alone passes the dispatch limit pauses instead of sending.
async function currentFindingsStayWholeAndOversizedPromptsPause() {
  const ctx = context({ acceptanceCriteria: { 'AC-001': 'one' }, authorMaxParallelism: 1, gateMode: 'enforce' });
  const big = Array.from({ length: 40 }, (_, i) => `check 'AC-001' finding ${i}: ${words(i, 2000)}`);
  const prompt = ctx.authorPrompt('author', 2, big, [{ attempt: 1, findings: big }]);
  for (const finding of big) assert(prompt.includes(finding), 'a current finding is shown whole');
  const huge = Array.from({ length: 200 }, (_, i) => `check 'AC-001' finding ${i}: ${words(i, 2000)}`);
  const pauses = [];
  let agents = 0;
  const w = {
    agent: async () => { agents += 1; return { status: 'failed', summary: 'unreached' }; },
    pause: async (id, evidence) => { pauses.push({ id, evidence }); if (pauses.length === 2) throw new Error('paused'); return { resumed: true }; },
  };
  await assert.rejects(ctx.authorAcceptanceEntries(w, ctx.authorPrompt('author', 2, huge, []), 1, { entries: new Map(), retryIds: null }), /paused/);
  assert.equal(agents, 0, 'never sent');
  assert.deepEqual(pauses.map((p) => p.id), ['pause-acceptance-prompt-AC-001-1', 'pause-acceptance-prompt-AC-001-2'], 'a resumed loop that still measures it pauses again');
  assert.equal(pauses[0].evidence.reason, 'author_prompt_oversized');
  assert.match(pauses[0].evidence.remedy, /raise AUTHOR_PROMPT_LIMIT/);
  assert.match(pauses[0].evidence.remedy, /resuming alone pauses again/);
  assert(pauses[0].evidence.prompt_bytes > pauses[0].evidence.limit_bytes);
}

// The findings a phase was opened with (attempt 0) were unbounded: a set
// gate that sent a body back with hundreds of findings repeated all of them
// in every prompt. Now they share the history budget, the rest are counted,
// and the exact history is a file.
function seededHistoryIsBounded() {
  const files = new Map();
  const ctx = context({}, authorContext(files));
  const seeded = Array.from({ length: 300 }, (_, i) => `seeded finding ${i}: ${words(i, 500)}`);
  const prompt = ctx.authorPrompt('author', 2, ['current'], [{ attempt: 0, findings: seeded }]);
  const history = prompt.split('Earlier attempts in this phase')[1];
  assert(Buffer.byteLength(history) <= 8192 + 600, `history is ${Buffer.byteLength(history)} bytes`);
  assert(prompt.includes(`the set gate, before this body was sent back: ${seeded[0]}`), 'the first seeded finding is shown');
  const counted = prompt.match(/\n(\d+) older distinct findings \((\d+) occurrences, attempts 0-0\) are not repeated here\./);
  assert(counted, prompt.slice(-500));
  const file = prompt.match(/in full with its count and attempts, is in (\S+) /)[1];
  const all = JSON.parse(files.get(file));
  assert.equal(all.length, 300);
  assert.deepEqual(all.map((found) => found.text), seeded, 'every seeded finding, exactly');
  assert.equal(Number(counted[1]) + (history.match(/\n- /g) || []).length, 300);
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
  // A current finding earlier attempts also triggered is marked as a repeat.
  assert.match(at30, /\(a repeat: seen in \d+ earlier attempts, attempts \d+-\d+\)/);
}

// Findings that differ by a sign, a digit, punctuation or case stay apart
// (a path or a flag can differ by case alone).
function nearFindingsStayApart() {
  const ctx = context({});
  const texts = ["check 'A' value < 1", "check 'A' value > 1", "check 'A' offset -1", "check 'A' offset 1", 'field a_b', 'field a-b', 'reads out/Data.json', 'reads out/data.json', 'flag -V', 'flag -v'];
  const history = texts.map((text, i) => ({ attempt: i + 1, findings: [text] }));
  const prompt = ctx.authorPrompt('author', 12, ['unrelated'], history);
  for (const text of texts) assert(prompt.includes(`: ${text}`), `${text} is shown: ${prompt}`);
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

// Round 1 keeps AC-001 and is cut on AC-002: retained work does not reset the
// operational window. Three cuts, including that mixed round, pause; resume
// opens a fresh operational window and completes.
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
  assert.equal(pauses[0].cuts, 3, 'the mixed cut and two more fill the operational window');
  assert.equal(pauses[0].evidence.reason, 'operational_no_progress');
}

(async () => {
  for (const test of [priorIsBoundedInAggregate, multibyteRecordsAreCutByBytes, unwritableContextIsAnOutageAndAMissingBindingThrows, currentFindingsStayWholeAndOversizedPromptsPause, historyGrowsBoundedPerAttempt, nearFindingsStayApart, seededFindingStays, seededHistoryIsBounded, inactivityCutWithProgressKeepsTheWindow]) {
    try { await test(); } catch (error) { console.error(`${test.name}: ${error.message}`); process.exitCode = 1; }
  }
})();
