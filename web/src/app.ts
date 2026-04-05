/// Archon Web UI — application entry point.
///
/// Wires together all UI components and the WebSocket connection.

import { ArchonConnection, ConnectionState } from "./connection.js";
import { ChatView } from "./chat.js";
import { InputArea } from "./input.js";
import { SessionList } from "./session.js";
import { SettingsPanel } from "./settings.js";

function qs<T extends HTMLElement>(sel: string): T {
  const el = document.querySelector<T>(sel);
  if (!el) throw new Error(`Element not found: ${sel}`);
  return el;
}

function updateStatusConn(el: HTMLElement, state: ConnectionState): void {
  el.textContent =
    state === "connected"
      ? "Connected"
      : state === "connecting"
        ? "Connecting…"
        : "Disconnected";
  el.className =
    state === "connected"
      ? "conn-connected"
      : state === "connecting"
        ? "conn-connecting"
        : "conn-disconnected";
}

async function main(): Promise<void> {
  // Determine WebSocket URL from current location
  const proto = location.protocol === "https:" ? "wss" : "ws";
  const wsUrl = `${proto}://${location.host}/ws/ide`;

  // Extract bearer token from URL search params (set by server when auth required)
  const params = new URLSearchParams(location.search);
  const token = params.get("token");

  // DOM refs
  const messagesEl = qs<HTMLDivElement>("#messages");
  const statusConnEl = qs<HTMLSpanElement>("#status-conn");
  const statusModelEl = qs<HTMLSpanElement>("#status-model");
  const sessionListEl = qs<HTMLUListElement>("#session-list");
  const newSessionBtn = qs<HTMLButtonElement>("#new-session-btn");
  const chatForm = qs<HTMLFormElement>("#chat-form");
  const settingsPanelEl = qs<HTMLDivElement>("#settings-panel");
  const settingsOpenBtn = qs<HTMLButtonElement>("#settings-open-btn");

  // Components
  const chat = new ChatView(messagesEl);
  const input = new InputArea(chatForm);
  const sessions = new SessionList(sessionListEl, newSessionBtn);
  const settings = new SettingsPanel(settingsPanelEl, settingsOpenBtn);
  const conn = new ArchonConnection(wsUrl, token);

  // Load persisted settings
  try {
    const saved = localStorage.getItem("archon-settings");
    if (saved) settings.load(JSON.parse(saved) as Record<string, string>);
  } catch { /* ignore */ }

  // Wire: connection state
  conn.onState((state) => {
    updateStatusConn(statusConnEl, state);
    input.setEnabled(state === "connected");
  });

  input.setEnabled(false);

  // Wire: streaming text
  conn.onTextDelta((delta) => {
    chat.appendTextDelta(delta);
  });

  conn.onTurnComplete(() => {
    chat.finishAssistantMessage();
    input.setEnabled(true);
    input.focus();
  });

  // Wire: send prompt
  input.onSubmit(async (text) => {
    if (!text) return;
    chat.addUserMessage(text);
    chat.startAssistantMessage();
    input.setEnabled(false);
    try {
      await conn.sendPrompt(text);
    } catch (err) {
      chat.appendTextDelta(`Error: ${(err as Error).message}`);
      chat.finishAssistantMessage();
      input.setEnabled(true);
    }
  });

  // Wire: session change → clear chat
  sessions.onChange((_id) => {
    chat.clear();
    input.focus();
  });

  // Wire: settings save
  settings.onSave((vals) => {
    localStorage.setItem("archon-settings", JSON.stringify(vals));
    statusModelEl.textContent = vals.model;
  });

  // Initialize UI
  statusModelEl.textContent = settings.current().model;
  sessions.setSessions([
    { id: "default", name: "Session 1", createdAt: Date.now() },
  ]);
  sessions.setActive("default");

  // Connect
  conn.connect();
}

// Bootstrap once DOM is ready
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", () => { main().catch(console.error); });
} else {
  main().catch(console.error);
}
