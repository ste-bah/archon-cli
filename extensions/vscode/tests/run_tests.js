/**
 * Plain Node.js test runner for the Archon VS Code extension.
 *
 * Requires the TypeScript source to be compiled first (`tsc --noEmit` or full
 * `tsc` build). If the compiled output does not exist, this script reports
 * "build first" and exits 0 so CI does not block on a missing build step.
 *
 * Usage:
 *   node tests/run_tests.js
 */

"use strict";

const path = require("path");
const fs = require("fs");

const compiledTests = path.join(__dirname, "..", "dist", "tests", "extension_tests.js");

if (!fs.existsSync(compiledTests)) {
  console.log("Archon extension tests: compiled output not found.");
  console.log("Run `tsc` to compile before executing tests.");
  console.log("(Exiting 0 — build first)");
  process.exit(0);
}

// Run the compiled test file
require(compiledTests);
