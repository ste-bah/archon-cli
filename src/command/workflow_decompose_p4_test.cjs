// Package P4 author contracts and stable gate-finding identity.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const test = require('node:test');
const path = require('node:path');
const read = file => fs.readFileSync(path.join(__dirname, file), 'utf8');
const S1 = "A path the check reads by its literal relative path, as an operand of a command that reads it (an existence test included), is a move-aside handle: the host may move it aside to prove the check can fail. A handle is not the deciding assertion; the test or implementation run whose status decides the check must still decide it.";
const S2 = "A move-aside handle line is neither the deciding assertion nor a reason to refute; judge the deciding test.";
function context() {
  const ctx = { args: { repositoryRoot: '/repository', projectRoot: '/project', prdPath: '/project/prd' }, TextEncoder,
    __archonAuthorContext: () => JSON.stringify({path:'/context/file.json',sha256:'a'.repeat(64)}) };
  vm.createContext(ctx);
  vm.runInContext(read('workflow_decompose_v1.js') + '\n' + read('workflow_decompose_v1_acceptance.js')
    + '\n' + read('workflow_decompose_v1_context.js') + '\n' + read('workflow_decompose_v1_progress.js')
    + '\n' + read('workflow_decompose_v1_frozen.js') + '\n' + read('workflow_decompose_v1_prompt.js'), ctx);
  return ctx;
}

test('P4_12_shared_path_rule_is_byte_equal', () => {
  const host = read('workflow_acceptance_live_root.rs').match(/("Where a check runs:.*")/)[1];
  const expected = JSON.parse(host).replace('{repository}', '/repository').replace('{project}', '/project');
  const actual = vm.runInContext('checkPathRule()', context());
  assert.equal(actual, expected);
  assert.ok(actual.includes(S1));
});

test('P4_13_author_names_source_test_and_data_reads', () => {
  const prompt = vm.runInContext('acceptanceAuthorPrompt()', context());
  assert.ok(prompt.includes('every file (source, test or data) it reads by a path relative to its working directory'));
  assert.ok(!prompt.includes('every data file it names'));
});

test('P4_15_judge_bounds_handle_duplicate_and_coverage_claims', () => {
  const source = read('workflow_task_set_judge.rs');
  assert.ok(source.includes(S2));
  assert.ok(source.includes('only a duplicate whose path you name and were shown'));
  assert.ok(source.includes('tests may cover less than the criterion only when named tests whose supplied content was shown leave a covered requirement unasserted'));
});

test('P4_16_both_command_cwds_are_examples', () => {
  const format = vm.runInContext('acceptanceReplyFormat()', context());
  assert.ok(format.includes('"cwd":"repo_root"'));
  assert.ok(format.includes('"cwd":"project_root"'));
  const source = read('workflow_acceptance_reauthor.rs');
  const raw = source.match(/r#"(\[.*\])"#/)[1];
  const shapes = JSON.parse(raw);
  assert.ok(shapes.some(entry => entry.check.kind === 'command' && entry.check.cwd === 'project_root'));
  const examples = source.match(/COMMAND_CWD_EXAMPLES: &str = r#"(.*?)"#/)[1];
  assert.ok(examples.includes('"cwd":"repo_root"'));
  assert.ok(examples.includes('"cwd":"project_root"'));
  assert.ok(format.includes(examples));
});

test('P4_17_sibling_consistency_keeps_own_deciding_file', () => {
  const text = vm.runInContext('priorText([{id:"A", check:{kind:"command", cwd:"repo_root", command:"make test"}}], "B", ["A","B"])', context());
  assert.ok(text.includes('Keep names, flags and formats consistent; a regression guard still names its own deciding file.'));
  assert.ok(text.includes(S1));
});

test('P4_19_finding_identity_survives_changed_details_and_reopening', () => {
  const ctx = context();
  vm.runInContext(`
    const state = {};
    const finding = (text, code = "check_not_proven", location = "check") => ({subject:"A", text,
      deterministic_defect:{code, subject:"A", location}});
    const latest = finding => attributeEntryFindings([finding], new Set(["A"]), false, null, new Set()).own;
    settleGateFindings(state, latest(finding("baseline failed; old detail")));
    settleGateFindings(state, latest(finding("baseline failed; new detail")));
    if (state.earlierGateFindings.size) throw Error("open defect listed as resolved");
    settleGateFindings(state, latest(finding("entirely new wording; detail")));
    if (state.earlierGateFindings.size) throw Error("structured identity lost");
    settleGateFindings(state, latest(finding("another defect; detail", "other")));
    if (state.earlierGateFindings.get("A").length !== 1) throw Error("resolved defect lost");
    settleGateFindings(state, latest(finding("reopened; detail")));
    if (state.earlierGateFindings.get("A").some(text => text.startsWith("entirely"))) throw Error("reopened defect still resolved");
    const positions = {};
    settleGateFindings(positions, latest(finding("same text", "check_not_proven", "left")));
    settleGateFindings(positions, latest(finding("same text", "check_not_proven", "right")));
    if (positions.earlierGateFindings.get("A")?.length !== 1) throw Error("different locations conflated");
    const both = attributeEntryFindings([finding("same text", "check_not_proven", "left"),
      finding("same text", "check_not_proven", "right")], new Set(["A"]), false, null, new Set()).own.get("A");
    if (both.length !== 2) throw Error("identical text dropped a distinct defect");
    settleGateFindings(positions, new Map([["A", both]]));
    if (positions.earlierGateFindings.size) throw Error("reopening by location failed");
    const legacy = {};
    settleGateFindings(legacy, new Map([["A", ["same lead; old argument"]]]));
    settleGateFindings(legacy, new Map([["A", ["same lead; new argument"]]]));
    if (legacy.earlierGateFindings.size) throw Error("legacy identity lost");
    settleGateFindings(legacy, new Map([["B", ["same lead; other id"]]]));
    if (!legacy.earlierGateFindings.has("A")) throw Error("different ids conflated");
  `, ctx);
});

test('P4_21_every_cover_must_be_false_detectable', () => {
  const prompt = vm.runInContext('acceptanceAuthorPrompt()', context());
  assert.ok(prompt.includes('each covered requirement must be made false-detectable by this check; split a check whose covers it cannot all fail on'));
});

test('R6_P4_19_moved_slot_remains_open_only', () => {
  const ctx = context();
  vm.runInContext(`
    const movedState = {};
    const atSlot = slot => attributeEntryFindings([{
      subject: "A", text: "same defect; slot " + slot,
      deterministic_defect: {code:"check_not_proven", subject:"checks/" + slot, location:"check"}
    }], new Set(["A"]), false, null, new Set()).own;
    settleGateFindings(movedState, atSlot(3));
    settleGateFindings(movedState, atSlot(4));
    if (movedState.earlierGateFindings.size) throw Error("moved entry also listed as resolved");
    if (movedState.gateFindings.get("A").length !== 1) throw Error("open finding lost");
  `, ctx);
});

test('R5_feedback_prefetch_is_forwarded_per_attempt', () => {
  const ctx = context();
  const seen = [];
  ctx.__archonNamedFiles = (entry, notes) => { seen.push(JSON.parse(notes)); return '[]'; };
  vm.runInContext(`namedFilesContext("author", {id:"A", criterion:"prose/only"}, ["inspect first.txt"]);
    namedFilesContext("author", {id:"A", check:{kind:"command", cwd:"project_root", command:"cat actual.txt"}}, ["inspect second.txt"]);`, ctx);
  assert.deepEqual(seen, [['inspect first.txt'], ['inspect second.txt']]);
});
