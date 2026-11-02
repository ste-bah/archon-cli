//! Byte equivalence and native-work regressions for the bounded context helpers.
use super::decl;

fn run(driver: &str) {
    let script = [
        decl("AUTHOR_PROMPT_LIMIT"),
        decl("utf8Bytes"),
        decl("namedFilesContext"),
        driver.to_owned(),
    ]
    .join("\n");
    let output = std::process::Command::new("node")
        .arg("-e")
        .arg(script)
        .output()
        .expect("node must be available");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn workflow_decompose_utf8_bytes_matches_old_oracle_without_string_iteration() {
    run(r#"
const assert = require('node:assert/strict');
function oldUtf8Bytes(text) {
  let bytes = 0;
  for (const char of String(text)) {
    const code = char.codePointAt(0);
    bytes += code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
  }
  return bytes;
}
const cases = ["", "ASCII", "\u0080\u07ff", "\u0800\uffff", "\ud800\udc00",
  "\ud800", "\udc00", "\udc00\ud800", "aé中😀\ud800!\udc00".repeat(18070) + "x".repeat(10)];
// The mixed case is exactly 300 KiB under the original byte semantics.
assert.equal(oldUtf8Bytes(cases.at(-1)), 300 * 1024);
// Cover every single UTF-16 unit and every possible valid surrogate pair.
for (let unit = 0; unit <= 0xffff; unit++) cases.push(String.fromCharCode(unit));
for (const value of cases) assert.equal(utf8Bytes(value), oldUtf8Bytes(value));
for (let high = 0xd800; high <= 0xdbff; high++) {
  const pairs = [];
  for (let low = 0xdc00; low <= 0xdfff; low++) pairs.push(String.fromCharCode(high, low));
  const value = pairs.join("");
  assert.equal(utf8Bytes(value), oldUtf8Bytes(value));
}
for (const value of [null, undefined, 123, {toString: () => "é😀"}]) {
  assert.equal(utf8Bytes(value), oldUtf8Bytes(value));
}
String.prototype[Symbol.iterator] = function () { throw new Error("interpreted string iteration"); };
assert.equal(utf8Bytes("aé中😀\ud800!\udc00"), 17);
"#);
}

#[test]
fn workflow_decompose_named_files_matches_old_oracle_with_linear_serialization() {
    run(r#"
const assert = require('node:assert/strict');
function oldNamedFilesContext(task, entry, notes, tail = "") {
  const base = `${task}\nAnswer once you have read the supplied named_files content for the deciding file, or inspected that file at its named path yourself.\nFiles already identified for this check are supplied here when available: `;
  if (utf8Bytes(base) + utf8Bytes("[]") + utf8Bytes(tail) > AUTHOR_PROMPT_LIMIT) return `${task}${tail}`;
  const room = Math.max(0, AUTHOR_PROMPT_LIMIT - utf8Bytes(base) - utf8Bytes(tail));
  const supplied = typeof __archonNamedFiles === "function"
    ? JSON.parse(__archonNamedFiles(JSON.stringify(entry), JSON.stringify(notes), room)) : [];
  const files = [];
  for (const file of supplied) {
    let shown = file;
    if (utf8Bytes(JSON.stringify([...files, shown])) > room) shown = {path:file.path, status:"not_supplied"};
    if (utf8Bytes(JSON.stringify([...files, shown])) > room) break;
    files.push(shown);
  }
  return `${base}${JSON.stringify(files)}${tail}`;
}
let supplied = [];
globalThis.__archonNamedFiles = () => JSON.stringify(supplied);
const files = [
  {path: "aé😀\ud800", status: "supplied", content: 'é中😀\n"\\\udc00'},
  {path: "second", status: "supplied", content: "x".repeat(100)},
];
function compare(task = "task", tail = "\nreply format") {
  const actual = namedFilesContext(task, {}, [], tail);
  assert.equal(actual, oldNamedFilesContext(task, {}, [], tail));
  return actual;
}
compare(); // No files.
supplied = files;
assert.ok(compare().includes('"path":"second"')); // All fit.
supplied = [{path:"large", content:"x".repeat(AUTHOR_PROMPT_LIMIT)}, files[0]];
assert.ok(compare().includes('{"path":"large","status":"not_supplied"}'));
// Exercise exact array boundaries, including comma and both brackets.
supplied = [];
const prefixBytes = utf8Bytes(oldNamedFilesContext("", {}, [], "")) - 2;
supplied = files;
for (const room of [1, 2, 3, utf8Bytes(JSON.stringify([files[0]])) - 1,
  utf8Bytes(JSON.stringify([files[0]])), utf8Bytes(JSON.stringify(files)) - 1,
  utf8Bytes(JSON.stringify(files))]) {
  compare("x".repeat(AUTHOR_PROMPT_LIMIT - prefixBytes - room), "");
}
// First fits exactly; even the replacement for the second cannot fit: stop.
const room = utf8Bytes(JSON.stringify([files[0]]));
const stopped = compare("x".repeat(AUTHOR_PROMPT_LIMIT - prefixBytes - room), "");
assert.ok(stopped.endsWith(JSON.stringify([files[0]])));
delete globalThis.__archonNamedFiles;
compare(); // Missing host binding.
// Count serialized objects only inside the helper, excluding the host transport.
supplied = Array.from({length:100}, (_, i) => ({path:String(i), content:"é".repeat(100)}));
const transport = JSON.stringify(supplied);
globalThis.__archonNamedFiles = () => transport;
const stringify = JSON.stringify;
let objects = 0;
JSON.stringify = (value) => stringify(value, (key, item) => {
  if (item && typeof item === "object" && "path" in item) objects++;
  return item;
});
namedFilesContext("task", {}, [], "");
assert.equal(objects, supplied.length, `serialized ${objects} file objects`);
"#);
}
