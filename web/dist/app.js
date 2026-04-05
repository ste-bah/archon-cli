// Archon Web UI — bundled 2026-04-05T15:42:07Z
;
/// WebSocket client — connects to Archon's /ws/ide endpoint.


interface JRpcRequest {
  jsonrpc: "2.0";
  id: number;
  method: string;
  params?: unknown;
}

interface JRpcResponse {
  jsonrpc: "2.0";
  id: number;
  result?: unknown;
  error?: { code: number; message: string };
}

interface JRpcNotification {
  jsonrpc: "2.0";
  method: string;
  params?: unknown;
}

  private ws: WebSocket | null = null;
  private nextId = 1;
  private pendingCallbacks = new Map<number, (res: JRpcResponse) => void>();
  private onTextDeltaHandler: TextDeltaHandler | null = null;
  private onTurnCompleteHandler: TurnCompleteHandler | null = null;
  private onStateChange: ((state: ConnectionState) => void) | null = null;
  private sessionId: string | null = null;

  constructor(
    private readonly url: string,
    private readonly token: string | null,
  ) {}

  onState(handler: (state: ConnectionState) => void): void {
    this.onStateChange = handler;
  }

  onTextDelta(handler: TextDeltaHandler): void {
    this.onTextDeltaHandler = handler;
  }

  onTurnComplete(handler: TurnCompleteHandler): void {
    this.onTurnCompleteHandler = handler;
  }

  connect(): void {
    if (this.ws) return;
    this.onStateChange?.("connecting");

    const wsUrl = this.token
      ? `${this.url}?token=${encodeURIComponent(this.token)}`
      : this.url;

    this.ws = new WebSocket(wsUrl);

    this.ws.addEventListener("open", () => {
      this.onStateChange?.("connected");
      this.initialize();
    });

    this.ws.addEventListener("message", (ev) => {
      this.handleMessage(ev.data as string);
    });

    this.ws.addEventListener("close", () => {
      this.ws = null;
      this.onStateChange?.("disconnected");
    });

    this.ws.addEventListener("error", () => {
      this.ws = null;
      this.onStateChange?.("disconnected");
    });
  }

  disconnect(): void {
    this.ws?.close();
    this.ws = null;
  }

  async sendPrompt(text: string): Promise<void> {
    return new Promise((resolve, reject) => {
      const id = this.nextId++;
      const req: JRpcRequest = {
        jsonrpc: "2.0",
        id,
        method: "archon/prompt",
        params: { sessionId: this.sessionId, text },
      };
      this.pendingCallbacks.set(id, (res) => {
        if (res.error) reject(new Error(res.error.message));
        else resolve();
      });
      this.send(req);
    });
  }

  private async initialize(): Promise<void> {
    const id = this.nextId++;
    const req: JRpcRequest = {
      jsonrpc: "2.0",
      id,
      method: "archon/initialize",
      params: { clientVersion: "0.1.0" },
    };
    return new Promise((resolve) => {
      this.pendingCallbacks.set(id, (res) => {
        if (res.result && typeof res.result === "object") {
          const r = res.result as Record<string, unknown>;
          this.sessionId = (r.sessionId as string) ?? null;
        }
        resolve();
      });
      this.send(req);
    });
  }

  private handleMessage(data: string): void {
    let msg: JRpcResponse | JRpcNotification;
    try {
      msg = JSON.parse(data) as JRpcResponse | JRpcNotification;
    } catch {
      return;
    }

    if ("id" in msg && msg.id !== undefined) {
      // Response to a request
      const cb = this.pendingCallbacks.get((msg as JRpcResponse).id);
      if (cb) {
        this.pendingCallbacks.delete((msg as JRpcResponse).id);
        cb(msg as JRpcResponse);
      }
      return;
    }

    // Notification
    const notif = msg as JRpcNotification;
    if (notif.method === "archon/textDelta") {
      const p = notif.params as Record<string, unknown>;
      this.onTextDeltaHandler?.(p.text as string);
    } else if (notif.method === "archon/turnComplete") {
      this.onTurnCompleteHandler?.();
    }
  }

  private send(msg: JRpcRequest): void {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(msg));
    }
  }
}
;
/// Chat message display and streaming rendering.


interface ThinkingChunk {
  type: "thinking";
  text: string;
}

interface TextChunk {
  type: "text";
  text: string;
}

interface ToolChunk {
  type: "tool";
  name: string;
  input: string;
  result?: string;
}

type ContentChunk = ThinkingChunk | TextChunk | ToolChunk;

interface Message {
  role: MessageRole;
  chunks: ContentChunk[];
  el: HTMLElement;
}

  private messages: Message[] = [];
  private currentAssistant: Message | null = null;
  private currentTextEl: HTMLElement | null = null;

  constructor(private readonly container: HTMLElement) {}

  addUserMessage(text: string): void {
    const msg = this.createMessage("user");
    this.appendText(msg, text);
    this.messages.push(msg);
    this.scrollBottom();
  }

  startAssistantMessage(): void {
    const msg = this.createMessage("assistant");
    this.currentAssistant = msg;
    this.currentTextEl = null;
    this.messages.push(msg);
  }

  appendTextDelta(delta: string): void {
    if (!this.currentAssistant) this.startAssistantMessage();
    const msg = this.currentAssistant!;

    // Append to or create a text chunk
    const last = msg.chunks[msg.chunks.length - 1];
    if (last?.type === "text") {
      last.text += delta;
      if (this.currentTextEl) {
        this.currentTextEl.textContent = last.text;
      }
    } else {
      const chunk: TextChunk = { type: "text", text: delta };
      msg.chunks.push(chunk);
      const el = document.createElement("div");
      el.className = "message-text";
      el.textContent = delta;
      msg.el.querySelector(".message-body")!.appendChild(el);
      this.currentTextEl = el;
    }

    this.scrollBottom();
  }

  appendThinking(text: string): void {
    if (!this.currentAssistant) this.startAssistantMessage();
    const msg = this.currentAssistant!;
    const chunk: ThinkingChunk = { type: "thinking", text };
    msg.chunks.push(chunk);

    const details = document.createElement("details");
    details.className = "thinking-block";
    const summary = document.createElement("summary");
    summary.textContent = "Thinking…";
    const content = document.createElement("div");
    content.className = "thinking-content";
    content.textContent = text;
    details.appendChild(summary);
    details.appendChild(content);
    msg.el.querySelector(".message-body")!.appendChild(details);
    this.currentTextEl = null;
    this.scrollBottom();
  }

  appendToolCall(name: string, input: string, result?: string): void {
    if (!this.currentAssistant) this.startAssistantMessage();
    const msg = this.currentAssistant!;
    const chunk: ToolChunk = { type: "tool", name, input, result };
    msg.chunks.push(chunk);

    const details = document.createElement("details");
    details.className = "tool-block";
    const summary = document.createElement("summary");
    summary.textContent = `Tool: ${name}`;
    const pre = document.createElement("pre");
    pre.textContent = input + (result ? `\n\n→ ${result}` : "");
    details.appendChild(summary);
    details.appendChild(pre);
    msg.el.querySelector(".message-body")!.appendChild(details);
    this.currentTextEl = null;
    this.scrollBottom();
  }

  finishAssistantMessage(): void {
    this.currentAssistant = null;
    this.currentTextEl = null;
  }

  clear(): void {
    this.messages = [];
    this.currentAssistant = null;
    this.currentTextEl = null;
    this.container.innerHTML = "";
  }

  private createMessage(role: MessageRole): Message {
    const el = document.createElement("div");
    el.className = `message ${role}`;

    const label = document.createElement("div");
    label.className = "role-label";
    label.textContent = role === "user" ? "You" : "Archon";
    el.appendChild(label);

    const body = document.createElement("div");
    body.className = "message-body";
    el.appendChild(body);

    this.container.appendChild(el);

    return { role, chunks: [], el };
  }

  private appendText(msg: Message, text: string): void {
    const chunk: TextChunk = { type: "text", text };
    msg.chunks.push(chunk);

    const div = document.createElement("div");
    div.className = "message-text";
    div.textContent = text;
    msg.el.querySelector(".message-body")!.appendChild(div);
  }

  private scrollBottom(): void {
    this.container.scrollTop = this.container.scrollHeight;
  }
}
;
/// Input area — handles text entry, Ctrl+Enter submit, and file upload.


  private readonly textarea: HTMLTextAreaElement;
  private readonly fileInput: HTMLInputElement;
  private readonly sendBtn: HTMLButtonElement;
  private onSubmitHandler: SubmitHandler | null = null;
  private pendingFiles: File[] = [];

  constructor(form: HTMLFormElement) {
    this.textarea = form.querySelector("#chat-input")!;
    this.fileInput = form.querySelector("#file-upload")!;
    this.sendBtn = form.querySelector("#send-btn")!;

    this.textarea.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" && ev.ctrlKey) {
        ev.preventDefault();
        this.submit();
      }
    });

    form.addEventListener("submit", (ev) => {
      ev.preventDefault();
      this.submit();
    });

    this.fileInput.addEventListener("change", () => {
      if (this.fileInput.files) {
        this.pendingFiles.push(...Array.from(this.fileInput.files));
      }
      this.fileInput.value = "";
    });
  }

  onSubmit(handler: SubmitHandler): void {
    this.onSubmitHandler = handler;
  }

  setEnabled(enabled: boolean): void {
    this.textarea.disabled = !enabled;
    this.sendBtn.disabled = !enabled;
  }

  clear(): void {
    this.textarea.value = "";
    this.pendingFiles = [];
  }

  focus(): void {
    this.textarea.focus();
  }

  private submit(): void {
    const text = this.textarea.value.trim();
    if (!text && this.pendingFiles.length === 0) return;
    const files = [...this.pendingFiles];
    this.clear();
    this.onSubmitHandler?.(text, files);
  }
}
;
/// Session list sidebar — create, resume, and switch sessions.

  id: string;
  name: string;
  createdAt: number;
}


  private sessions: SessionInfo[] = [];
  private activeId: string | null = null;
  private onChangeHandler: SessionChangeHandler | null = null;

  constructor(
    private readonly listEl: HTMLUListElement,
    private readonly newBtn: HTMLButtonElement,
  ) {
    this.newBtn.addEventListener("click", () => {
      this.createSession();
    });
  }

  onChange(handler: SessionChangeHandler): void {
    this.onChangeHandler = handler;
  }

  setSessions(sessions: SessionInfo[]): void {
    this.sessions = sessions;
    this.render();
  }

  addSession(session: SessionInfo): void {
    this.sessions.unshift(session);
    this.render();
  }

  setActive(id: string): void {
    this.activeId = id;
    this.render();
  }

  exportConversation(): string {
    return JSON.stringify(
      { sessionId: this.activeId, exportedAt: Date.now() },
      null,
      2,
    );
  }

  private createSession(): void {
    const id = crypto.randomUUID();
    const session: SessionInfo = {
      id,
      name: `Session ${this.sessions.length + 1}`,
      createdAt: Date.now(),
    };
    this.addSession(session);
    this.activateSession(id);
  }

  private activateSession(id: string): void {
    this.activeId = id;
    this.render();
    this.onChangeHandler?.(id);
  }

  private render(): void {
    this.listEl.innerHTML = "";
    for (const session of this.sessions) {
      const li = document.createElement("li");
      li.textContent = session.name;
      li.title = session.id;
      if (session.id === this.activeId) li.classList.add("active");
      li.addEventListener("click", () => this.activateSession(session.id));
      this.listEl.appendChild(li);
    }
  }
}
;
/// Settings panel — model, provider, effort controls.

  model: string;
  provider: string;
  effort: string;
}


  private readonly panel: HTMLElement;
  private readonly modelInput: HTMLInputElement;
  private readonly providerSelect: HTMLSelectElement;
  private readonly effortSelect: HTMLSelectElement;
  private onSaveHandler: SettingsSaveHandler | null = null;

  constructor(
    panel: HTMLElement,
    openBtn: HTMLButtonElement,
  ) {
    this.panel = panel;
    this.modelInput = panel.querySelector("#settings-model")!;
    this.providerSelect = panel.querySelector("#settings-provider")!;
    this.effortSelect = panel.querySelector("#settings-effort")!;

    openBtn.addEventListener("click", () => this.show());

    panel.querySelector("#settings-close-btn")!
      .addEventListener("click", () => this.hide());

    panel.querySelector("#settings-save-btn")!
      .addEventListener("click", () => {
        this.onSaveHandler?.(this.current());
        this.hide();
      });
  }

  onSave(handler: SettingsSaveHandler): void {
    this.onSaveHandler = handler;
  }

  current(): SettingsValues {
    return {
      model: this.modelInput.value.trim(),
      provider: this.providerSelect.value,
      effort: this.effortSelect.value,
    };
  }

  load(values: Partial): void {
    if (values.model) this.modelInput.value = values.model;
    if (values.provider) this.providerSelect.value = values.provider;
    if (values.effort) this.effortSelect.value = values.effort;
  }

  private show(): void {
    this.panel.hidden = false;
  }

  private hide(): void {
    this.panel.hidden = true;
  }
}
;
/// Archon Web UI — application entry point.
///
/// Wires together all UI components and the WebSocket connection.


function qs<T extends HTMLElement>(sel: string): T {
  const el = document.querySelector(sel);
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
  const messagesEl = qs("#messages");
  const statusConnEl = qs("#status-conn");
  const statusModelEl = qs("#status-model");
  const sessionListEl = qs("#session-list");
  const newSessionBtn = qs("#new-session-btn");
  const chatForm = qs("#chat-form");
  const settingsPanelEl = qs("#settings-panel");
  const settingsOpenBtn = qs("#settings-open-btn");

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
;
