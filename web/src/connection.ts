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

type PendingRequest =
  | { type: "initialize"; resolve: () => void }
  | { type: "prompt"; resolve: () => void; reject: (error: Error) => void };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isJRpcError(value: unknown): value is JRpcResponse["error"] {
  return (
    isRecord(value) &&
    typeof value.code === "number" &&
    typeof value.message === "string"
  );
}

function isJRpcResponse(value: unknown): value is JRpcResponse {
  return (
    isRecord(value) &&
    value.jsonrpc === "2.0" &&
    Number.isSafeInteger(value.id) &&
    (value.error === undefined || isJRpcError(value.error))
  );
}

function isJRpcNotification(value: unknown): value is JRpcNotification {
  return (
    isRecord(value) &&
    value.jsonrpc === "2.0" &&
    typeof value.method === "string" &&
    !("id" in value)
  );
}

export class ArchonConnection {
  private ws: WebSocket | null = null;
  private nextId = 1;
  private pendingRequests = new Map<number, PendingRequest>();
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
      this.pendingRequests.set(id, { type: "prompt", resolve, reject });
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
      this.pendingRequests.set(id, { type: "initialize", resolve });
      this.send(req);
    });
  }

  private handleMessage(data: string): void {
    let msg: unknown;
    try {
      msg = JSON.parse(data) as unknown;
    } catch {
      return;
    }

    if (isJRpcResponse(msg)) {
      this.handleResponse(msg);
      return;
    }

    if (isJRpcNotification(msg)) {
      this.handleNotification(msg);
    }
  }

  private handleResponse(response: JRpcResponse): void {
    const pending = this.pendingRequests.get(response.id);
    if (!pending) {
      return;
    }
    this.pendingRequests.delete(response.id);

    switch (pending.type) {
      case "initialize":
        this.handleInitializeResponse(response, pending);
        break;
      case "prompt":
        this.handlePromptResponse(response, pending);
        break;
    }
  }

  private handleInitializeResponse(
    response: JRpcResponse,
    pending: Extract<PendingRequest, { type: "initialize" }>,
  ): void {
    if (isRecord(response.result)) {
      this.sessionId =
        typeof response.result.sessionId === "string"
          ? response.result.sessionId
          : null;
    }
    pending.resolve();
  }

  private handlePromptResponse(
    response: JRpcResponse,
    pending: Extract<PendingRequest, { type: "prompt" }>,
  ): void {
    if (response.error) {
      pending.reject(new Error(response.error.message));
    } else {
      pending.resolve();
    }
  }

  private handleNotification(notif: JRpcNotification): void {
    switch (notif.method) {
      case "archon/textDelta": {
        const p = isRecord(notif.params) ? notif.params : null;
        if (typeof p?.text === "string") {
          this.onTextDeltaHandler?.(p.text);
        }
        break;
      }
      case "archon/turnComplete":
        this.onTurnCompleteHandler?.();
        break;
    }
  }

  private send(msg: JRpcRequest): void {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(msg));
    }
  }
}
