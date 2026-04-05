/**
 * Unit tests for Archon VS Code Extension — no vscode runtime required.
 *
 * These tests import ONLY from the extension's own source files and verify
 * pure-logic behaviour: constants, types, serialization, and formatting.
 * They are executed by tests/run_tests.js via the compiled dist/ output.
 */

import * as assert from "assert";
import { ConnectionMode, COMMANDS, CODE_ACTION_TITLES, CONFIG_KEY_CONNECTION_MODE } from "../src/constants";
import {
  formatStatusText,
  DEFAULT_WS_CONFIG,
  InitializeMessage,
  PromptMessage,
} from "../src/types";

// ── Test helpers ──────────────────────────────────────────────────────────────

let passed = 0;
let failed = 0;

function test(name: string, fn: () => void): void {
  try {
    fn();
    console.log(`  PASS  ${name}`);
    passed++;
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    console.log(`  FAIL  ${name}`);
    console.log(`        ${message}`);
    failed++;
  }
}

// ── Test 1: connection_mode_default ───────────────────────────────────────────

test("connection_mode_default: ConnectionMode.Stdio is default value", () => {
  assert.strictEqual(ConnectionMode.Stdio, "stdio");
});

// ── Test 2: connection_config_websocket_url ───────────────────────────────────

test("connection_config_websocket_url: WsConnectionConfig has correct default URL", () => {
  assert.strictEqual(DEFAULT_WS_CONFIG.url, "ws://localhost:8420/ws/ide");
});

// ── Test 3: message_serialize_prompt ─────────────────────────────────────────

test("message_serialize_prompt: PromptMessage serializes to JSON with method=archon/prompt", () => {
  const msg: PromptMessage = {
    jsonrpc: "2.0",
    id: 1,
    method: "archon/prompt",
    params: { sessionId: "test-session", text: "Hello Archon" },
  };
  const json = JSON.stringify(msg);
  const parsed = JSON.parse(json) as Record<string, unknown>;
  assert.strictEqual(parsed["method"], "archon/prompt");
  assert.strictEqual((parsed["params"] as Record<string, unknown>)["text"], "Hello Archon");
});

// ── Test 4: message_serialize_initialize ─────────────────────────────────────

test("message_serialize_initialize: InitializeMessage has required fields", () => {
  const msg: InitializeMessage = {
    jsonrpc: "2.0",
    id: 1,
    method: "archon/initialize",
    params: {
      clientInfo: { name: "archon-vscode", version: "0.1.0" },
      capabilities: {
        inlineCompletion: true,
        toolExecution: true,
        diff: true,
        terminal: true,
      },
    },
  };
  assert.strictEqual(msg.jsonrpc, "2.0");
  assert.ok(typeof msg.id === "number");
  assert.strictEqual(msg.method, "archon/initialize");
  assert.ok(msg.params.clientInfo.name.length > 0);
  assert.ok(typeof msg.params.capabilities.inlineCompletion === "boolean");
});

// ── Test 5: status_bar_idle_text ──────────────────────────────────────────────

test("status_bar_idle_text: formatStatusText('idle') returns expected string", () => {
  const text = formatStatusText("idle");
  assert.ok(text.includes("idle"), `Expected 'idle' in "${text}"`);
});

// ── Test 6: status_bar_connected_text ────────────────────────────────────────

test("status_bar_connected_text: formatStatusText('connected') includes 'Archon'", () => {
  const text = formatStatusText("connected");
  assert.ok(text.includes("Archon"), `Expected 'Archon' in "${text}"`);
});

// ── Test 7: code_action_titles ────────────────────────────────────────────────

test("code_action_titles: CODE_ACTION_TITLES array has at least 4 entries", () => {
  assert.ok(
    CODE_ACTION_TITLES.length >= 4,
    `Expected >= 4 entries, got ${CODE_ACTION_TITLES.length}`
  );
  assert.ok(CODE_ACTION_TITLES.includes("Ask Archon"));
  assert.ok(CODE_ACTION_TITLES.includes("Explain Code"));
  assert.ok(CODE_ACTION_TITLES.includes("Fix Error"));
  assert.ok(CODE_ACTION_TITLES.includes("Generate Tests"));
});

// ── Test 8: webview_html_has_form ────────────────────────────────────────────

test("webview_html_has_form: ChatWebviewHtml contains form and input elements", () => {
  // Read the webview HTML directly (not importing vscode)
  const fs = require("fs") as typeof import("fs");
  const path = require("path") as typeof import("path");
  // __dirname is dist/tests/ after compilation; go up two levels to reach extension root
  const htmlPath = path.join(__dirname, "..", "..", "src", "chat", "webview.html");
  const html = fs.readFileSync(htmlPath, "utf8");
  assert.ok(html.includes("<form"), `Expected '<form' in webview.html`);
  assert.ok(html.includes("input") || html.includes("textarea"), `Expected 'input' or 'textarea' in webview.html`);
});

// ── Test 9: config_key_connection_mode ───────────────────────────────────────

test("config_key_connection_mode: CONFIG_KEY_CONNECTION_MODE constant is defined", () => {
  assert.strictEqual(typeof CONFIG_KEY_CONNECTION_MODE, "string");
  assert.ok(CONFIG_KEY_CONNECTION_MODE.length > 0);
  assert.strictEqual(CONFIG_KEY_CONNECTION_MODE, "archon.connectionMode");
});

// ── Test 10: archon_command_ids ───────────────────────────────────────────────

test("archon_command_ids: COMMANDS object has at least 4 command ID strings", () => {
  const commandValues = Object.values(COMMANDS);
  assert.ok(
    commandValues.length >= 4,
    `Expected >= 4 commands, got ${commandValues.length}`
  );
  for (const cmd of commandValues) {
    assert.ok(
      typeof cmd === "string" && cmd.startsWith("archon."),
      `Command "${cmd}" must be a string starting with "archon."`
    );
  }
});

// ── Summary ───────────────────────────────────────────────────────────────────

console.log(`\nResults: ${passed} passed, ${failed} failed`);
if (failed > 0) {
  process.exit(1);
}
