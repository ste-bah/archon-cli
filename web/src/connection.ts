/// WebSocket client — connects to Archon's /ws/ide endpoint.

export type ConnectionState = "disconnected" | "connecting" | "connected";
export type TextDeltaHandler = (delta: string) => void;
export type TurnCompleteHandler = () => void;

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

export class ArchonConnection {
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
