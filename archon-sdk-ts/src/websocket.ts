import type {
  IdeCancelParams,
  IdeCapabilities,
  IdeClientInfo,
  IdeConfigParams,
  IdeConfigResult,
  IdeErrorNotification,
  IdeInitializeParams,
  IdeInitializeResult,
  IdePermissionRequest,
  IdePromptParams,
  IdeStatusParams,
  IdeStatusResult,
  IdeTextDelta,
  IdeThinkingDelta,
  IdeToolCall,
  IdeToolResultParams,
  IdeTurnComplete,
  JRpcNotification,
  JRpcRequest,
  JRpcResponse,
  PendingRequest,
} from "./protocol";

/**
 * WebSocket-based Archon client for IDE extensions.
 */
export class ArchonClient {
  private ws: WebSocket | null = null;
  private pending: Map<number, PendingRequest> = new Map();
  private nextId = 1;

  private textDeltaHandlers: Array<(sessionId: string, text: string) => void> = [];
  private thinkingDeltaHandlers: Array<(sessionId: string, thinking: string) => void> = [];
  private toolCallHandlers: Array<
    (sessionId: string, toolUseId: string, name: string, input: unknown) => void
  > = [];
  private permissionRequestHandlers: Array<
    (sessionId: string, action: string, description: string) => void
  > = [];
  private turnCompleteHandlers: Array<
    (sessionId: string, inputTokens: number, outputTokens: number, cost: number) => void
  > = [];
  private errorHandlers: Array<
    (sessionId: string | undefined, message: string, code: number) => void
  > = [];

  connect(url: string): Promise<void> {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(url);
      this.ws = ws;

      ws.onopen = () => resolve();
      ws.onerror = (event) => reject(new Error(`WebSocket error: ${String(event)}`));
      ws.onclose = () => {
        for (const [, pending] of this.pending) {
          pending.reject(new Error("WebSocket closed"));
        }
        this.pending.clear();
      };
      ws.onmessage = (event: MessageEvent<string>) => {
        this.handleMessage(event.data);
      };
    });
  }

  async initialize(
    clientInfo: IdeClientInfo,
    capabilities: IdeCapabilities,
  ): Promise<IdeInitializeResult> {
    const params: IdeInitializeParams = { clientInfo, capabilities };
    return this.request<IdeInitializeResult>("archon/initialize", params);
  }

  async sendPrompt(
    sessionId: string,
    text: string,
    contextFiles?: string[],
  ): Promise<void> {
    const params: IdePromptParams = { sessionId, text, contextFiles };
    await this.request<{ queued: boolean }>("archon/prompt", params);
  }

  async cancel(sessionId: string): Promise<boolean> {
    const params: IdeCancelParams = { sessionId };
    const resp = await this.request<{ cancelled: boolean }>("archon/cancel", params);
    return resp.cancelled;
  }

  async toolResult(
    sessionId: string,
    toolUseId: string,
    result: string,
    isError = false,
  ): Promise<void> {
    const params: IdeToolResultParams = { sessionId, toolUseId, result, isError };
    await this.request<{ ok: boolean }>("archon/toolResult", params);
  }

  status(sessionId: string): Promise<IdeStatusResult> {
    const params: IdeStatusParams = { sessionId };
    return this.request<IdeStatusResult>("archon/status", params);
  }

  config(key?: string, value?: unknown): Promise<IdeConfigResult> {
    const params: IdeConfigParams = { key, value };
    return this.request<IdeConfigResult>("archon/config", params);
  }

  onTextDelta(handler: (sessionId: string, text: string) => void): void {
    this.textDeltaHandlers.push(handler);
  }

  onThinkingDelta(handler: (sessionId: string, thinking: string) => void): void {
    this.thinkingDeltaHandlers.push(handler);
  }

  onToolCall(
    handler: (sessionId: string, toolUseId: string, name: string, input: unknown) => void,
  ): void {
    this.toolCallHandlers.push(handler);
  }

  onPermissionRequest(
    handler: (sessionId: string, action: string, description: string) => void,
  ): void {
    this.permissionRequestHandlers.push(handler);
  }

  onTurnComplete(
    handler: (sessionId: string, inputTokens: number, outputTokens: number, cost: number) => void,
  ): void {
    this.turnCompleteHandlers.push(handler);
  }

  onError(
    handler: (sessionId: string | undefined, message: string, code: number) => void,
  ): void {
    this.errorHandlers.push(handler);
  }

  disconnect(): void {
    this.ws?.close();
    this.ws = null;
  }

  private request<T>(method: string, params: unknown): Promise<T> {
    return new Promise((resolve, reject) => {
      const ws = this.ws;
      if (!ws || ws.readyState !== WebSocket.OPEN) {
        reject(new Error("WebSocket is not connected"));
        return;
      }

      const id = this.nextId++;
      const request: JRpcRequest = { jsonrpc: "2.0", id, method, params };
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

      ws.send(JSON.stringify(request));
    });
  }

  private handleMessage(data: string): void {
    let msg: unknown;
    try {
      msg = JSON.parse(data) as unknown;
    } catch {
      return;
    }

    if (typeof msg !== "object" || msg === null) return;
    const obj = msg as Record<string, unknown>;

    if ("id" in obj && typeof obj["id"] === "number") {
      const resp = msg as JRpcResponse;
      const pending = this.pending.get(resp.id);
      if (pending) {
        this.pending.delete(resp.id);
        pending.resolve(resp);
      }
      return;
    }

    if ("method" in obj && typeof obj["method"] === "string") {
      const notif = msg as JRpcNotification<Record<string, unknown>>;
      this.dispatchNotification(notif.method, notif.params ?? {});
    }
  }

  private dispatchNotification(method: string, params: Record<string, unknown>): void {
    switch (method) {
      case "archon/textDelta":
        this.dispatchTextDelta(params);
        break;
      case "archon/thinkingDelta":
        this.dispatchThinkingDelta(params);
        break;
      case "archon/toolCall":
        this.dispatchToolCall(params);
        break;
      case "archon/permissionRequest":
        this.dispatchPermissionRequest(params);
        break;
      case "archon/turnComplete":
        this.dispatchTurnComplete(params);
        break;
      case "archon/error":
        this.dispatchError(params);
        break;
      default:
        break;
    }
  }

  private dispatchTextDelta(params: Record<string, unknown>): void {
    const p = params as unknown as IdeTextDelta;
    for (const h of this.textDeltaHandlers) h(p.sessionId, p.text);
  }

  private dispatchThinkingDelta(params: Record<string, unknown>): void {
    const p = params as unknown as IdeThinkingDelta;
    for (const h of this.thinkingDeltaHandlers) h(p.sessionId, p.thinking);
  }

  private dispatchToolCall(params: Record<string, unknown>): void {
    const p = params as unknown as IdeToolCall;
    for (const h of this.toolCallHandlers) h(p.sessionId, p.toolUseId, p.name, p.input);
  }

  private dispatchPermissionRequest(params: Record<string, unknown>): void {
    const p = params as unknown as IdePermissionRequest;
    for (const h of this.permissionRequestHandlers) h(p.sessionId, p.action, p.description);
  }

  private dispatchTurnComplete(params: Record<string, unknown>): void {
    const p = params as unknown as IdeTurnComplete;
    for (const h of this.turnCompleteHandlers) {
      h(p.sessionId, p.inputTokens, p.outputTokens, p.cost);
    }
  }

  private dispatchError(params: Record<string, unknown>): void {
    const p = params as unknown as IdeErrorNotification;
    for (const h of this.errorHandlers) h(p.sessionId, p.message, p.code);
  }
}
