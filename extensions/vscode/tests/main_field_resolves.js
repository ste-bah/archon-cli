"use strict";

const assert = require("assert");
const fs = require("fs");
const Module = require("module");
const path = require("path");

const extensionRoot = path.join(__dirname, "..");
const manifestPath = path.join(extensionRoot, "package.json");
const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
const mainPath = path.resolve(extensionRoot, manifest.main);

assert.strictEqual(
  manifest.main,
  "./dist/src/extension.js",
  "package.json main must match the emitted TypeScript entry point"
);
assert.ok(fs.existsSync(mainPath), `main entry point missing: ${mainPath}`);

const originalLoad = Module._load;
Module._load = function patchedLoad(request, parent, isMain) {
  if (request === "vscode") {
    return {
      CodeActionKind: { Empty: "empty" },
      EventEmitter: class {
        constructor() {
          this.event = () => ({ dispose() {} });
        }
        fire() {}
        dispose() {}
      },
      Selection: class {
        constructor(start, end) {
          this.start = start;
          this.end = end;
          this.isEmpty = start === end;
        }
      },
      StatusBarAlignment: { Right: 2 },
      Uri: {
        joinPath(uri, ...parts) {
          return { fsPath: path.join(uri?.fsPath ?? "", ...parts) };
        },
      },
      ViewColumn: { One: 1, Beside: 2 },
      window: {
        activeTextEditor: undefined,
        createStatusBarItem() {
          return {
            show() {},
            dispose() {},
          };
        },
        createWebviewPanel() {
          return {
            webview: {
              html: "",
              onDidReceiveMessage() {},
              postMessage() {},
            },
            onDidDispose() {},
            reveal() {},
            dispose() {},
          };
        },
        showErrorMessage() {
          return Promise.resolve(undefined);
        },
        showWarningMessage() {
          return Promise.resolve(undefined);
        },
      },
      commands: {
        registerCommand() {
          return { dispose() {} };
        },
      },
      languages: {
        registerCodeActionsProvider() {
          return { dispose() {} };
        },
        registerInlineCompletionItemProvider() {
          return { dispose() {} };
        },
      },
      CodeAction: class {
        constructor(title, kind) {
          this.title = title;
          this.kind = kind;
        }
      },
    };
  }
  return originalLoad.call(this, request, parent, isMain);
};

try {
  const loaded = require(mainPath);
  assert.strictEqual(typeof loaded.activate, "function");
  assert.strictEqual(typeof loaded.deactivate, "function");
  console.log("  PASS  main_field_resolves: package main exists and loads");
} finally {
  Module._load = originalLoad;
}
