/**
 * Unit tests for Archon VS Code Extension — no vscode runtime required.
 *
 * These tests import ONLY from the extension's own source files and verify
 * pure-logic behaviour: constants, types, serialization, and formatting.
 * They are executed by tests/run_tests.js via the compiled dist/ output.
 */

import * as assert from "assert";
import {
  ConnectionMode,
  COMMANDS,
  CODE_ACTION_TITLES,
  CONFIG_KEY_CONNECTION_MODE,
  PERMISSION_MODES,
} from "../src/constants";
import {
  formatStatusText,
  DEFAULT_WS_CONFIG,
  InitializeMessage,
  PermissionResponseMessage,
  PromptMessage,
  SessionStatus,
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

// ── Test 10: connection_manager_initial_state ────────────────────────────────

test("connection_manager_initial_state: getState='idle', getSessionId=null before connect", () => {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { ConnectionManager } = require("../src/connection/manager") as typeof import("../src/connection/manager");
  const mgr = new ConnectionManager();
  assert.strictEqual(mgr.getState(), "idle");
  assert.strictEqual(mgr.getSessionId(), null);
});

// ── Test 11: archon_command_ids ───────────────────────────────────────────────

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

// ── Test 12: permission_response_serialize ────────────────────────────────────

test("permission_response_serialize: carries the requestId the backend correlates on", () => {
  const msg: PermissionResponseMessage = {
    jsonrpc: "2.0",
    id: 7,
    method: "archon/permissionResponse",
    params: { sessionId: "s", requestId: "perm-3", approved: false },
  };
  const parsed = JSON.parse(JSON.stringify(msg)) as Record<string, unknown>;
  const params = parsed["params"] as Record<string, unknown>;
  assert.strictEqual(parsed["method"], "archon/permissionResponse");
  assert.strictEqual(params["requestId"], "perm-3");
  assert.strictEqual(params["approved"], false);
});

// ── Test 13: capabilities_claim_an_approval_ui ────────────────────────────────

test("capabilities_claim_an_approval_ui: initialize advertises toolExecution", () => {
  // The backend reads this as "there is somebody here who can answer a
  // permission prompt". Advertising false means every request is refused on
  // arrival, so the chat panel's allow/deny buttons would never appear.
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { ConnectionManager } = require("../src/connection/manager") as typeof import("../src/connection/manager");
  const source = require("fs").readFileSync(
    require("path").join(__dirname, "..", "..", "src", "connection", "manager.ts"),
    "utf8"
  ) as string;
  assert.ok(typeof ConnectionManager === "function");
  assert.ok(
    /toolExecution:\s*true/.test(source),
    "DEFAULT_CAPABILITIES must advertise toolExecution: true"
  );
});

// ── Test 14: permission_notification_dispatch ─────────────────────────────────

test("permission_notification_dispatch: archon/permissionRequest reaches the callback", () => {
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { ConnectionManager } = require("../src/connection/manager") as typeof import("../src/connection/manager");
  const mgr = new ConnectionManager();
  const seen: { requestId: string; action: string }[] = [];
  mgr.onPermissionRequest = (request) => {
    seen.push({ requestId: request.requestId, action: request.action });
  };

  // The private inbound path is the unit under test; there is no transport in
  // a plain-Node test to push a frame through.
  (mgr as unknown as { _handleMessage: (data: string) => void })._handleMessage(
    JSON.stringify({
      jsonrpc: "2.0",
      method: "archon/permissionRequest",
      params: {
        sessionId: "s",
        requestId: "perm-1",
        action: "Bash",
        description: "run a command",
      },
    })
  );

  assert.strictEqual(seen.length, 1, "permission request was dropped");
  assert.strictEqual(seen[0]!.requestId, "perm-1");
  assert.strictEqual(seen[0]!.action, "Bash");
});

// ── Test 15: permission_modes_exclude_auto ────────────────────────────────────

test("permission_modes_exclude_auto: the mode picker never offers a mode that cannot prompt", () => {
  assert.ok(PERMISSION_MODES.includes("default"));
  assert.ok(
    !PERMISSION_MODES.includes("auto"),
    "auto never raises a permission prompt, so offering it hides the approval UI"
  );
  assert.ok(
    !PERMISSION_MODES.includes("bypassPermissions"),
    "bypassPermissions must not be a one-click option in the editor"
  );
});

// ── Test 16: status_absence_is_representable ──────────────────────────────────

test("status_absence_is_representable: SessionStatus can say 'no reading' without zeros", () => {
  const status: SessionStatus = {
    model: "claude-sonnet-4-6",
    unavailable: "no turn has completed in this session yet",
  };
  assert.strictEqual(status.inputTokens, undefined);
  assert.ok(status.unavailable && status.unavailable.length > 0);
});

// ── Test 17: webview_renders_the_permission_prompt ────────────────────────────

test("webview_renders_the_permission_prompt: allow/deny buttons and their wiring exist", () => {
  const fs = require("fs") as typeof import("fs");
  const path = require("path") as typeof import("path");
  const htmlPath = path.join(__dirname, "..", "..", "src", "chat", "webview.html");
  const html = fs.readFileSync(htmlPath, "utf8");
  assert.ok(html.includes('id="permission-allow"'), "missing Allow button");
  assert.ok(html.includes('id="permission-deny"'), "missing Deny button");
  assert.ok(
    html.includes("permissionDecision"),
    "buttons must post a permissionDecision back to the host"
  );
  assert.ok(
    html.includes("case 'permissionRequest'"),
    "webview must handle the permissionRequest message"
  );
  assert.ok(
    html.includes("case 'toolCall'") && html.includes("case 'toolResult'"),
    "webview must render tool activity"
  );
  assert.ok(
    html.includes("case 'thinkingDelta'"),
    "webview must render thinking deltas"
  );
});

// ── Test 18: stdio_spawn_hides_the_console_window ─────────────────────────────

test("stdio_spawn_hides_the_console_window: connectStdio spawns with windowsHide", () => {
  // On Windows, spawning a console-subsystem binary without `windowsHide` pops
  // a conhost window on every connect and reconnect. It renders blank — the
  // process speaks JSON-RPC over stdio, not a TUI — so it reads as a hung
  // Archon, and closing it kills the extension backend. The flag is a no-op on
  // other platforms, so nothing but this test stops a refactor from dropping
  // it on a machine where the symptom is invisible. (#159)
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { ConnectionManager } = require("../src/connection/manager") as typeof import("../src/connection/manager");
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const childProcess = require("child_process") as typeof import("child_process");

  const spawnSlot = childProcess as unknown as { spawn: unknown };
  const realSpawn = spawnSlot.spawn;

  let capturedOptions: Record<string, unknown> | undefined;

  // Reports the spawn as failed the moment the manager subscribes to "error",
  // so connectStdio rejects instead of hanging on an initialize handshake that
  // no real process is there to answer.
  const fakeChild = {
    stdout: { on: (): void => {} },
    stdin: { write: (): void => {}, destroyed: true },
    on(event: string, handler: (arg: unknown) => void): unknown {
      if (event === "error") {
        handler(new Error("spawn intercepted by test"));
      }
      return this;
    },
    kill: (): void => {},
  };

  spawnSlot.spawn = (
    _binary: string,
    _args: string[],
    options: Record<string, unknown>
  ): unknown => {
    capturedOptions = options;
    return fakeChild;
  };

  try {
    // spawn() runs synchronously inside connectStdio's promise executor, so
    // capturedOptions is populated by the time this expression returns.
    void new ConnectionManager()
      .connectStdio("archon", ConnectionMode.Stdio, "archon-test-workspace")
      .catch(() => undefined);
  } finally {
    spawnSlot.spawn = realSpawn;
  }

  assert.ok(
    capturedOptions !== undefined,
    "connectStdio never called child_process.spawn"
  );
  const options = capturedOptions as Record<string, unknown>;
  assert.strictEqual(
    options["windowsHide"],
    true,
    "stdio spawn must pass windowsHide: true, or every connect flashes a console window that steals focus from VS Code (#159)"
  );
});

// ── Test 19: stdio_stderr_reaches_the_log_sink ────────────────────────────────

test("stdio_stderr_reaches_the_log_sink: backend stderr is piped to onBackendLog", () => {
  // Second half of #159: with windowsHide there is no console, so inheriting
  // stderr would throw away every backend diagnostic and leave a crashed
  // backend showing as nothing but "Archon: error". stderr must be piped and
  // routed to the sink the activation side points at its output channel.
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const { ConnectionManager } = require("../src/connection/manager") as typeof import("../src/connection/manager");
  // eslint-disable-next-line @typescript-eslint/no-require-imports
  const childProcess = require("child_process") as typeof import("child_process");

  const spawnSlot = childProcess as unknown as { spawn: unknown };
  const realSpawn = spawnSlot.spawn;

  let capturedStdio: unknown;
  let stderrDataHandler: ((chunk: Buffer) => void) | undefined;

  const fakeChild = {
    stdout: { on: (): void => {} },
    stderr: {
      on(event: string, handler: (chunk: Buffer) => void): void {
        if (event === "data") {
          stderrDataHandler = handler;
        }
      },
    },
    stdin: { write: (): void => {}, destroyed: true },
    on(event: string, handler: (arg: unknown) => void): unknown {
      // As in test 18: fail the spawn so connectStdio rejects rather than
      // hanging. The stderr handler is registered before this fires.
      if (event === "error") {
        handler(new Error("spawn intercepted by test"));
      }
      return this;
    },
    kill: (): void => {},
  };

  spawnSlot.spawn = (
    _binary: string,
    _args: string[],
    options: Record<string, unknown>
  ): unknown => {
    capturedStdio = options["stdio"];
    return fakeChild;
  };

  const manager = new ConnectionManager();
  const logged: string[] = [];
  manager.onBackendLog = (text) => logged.push(text);

  try {
    void manager
      .connectStdio("archon", ConnectionMode.Stdio, "archon-test-workspace")
      .catch(() => undefined);
  } finally {
    spawnSlot.spawn = realSpawn;
  }

  // stderr must be piped — "inherit" sends it to a console that does not exist.
  assert.ok(Array.isArray(capturedStdio), "spawn was given no stdio array");
  assert.strictEqual(
    (capturedStdio as unknown[])[2],
    "pipe",
    "stderr must be piped, not inherited: windowsHide leaves no console for inherited output (#159)"
  );

  // And the piped data must actually reach the sink.
  assert.ok(
    stderrDataHandler !== undefined,
    "connectStdio never subscribed to child stderr"
  );
  const emit = stderrDataHandler as (chunk: Buffer) => void;
  emit(Buffer.from("panicked at 'backend exploded'\n", "utf8"));

  assert.ok(
    logged.some((line) => line.includes("panicked at 'backend exploded'")),
    `backend stderr never reached onBackendLog; got ${JSON.stringify(logged)}`
  );
});

// ── Summary ───────────────────────────────────────────────────────────────────

console.log(`\nResults: ${passed} passed, ${failed} failed`);
if (failed > 0) {
  process.exit(1);
}
