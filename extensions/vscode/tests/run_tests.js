/**
 * Plain Node.js test runner for the Archon VS Code extension.
 *
 * Requires the TypeScript source to be compiled first with `npm run build`.
 * Missing compiled output is a hard failure so CI cannot silently skip tests.
 *
 * Usage:
 *   node tests/run_tests.js
 */

"use strict";

const path = require("path");
const fs = require("fs");

const compiledTests = path.join(__dirname, "..", "dist", "tests", "extension_tests.js");

if (!fs.existsSync(compiledTests)) {
  console.error("Archon extension tests: compiled output not found.");
  console.error("Run `npm run build` before `npm test`.");
  console.error(`Expected: ${compiledTests}`);
  process.exit(1);
}

require("./main_field_resolves");
require(compiledTests);
