import type {
  IdeCapabilities,
  IdeClientInfo,
  IdeInitializeResult,
  IdeTextDelta,
  IdeTurnComplete,
  JRpcNotification,
  JRpcRequest,
  JRpcResponse,
  PendingRequest,
} from "./protocol";

interface StdioProcess {
  stdin: {
    setEncoding(encoding: string): void;
    on(event: "data", handler: (chunk: string) => void): void;
  };
  stdout: {
    write(data: string): void;
  };
}

declare const process: StdioProcess | undefined;

/**
 * Stdio JSON-lines transport for IDE extensions that spawn an Archon process.
 */
export class ArchonStdioClient {
  private pending: Map<number, PendingRequest> = new Map();
  private nextId = 1;
  private buffer = "";

  private textDeltaHandlers: Array<(sessionId: string, text: string) => void> = [];
  private turnCompleteHandlers: Array<
    (sessionId: string, inputTokens: number, outputTokens: number, cost: number) => void
  > = [];

  onTextDelta(handler: (sessionId: string, text: string) => void): void {
    this.textDeltaHandlers.push(handler);
  }

  onTurnComplete(
    handler: (sessionId: string, inputTokens: number, outputTokens: number, cost: number) => void,
  ): void {
    this.turnCompleteHandlers.push(handler);
  }

  start(): void {
    if (typeof process === "undefined") return;

    process.stdin.setEncoding("utf8");
    process.stdin.on("data", (chunk: string) => {
      this.buffer += chunk;
      const lines = this.buffer.split("\n");
      this.buffer = lines.pop() ?? "";
      for (const line of lines) {
        const trimmed = line.trim();
        if (trimmed.length > 0) this.handleLine(trimmed);
      }
    });
  }

  initialize(
    clientInfo: IdeClientInfo,
    capabilities: IdeCapabilities,
  ): Promise<IdeInitializeResult> {
    return this.request<IdeInitializeResult>("archon/initialize", {
      clientInfo,
      capabilities,
    });
  }

  sendPrompt(sessionId: string, text: string): Promise<void> {
    return this.request<{ queued: boolean }>("archon/prompt", {
      sessionId,
      text,
    }).then(() => undefined);
  }

  private request<T>(method: string, params: unknown): Promise<T> {
    return new Promise((resolve, reject) => {
      if (typeof process === "undefined") {
        reject(new Error("stdio transport requires process stdio"));
        return;
      }
      const id = this.nextId++;
      const req: JRpcRequest = { jsonrpc: "2.0", id, method, params };
      this.pending.set(id, {
        resolve: (resp: JRpcResponse) => {
          if (resp.error) {
            reject(new Error(`JSON-RPC error ${resp.error.code}: ${resp.error.message}`));
          } else {
            resolve(resp.result as T);
          }
        },
        reject,
      });
      process.stdout.write(JSON.stringify(req) + "\n");
    });
  }

  private handleLine(line: string): void {
    let msg: unknown;
    try {
      msg = JSON.parse(line) as unknown;
    } catch {
      return;
    }
    if (typeof msg !== "object" || msg === null) return;
    const obj = msg as Record<string, unknown>;

    if ("id" in obj && typeof obj["id"] === "number") {
      this.resolveResponse(msg as JRpcResponse);
      return;
    }

    if ("method" in obj && typeof obj["method"] === "string") {
      const notif = msg as JRpcNotification<Record<string, unknown>>;
      this.dispatchNotification(notif);
    }
  }

  private resolveResponse(resp: JRpcResponse): void {
    const pending = this.pending.get(resp.id);
    if (pending) {
      this.pending.delete(resp.id);
      pending.resolve(resp);
    }
  }

  private dispatchNotification(notif: JRpcNotification<Record<string, unknown>>): void {
    switch (notif.method) {
      case "archon/textDelta": {
        const p = notif.params as unknown as IdeTextDelta;
        for (const h of this.textDeltaHandlers) h(p.sessionId, p.text);
        break;
      }
      case "archon/turnComplete": {
        const p = notif.params as unknown as IdeTurnComplete;
        for (const h of this.turnCompleteHandlers) {
          h(p.sessionId, p.inputTokens, p.outputTokens, p.cost);
        }
        break;
      }
      default:
        break;
    }
  }
}
