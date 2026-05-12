// JSON-RPC 2.0 core types.

export interface JRpcRequest<T = unknown> {
  jsonrpc: "2.0";
  id: number;
  method: string;
  params: T;
}

export interface JRpcResponse<T = unknown> {
  jsonrpc: "2.0";
  id: number;
  result?: T;
  error?: JRpcError;
}

export interface JRpcNotification<T = unknown> {
  jsonrpc: "2.0";
  method: string;
  params: T;
}

export interface JRpcError {
  code: number;
  message: string;
  data?: unknown;
}

export const JRpcErrorCode = {
  PARSE_ERROR: -32700,
  INVALID_REQUEST: -32600,
  METHOD_NOT_FOUND: -32601,
  INVALID_PARAMS: -32602,
  INTERNAL_ERROR: -32603,
} as const;

export interface IdeCapabilities {
  inlineCompletion: boolean;
  toolExecution: boolean;
  diff: boolean;
  terminal: boolean;
}

export interface IdeClientInfo {
  name: string;
  version: string;
}

export interface IdeInitializeParams {
  clientInfo: IdeClientInfo;
  capabilities: IdeCapabilities;
}

export interface IdeInitializeResult {
  sessionId: string;
  serverVersion: string;
  capabilities: IdeCapabilities;
}

export interface IdePromptParams {
  sessionId: string;
  text: string;
  contextFiles?: string[];
}

export interface IdeCancelParams {
  sessionId: string;
}

export interface IdeToolResultParams {
  sessionId: string;
  toolUseId: string;
  result: string;
  isError: boolean;
}

export interface IdeStatusParams {
  sessionId: string;
}

export interface IdeStatusResult {
  model: string;
  inputTokens: number;
  outputTokens: number;
  cost: number;
}

export interface IdeConfigParams {
  key?: string;
  value?: unknown;
}

export interface IdeConfigResult {
  value?: unknown;
  ok?: boolean;
}

export interface IdeTextDelta {
  sessionId: string;
  text: string;
}

export interface IdeThinkingDelta {
  sessionId: string;
  thinking: string;
}

export interface IdeToolCall {
  sessionId: string;
  toolUseId: string;
  name: string;
  input: unknown;
}

export interface IdePermissionRequest {
  sessionId: string;
  action: string;
  description: string;
}

export interface IdeTurnComplete {
  sessionId: string;
  inputTokens: number;
  outputTokens: number;
  cost: number;
}

export interface IdeErrorNotification {
  sessionId?: string;
  message: string;
  code: number;
}

export interface PendingRequest {
  resolve: (value: JRpcResponse) => void;
  reject: (reason: Error) => void;
}
