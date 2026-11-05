// Node stand-in for the host's __archonAuthorContext binding (Issue 288,
// crates/archon-workflow/src/v2/script/author_context.rs): the same SHA-256
// naming and the same reply, with the files kept in memory.
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const policySource = fs.readFileSync(path.join(__dirname, '../../crates/archon-workflow/src/task_universe_contract_audit.rs'), 'utf8');
const contractRule = JSON.parse(policySource.match(/pub const DELIVERABLE_CONTRACT_RULE: &str = ("[^\n]+");/)[1]);

function authorContext(files = new Map()) {
  const binding = (extension, text) => {
    if (!['json', 'jsonl', 'txt'].includes(extension)) throw new Error(`author context extension '${extension}' is not one of json, jsonl, txt`);
    const sha256 = crypto.createHash('sha256').update(String(text), 'utf8').digest('hex');
    const path = `/run/author-context/${sha256}.${extension}`;
    files.set(path, String(text));
    return JSON.stringify({ path, sha256 });
  };
  binding.files = files;
  return binding;
}

// Explicit Node-only stand-in for the native redactor binding. This small
// fixture covers the shapes exercised by these harnesses; Rust/QuickJS tests
// compare the product script against the complete host redactor corpus.
function redactPublicTextFixture(value) {
  return String(value)
    .replace(/(["']?(?:x-api-key|api-key)["']\s*:\s*)"[^"\r\n]*"/giu, '$1"<redacted>"')
    .replace(/(?<![\p{L}\p{N}_])authorization:[^\s]*/giu, '<redacted>')
    .replace(/(?<![\p{L}\p{N}_])sk-ant-[A-Za-z0-9_-]{20,}/giu, '<redacted>')
    .replace(/(?<![\p{L}\p{N}_])sk-[A-Za-z0-9_-]{17,}/giu, '<redacted>')
    .replace(/["']?(?:x-api-key|api-key|api[_-]?key|apikey|token|secret|password)\s*[:=]\s*[^\s,;}"']+/giu, '<redacted>')
    .replace(/(?<![\p{L}\p{N}_])(bearer\s+)\S+/giu, '$1<redacted>')
    .replace(/((?:x-api-key|api-key)\s+)[^\s]+/giu, '$1<redacted>');
}

// `context` with the binding installed, unless the test installed its own.
function withAuthorContext(context) {
  if (context.args && !Object.hasOwn(context.args, "deliverableContractRule")) context.args.deliverableContractRule = contractRule;
  if (!Object.prototype.hasOwnProperty.call(context, '__archonAuthorContext')) context.__archonAuthorContext = authorContext();
  if (!Object.prototype.hasOwnProperty.call(context, '__archonRedactPublicText')) context.__archonRedactPublicText = redactPublicTextFixture;
  return context;
}

module.exports = { authorContext, withAuthorContext, redactPublicTextFixture };
