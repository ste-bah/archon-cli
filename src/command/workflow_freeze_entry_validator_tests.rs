//! Issue 357 round 4: the author-step entry validator never throws on model
//! content, names the entry it refuses, and validates the entry the author
//! step keeps (host-owned fields set first). Each test drives the fixed
//! script through the real native binding, so it is independent of the
//! binding's argument list and fails on the round-3 script and binding.
use super::entry_validator::refusal_text;
use super::shape_tests::script_source;
use super::*;
use serde_json::{Value, json};

/// JS helpers shared by the tests: `author(contents, criteria, owed)` runs one
/// acceptance author round on the given replies and reports its outcome, or
/// the error it threw. `loop(contents)` runs the whole author loop to freeze.
const PRELUDE: &str = r#"
const reply = content => ({status:'accepted', stopReason:'end_turn', content});
const check = {kind:'command', command:'test -f output', cwd:'project_root'};
const valid = id => JSON.stringify({id, criterion:'', check});
async function author(contents, criteria, owed = []) {
  args.acceptanceCriteria = criteria;
  for (const [id, requirement] of owed) owedSupplementary().set(id, {requirement, text:'owed'});
  const w = {agent: async (_, options) => {
    const id = options.task.match(/Author ONLY entry ([^:]+):/)[1];
    return reply(contents[id]);
  }};
  const state = {entries:new Map(), retryIds:null};
  try {
    const out = await authorAcceptanceEntries(w, 'author', 1, state);
    return {out, entries:Object.fromEntries(state.entries)};
  } catch (e) { return {error:String(e && e.message)}; }
}
async function loop(contents, criteria) {
  args.acceptanceCriteria = criteria;
  const seen = new Map(), prompts = [];
  let freezes = 0;
  const w = {
    agent: async (_, options) => {
      const id = options.task.match(/Author ONLY entry ([^:]+):/)[1];
      prompts.push({id, task:options.task});
      const n = seen.get(id) || 0; seen.set(id, n + 1);
      const list = contents[id];
      if (prompts.length > 20) throw Error('bounded test exhausted');
      return reply(list[Math.min(n, list.length - 1)]);
    },
    hostCommand: async () => { freezes++; return {publicationReceipt:{call_id:'freeze'},
      postcondition:{satisfied:true}, gateEnvelope:{policy_findings:[]}}; },
    pause: async () => { throw Error('paused'); }
  };
  try {
    await authorCandidate(w, {phase:'acceptance', prompt:() => 'author', author:authorAcceptanceEntries,
      capability:'freeze-acceptance', retryScopes:new Set(['candidate_artifact'])});
    return {freezes, prompts};
  } catch (e) { return {error:String(e && e.message), freezes, prompts}; }
}
"#;

fn run(body: &str) -> Value {
    let source = script_source();
    let runtime = rquickjs::Runtime::new().unwrap();
    let context = rquickjs::Context::full(&runtime).unwrap();
    context.with(|ctx| {
        install_entry_validator(&ctx).unwrap();
        let script = [
            "const args = {repositoryRoot:'/archon-test-roots/repo',projectRoot:'/archon-test-roots/project',authorMaxParallelism:1, gateMode:'enforce'};",
            &source,
            PRELUDE,
            "(async () => {",
            body,
            "})()",
        ]
        .join("\n");
        let promise: rquickjs::Promise = ctx.eval(script).unwrap();
        let result: String = promise.finish().unwrap();
        serde_json::from_str(&result).unwrap()
    })
}

/// The round-1 outcome for entry A whose reply is the JS expression `content`.
fn author_a(content: &str) -> Value {
    run(&format!(
        "return JSON.stringify(await author({{A:{content}}}, {{A:'a'}}));"
    ))
}

fn assert_invalid_json(result: &Value) {
    assert_eq!(
        result["error"],
        Value::Null,
        "a model reply never throws: {result}"
    );
    let out = &result["out"];
    assert_eq!(out["status"], "failed", "{result}");
    assert_eq!(out["malformed"], true, "{result}");
    assert_eq!(
        out["refusals"][0]["entryId"], "A",
        "measured in the frontier of its entry: {result}"
    );
    let findings = out["findings"].as_array().expect("findings");
    assert_eq!(findings.len(), 1, "{result}");
    assert_eq!(
        findings[0]["deterministic_defect"]["code"], "invalid_json",
        "{result}"
    );
    assert_eq!(
        result["entries"],
        json!({}),
        "a refused entry is not kept: {result}"
    );
}

// F1: JSON.parse accepts an unpaired surrogate escape; serde_json refuses it.
// The round-3 binding re-parsed the entry with serde and threw, failing the run.
#[test]
fn unpaired_surrogates_are_an_invalid_json_refusal_not_a_throw() {
    let surrogate_body = r#"'{"id":"A","criterion":"","check":{"kind":"command","command":"x\\ud800y","cwd":"project_root"}}'"#;
    let trailing = r#"'{"id":"A","criterion":"","check":{"kind":"command","command":"test -f \\udc00","cwd":"project_root"}}'"#;
    let reversed = r#"'{"id":"A","criterion":"","check":{"kind":"command","command":"x","cwd":"\\udc00\\ud800"}}'"#;
    let key = r#"'{"id":"A","\\ud800":1,"criterion":"","check":{"kind":"command","command":"x","cwd":"project_root"}}'"#;
    for content in [surrogate_body, trailing, reversed, key] {
        assert_invalid_json(&author_a(content));
    }
}

// F1: the validator's other model-controlled limits. serde stops at depth 128;
// QuickJS's own stack stops deeper replies before or at serialisation.
#[test]
fn deep_nesting_is_refused_never_thrown() {
    for depth in [129, 200, 5_000, 200_000] {
        let content = format!(
            r#"'{{"id":"A","criterion":"","check":{{"kind":"command","command":"x","cwd":"project_root"}},"note":' + '['.repeat({depth}) + ']'.repeat({depth}) + '}}'"#
        );
        let result = author_a(&content);
        assert_eq!(
            result["error"],
            Value::Null,
            "depth {depth} threw: {result}"
        );
        assert_eq!(result["out"]["status"], "failed", "depth {depth}: {result}");
        assert_eq!(result["out"]["malformed"], true, "depth {depth}: {result}");
        assert_eq!(result["entries"], json!({}), "depth {depth}: {result}");
    }
    assert_invalid_json(&author_a(
        r#"'{"id":"A","criterion":"","check":{"kind":"command","command":"x","cwd":"project_root"},"note":' + '['.repeat(200) + ']'.repeat(200) + '}'"#,
    ));
}

// F1: a huge reply is validated, not thrown; a refusal never echoes the value.
#[test]
fn huge_replies_are_validated_and_refusals_stay_bounded() {
    let huge = "'x'.repeat(8 * 1024 * 1024)";
    let accepted = author_a(&format!(
        "JSON.stringify({{id:'A', check:{{...check, command:{huge}}}}})"
    ));
    assert_eq!(accepted["error"], Value::Null, "{accepted}");
    assert_eq!(accepted["out"]["status"], "accepted", "{accepted}");
    assert_eq!(
        accepted["entries"]["A"]["check"]["command"]
            .as_str()
            .map(str::len),
        Some(8 * 1024 * 1024)
    );
    let refused = author_a(&format!(
        "JSON.stringify({{id:'A', gap_permitted:42, note:{huge}, check:{{...check, command:[{huge}]}}}})"
    ));
    assert_eq!(
        refused["error"],
        Value::Null,
        "{}",
        refused.to_string().chars().take(500).collect::<String>()
    );
    assert_eq!(refused["out"]["status"], "failed");
    for finding in refused["out"]["findings"].as_array().expect("findings") {
        assert!(
            finding["text"].as_str().unwrap().len() < 1_000,
            "refusal echoes the value"
        );
    }
    let lone = author_a(&format!(
        r#"JSON.stringify({{id:'A', check:{{...check, command:{huge} + '\ud800'}}}})"#
    ));
    assert_invalid_json(&lone);
}

// F1: a JS string that is not valid UTF-8 (a raw unpaired surrogate) is
// content the validator refuses; only a non-string argument, or text that
// is not exactly one JSON value (it would validate another document), faults.
#[test]
fn binding_refuses_non_utf8_text_and_faults_only_on_misuse() {
    let result = run(r#"
        const raw = '{"id":"A","criterion":"\ud800","check":{"kind":"command","command":"x","cwd":"project_root"}}';
        const defects = JSON.parse(__archonValidateAcceptanceEntry('A', raw));
        let misuse = null;
        try { __archonValidateAcceptanceEntry('A', 42); } catch (e) { misuse = String(e.message); }
        let two = null;
        try { __archonValidateAcceptanceEntry('A', '{"id":"A"},{"id":"B"}'); } catch (e) { two = String(e.message); }
        return JSON.stringify({defects, misuse, two});
    "#);
    let defects = result["defects"].as_array().expect("defects");
    assert_eq!(defects.len(), 1, "{result}");
    assert_eq!(defects[0]["deterministic_defect"]["code"], "invalid_json");
    assert!(
        defects[0]["text"]
            .as_str()
            .unwrap()
            .starts_with("acceptance entry 'A' was refused: ")
    );
    assert!(
        result["misuse"].is_string(),
        "a non-string argument is a binding fault: {result}"
    );
    assert!(
        result["two"]
            .as_str()
            .is_some_and(|message| message.contains("not exactly one JSON value")),
        "{result}"
    );
}

// F1: the refusal is resumable: the loop re-authors the entry and freezes once.
#[test]
fn surrogate_reply_is_repaired_at_its_author_call() {
    let result = run(r#"
        const bad = '{"id":"A","check":{"kind":"command","command":"\\ud800","cwd":"project_root"}}';
        return JSON.stringify(await loop({A:[bad, valid('A')]}, {A:'a'}));
    "#);
    assert_eq!(result["error"], Value::Null, "{result}");
    assert_eq!(result["freezes"], 1);
    assert_eq!(result["prompts"].as_array().unwrap().len(), 2);
}

/// The texts of the round-1 refusal of `id` among `criteria` (+ `owed`).
fn refusal_texts(contents: &str, criteria: &str, owed: &str, id: &str) -> Vec<String> {
    let result = run(&format!(
        "return JSON.stringify(await author({contents}, {criteria}, {owed}));"
    ));
    assert_eq!(result["error"], Value::Null, "{result}");
    assert_eq!(result["out"]["refusals"][0]["entryId"], id, "{result}");
    let findings = result["out"]["findings"].as_array().expect("findings");
    for finding in findings {
        // The measurement identity keeps the native freeze pointer.
        let subject = finding["deterministic_defect"]["subject"].as_str().unwrap();
        assert!(subject.starts_with("entries/0/"), "{subject}");
    }
    findings
        .iter()
        .map(|finding| finding["text"].as_str().unwrap().to_string())
        .collect()
}

// F2: the feedback every pending sibling reads names the refused entry by id,
// never the envelope's `entries/0`, which in the freeze candidate is another
// (healthy) entry.
#[test]
fn refusal_text_names_the_entry_not_a_positional_pointer() {
    let sup = refusal_texts(
        r#"{A:valid('A'), 'SUP-REQ-X':JSON.stringify({id:'SUP-REQ-X', check:{...check, cwd:null}})}"#,
        "{A:'a'}",
        "[['SUP-REQ-X','REQ-X']]",
        "SUP-REQ-X",
    );
    assert_eq!(sup.len(), 1, "{sup:?}");
    assert!(
        sup[0].starts_with("acceptance entry 'SUP-REQ-X' was refused: check/cwd "),
        "{sup:?}"
    );
    let nested = refusal_texts(
        r#"{A:JSON.stringify({id:'A', criterion:'', check:{...check, command:42}})}"#,
        "{A:'a'}",
        "[]",
        "A",
    );
    assert_eq!(nested.len(), 1);
    assert!(
        nested[0].starts_with("acceptance entry 'A' was refused: check/command "),
        "{nested:?}"
    );
    let variant = refusal_texts(
        r#"{A:JSON.stringify({id:'A', criterion:'', check:{kind:'bogus', command:'x'}})}"#,
        "{A:'a'}",
        "[]",
        "A",
    );
    assert!(!variant.is_empty());
    for text in &variant {
        assert!(
            text.starts_with("acceptance entry 'A' was refused: check/"),
            "{text}"
        );
    }
    for text in sup.iter().chain(&nested).chain(&variant) {
        assert!(
            !text.contains("entries/0"),
            "positional pointer leaked: {text}"
        );
    }
}

// Round 5: a refusal is shown to the entry it names and to no sibling.
#[test]
fn each_entry_reads_only_its_own_refusal() {
    let result = run(r#"
        const missing = JSON.stringify({id:'A', check:{...check, command:42}});
        return JSON.stringify(await loop({A:[missing, valid('A')], B:[valid('B')]}, {A:'a', B:'b'}));
    "#);
    assert_eq!(result["error"], Value::Null, "{result}");
    assert_eq!(result["freezes"], 1);
    let prompts = result["prompts"].as_array().unwrap();
    let task = |id: &str, nth: usize| {
        prompts
            .iter()
            .filter(|p| p["id"] == id)
            .nth(nth)
            .expect("authored")["task"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let repair = task("A", 1);
    assert!(
        repair.contains("acceptance entry 'A' was refused: check/command "),
        "{repair}"
    );
    let sibling = task("B", 0);
    assert!(!sibling.contains("was refused"), "{sibling}");
    for text in [&repair, &sibling] {
        assert!(!text.contains("entries/0"), "{text}");
    }
}

/// Round-1 outcome of the owed supplementary entry `SUP-REQ-X` with `fields`.
fn supplementary(fields: &str) -> Value {
    run(&format!(
        "return JSON.stringify(await author({{A:valid('A'), 'SUP-REQ-X':JSON.stringify({{id:'SUP-REQ-X', check, {fields}}})}}, {{A:'a'}}, [['SUP-REQ-X','REQ-X']]));"
    ))
}

// F4 and round 5 (P3-A): an entry's criterion, and a supplementary entry's
// covers and gap_permitted, are host-owned and set before validation, so a
// reply the host would overwrite costs no repair.
#[test]
fn host_owned_fields_are_set_before_validation() {
    for (fields, covers) in [
        (r#"covers:'REQ-X', gap_permitted:'false'"#, json!(["REQ-X"])),
        (
            r#"covers:[42, 'REQ-Y', null], gap_permitted:null"#,
            json!(["REQ-X", "REQ-Y"]),
        ),
        ("", json!(["REQ-X"])),
    ] {
        let result = supplementary(fields);
        assert_eq!(result["error"], Value::Null, "{result}");
        assert_eq!(result["out"]["status"], "accepted", "{fields}: {result}");
        let entry = &result["entries"]["SUP-REQ-X"];
        assert_eq!(entry["covers"], covers, "{fields}");
        assert_eq!(entry["gap_permitted"], false, "{fields}");
        assert_eq!(
            entry["criterion"], "owed",
            "{fields}: the owed requirement text"
        );
    }
    // The R8 shape: no criterion, null, a number, or model text. Accepted on
    // the first reply with the host criterion (the validator would refuse the
    // first three, as freeze did before the host stamped it).
    for criterion in ["", "criterion:null,", "criterion:42,", "criterion:'model',"] {
        let result = author_a(&format!("JSON.stringify({{id:'A', {criterion} check}})"));
        assert_eq!(result["out"]["status"], "accepted", "{criterion}: {result}");
        assert_eq!(result["entries"]["A"]["criterion"], "a", "{criterion}");
    }
    // A field the host does not own is still validated as authored.
    let own = author_a(r#"JSON.stringify({id:'A', criterion:'', check, gap_permitted:'false'})"#);
    assert_eq!(own["out"]["status"], "failed", "{own}");
    assert_eq!(
        own["out"]["findings"][0]["deterministic_defect"]["subject"],
        "entries/0/gap_permitted"
    );
}

// Moved from workflow_freeze_shape.rs (round 3): the binding reports freeze's
// identities and stages unchanged, with the entry named in the text.
fn preserves(entry: Value) {
    let runtime = rquickjs::Runtime::new().unwrap();
    let context = rquickjs::Context::full(&runtime).unwrap();
    context.with(|ctx| {
        install_entry_validator(&ctx).unwrap();
        let native: rquickjs::Function = ctx
            .globals()
            .get("__archonValidateAcceptanceEntry")
            .unwrap();
        let actual: String = native.call(("A", entry.to_string())).unwrap();
        let actual: Value = serde_json::from_str(&actual).unwrap();
        let candidate = serde_json::to_vec(&json!({"entries":[entry]})).unwrap();
        let expected: Vec<_> = element_shape_defects(&candidate, &ENTRY_SHAPE)
            .into_iter()
            .map(|defect| {
                json!({
                    "text":refusal_text("A", &defect),
                    "deterministic_defect":defect.identity,
                })
            })
            .collect();
        assert!(!expected.is_empty());
        assert_eq!(
            actual,
            json!(expected),
            "author must preserve freeze identities and stages"
        );
    });
}

#[test]
fn author_validator_preserves_missing_criterion() {
    preserves(
        json!({"id":"A","check":{"kind":"command","command":"test -f output","cwd":"project_root"}}),
    );
}

#[test]
fn author_validator_preserves_nested_command_fields() {
    preserves(json!({"id":"A","criterion":"","check":{"kind":"command","command":42,"cwd":null}}));
}

#[test]
fn author_validator_preserves_gap_and_covers_elements() {
    preserves(
        json!({"id":"A","criterion":"","gap_permitted":"false","covers":[42,null],
        "check":{"kind":"command","command":"test -f output","cwd":"project_root"}}),
    );
}

#[test]
fn native_author_shape_repairs_decrease_five_to_zero_without_pause() {
    let runtime = rquickjs::Runtime::new().unwrap();
    let context = rquickjs::Context::full(&runtime).unwrap();
    context.with(|ctx| {
        install_entry_validator(&ctx).unwrap();
        let script = format!(
            r#"const args = {{repositoryRoot:'/archon-test-roots/repo',projectRoot:'/archon-test-roots/project',acceptanceCriteria:{{A:'a'}},authorMaxParallelism:1,gateMode:'enforce'}};
            {source}
            (async () => {{
                let calls = 0, freezes = 0;
                const w = {{
                    agent: async () => {{
                        if (++calls > 6) throw Error('bounded test exhausted');
                        return {{status:'accepted',stopReason:'end_turn',content:JSON.stringify({{
                            id:'A',
                            check:{{kind:'command',command:calls >= 2 ? 'test -f output' : 42,
                                cwd:calls >= 3 ? 'project_root' : null}},
                            gap_permitted:calls >= 4 ? false : 'false',
                            covers:calls >= 6 ? [] : calls >= 5 ? ['REQ-Y', null] : [42, null]
                        }})}};
                    }},
                    hostCommand: async () => {{freezes++; return {{publicationReceipt:{{call_id:'freeze'}},
                        postcondition:{{satisfied:true}},gateEnvelope:{{policy_findings:[]}}}};}},
                    pause: async () => {{throw Error('unexpected pause');}}
                }};
                await authorCandidate(w, {{phase:'acceptance',prompt:()=> 'author',
                    author:authorAcceptanceEntries,capability:'freeze-acceptance',
                    retryScopes:new Set(['candidate_artifact'])}});
                return JSON.stringify({{calls,freezes}});
            }})()"#,
            source = crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE,
        );
        let promise: rquickjs::Promise = ctx.eval(script).unwrap();
        let result: String = promise.finish().unwrap();
        let result: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result, json!({"calls":6,"freezes":1}));
    });
}
