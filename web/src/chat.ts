/// Chat message display and streaming rendering.

export type MessageRole = "user" | "assistant";

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

export class ChatView {
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
