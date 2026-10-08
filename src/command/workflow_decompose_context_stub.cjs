// Node stand-in for the host's __archonAuthorContext binding (Issue 288,
// crates/archon-workflow/src/v2/script/author_context.rs): the same SHA-256
// naming and the same reply, with the files kept in memory.
const crypto = require('node:crypto');

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

// `context` with the binding installed, unless the test installed its own.
function withAuthorContext(context) {
  if (!Object.prototype.hasOwnProperty.call(context, '__archonAuthorContext')) context.__archonAuthorContext = authorContext();
  return context;
}

module.exports = { authorContext, withAuthorContext };
