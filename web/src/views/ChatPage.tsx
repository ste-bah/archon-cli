import { Paperclip, RefreshCw, SendHorizontal, X } from "lucide-react";
import { type ChangeEvent, type FormEvent, useEffect, useRef, useState } from "react";
import { apiClient } from "../api/client";
import type { WebChatAttachment, WebChatHistoryMessage, WebUploadPolicy } from "../api/generated/web";
import "./ChatPage.css";

interface ChatPageProps {
  uploadPolicy?: WebUploadPolicy;
}

type ChatMessage = {
  id: string;
  role: "assistant" | "system" | "user";
  title: string;
  body: string;
  attachments: WebChatAttachment[];
  createdAtMs?: number;
  policyReason?: string;
  storedPath?: string;
  pending?: boolean;
};

type HistoryMeta = {
  storedPath: string;
  truncated: boolean;
  status: string;
};

type PendingAttachment = WebChatAttachment & { id: string; file: File };

const readyMessage: ChatMessage = {
  id: "system:init",
  role: "system",
  title: "Workbench chat",
  body: "Ready for local web-session messages.",
  attachments: [],
};

export function ChatPage({ uploadPolicy }: ChatPageProps) {
  const fileInput = useRef<HTMLInputElement | null>(null);
  const threadEnd = useRef<HTMLDivElement | null>(null);
  const [draft, setDraft] = useState("");
  const [pending, setPending] = useState(false);
  const [turnStatus, setTurnStatus] = useState("Idle");
  const [historyMeta, setHistoryMeta] = useState<HistoryMeta>({
    storedPath: "",
    truncated: false,
    status: "Loading history",
  });
  const [attachments, setAttachments] = useState<PendingAttachment[]>([]);
  const [messages, setMessages] = useState<ChatMessage[]>([readyMessage]);
  const canSend = draft.trim().length > 0 || attachments.length > 0;

  useEffect(() => {
    let active = true;
    void refreshHistory(() => !active);
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    threadEnd.current?.scrollIntoView({ block: "end" });
  }, [messages, pending]);

  async function refreshHistory(isCancelled: () => boolean = () => false) {
    setHistoryMeta((current) => ({ ...current, status: "Loading history" }));
    try {
      const history = await apiClient.chatHistory();
      if (isCancelled()) {
        return;
      }
      setMessages(history.messages.length ? history.messages.map(fromHistoryMessage) : [readyMessage]);
      setHistoryMeta({
        storedPath: history.storedPath,
        truncated: history.truncated,
        status: history.messages.length ? "History restored" : "No saved turns yet",
      });
    } catch (error) {
      if (!isCancelled()) {
        setHistoryMeta((current) => ({ ...current, status: "History unavailable" }));
        addSystem("History unavailable", error instanceof Error ? error.message : "Could not load chat history.");
      }
    }
  }

  async function handleAttach(files: FileList | null) {
    if (!files?.length) {
      return;
    }
    if (!uploadPolicy?.enabled) {
      addSystem("Attachment blocked", uploadPolicy?.policyReason ?? "Upload policy is not loaded.");
      return;
    }
    const remaining = Math.max(uploadPolicy.maxFiles - attachments.length, 0);
    const selected = Array.from(files).slice(0, remaining);
    if (selected.length === 0) {
      addSystem("Attachment blocked", `Maximum attachment count is ${uploadPolicy.maxFiles}.`);
      return;
    }
    for (const file of selected) {
      const mimeType = file.type || "application/octet-stream";
      try {
        const result = await apiClient.uploadIntent({
          fileName: file.name,
          sizeBytes: file.size,
          mimeType,
        });
        const item: PendingAttachment = {
          id: `${file.name}:${file.size}:${crypto.randomUUID()}`,
          file,
          fileName: file.name,
          sizeBytes: file.size,
          mimeType,
          accepted: result.accepted,
          policyReason: result.decision.policyReason,
          dataBase64: null,
          storedPath: null,
        };
        if (result.accepted) {
          setAttachments((current) => [...current, item]);
        } else {
          addSystem("Attachment blocked", result.decision.policyReason);
        }
      } catch (error) {
        addSystem("Attachment failed", error instanceof Error ? error.message : "Upload intent failed.");
      }
    }
    if (fileInput.current) {
      fileInput.current.value = "";
    }
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!canSend || pending) {
      return;
    }
    const localId = crypto.randomUUID();
    const text = draft.trim();
    setPending(true);
    setTurnStatus("Encoding attachments");
    try {
      const outgoing = await Promise.all(
        attachments.map(async ({ id: _id, file, ...attachment }) => ({
          ...attachment,
          dataBase64: attachment.accepted ? await fileToBase64(file) : null,
        })),
      );
      const displayedAttachments = outgoing.map(({ dataBase64: _dataBase64, ...attachment }) => ({
        ...attachment,
        dataBase64: null,
      }));
      const userId = `local:${localId}:user`;
      const assistantId = `local:${localId}:assistant`;
      setMessages((current) => [
        ...withoutReadyMessage(current),
        message(userId, "user", "You", text || "Attachments", displayedAttachments, true),
        message(assistantId, "assistant", "Archon", "Working on it...", [], true),
      ]);
      setDraft("");
      setAttachments([]);
      setTurnStatus("Waiting for live session");
      const response = await apiClient.submitChat({ message: text, attachments: outgoing });
      if (response.accepted) {
        const restoredAttachments = response.attachments.length ? response.attachments : displayedAttachments;
        setMessages((current) => current.map((item) => {
          if (item.id === userId) {
            return message(`${response.messageId}:user`, "user", "You", text || "Attachments", restoredAttachments, false, response);
          }
          if (item.id === assistantId) {
            return message(`${response.messageId}:assistant`, "assistant", "Archon", response.reply.trim() || "Live session completed without assistant text.", [], false, response);
          }
          return item;
        }));
        setHistoryMeta((current) => ({ ...current, storedPath: response.storedPath, status: "Turn saved" }));
      } else {
        replacePending(assistantId, "Message blocked", response.policyReason);
      }
    } catch (error) {
      replacePending(
        `local:${localId}:assistant`,
        "Send failed",
        error instanceof Error ? error.message : "Chat submit failed.",
      );
    } finally {
      setPending(false);
      setTurnStatus("Idle");
    }
  }

  function replacePending(id: string, title: string, body: string) {
    setMessages((current) => {
      let replaced = false;
      const next = current.map((item) => {
        if (item.id !== id) {
          return item;
        }
        replaced = true;
        return { ...item, role: "system" as const, title, body, pending: false };
      });
      return replaced
        ? next
        : [...withoutReadyMessage(next), { id: `system:${crypto.randomUUID()}`, role: "system", title, body, attachments: [] }];
    });
  }

  function addSystem(title: string, body: string) {
    setMessages((current) => [
      ...withoutReadyMessage(current),
      { id: `system:${crypto.randomUUID()}`, role: "system", title, body, attachments: [] },
    ]);
  }

  return (
    <section className="chat-layout">
      <div className="chat-thread" aria-live="polite">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Agent session</span>
            <h3>Conversation</h3>
          </div>
          <button className="icon-action" type="button" aria-label="Refresh chat history" onClick={() => void refreshHistory()}>
            <RefreshCw size={16} />
          </button>
        </div>
        <div className="chat-status" role="status">
          <span>{pending ? turnStatus : historyMeta.status}</span>
          {historyMeta.truncated ? <span>recent turns only</span> : null}
          {historyMeta.storedPath ? <span title={historyMeta.storedPath}>{historyMeta.storedPath}</span> : null}
        </div>
        {messages.map((item) => (
          <article key={item.id} className={`message message--${item.role}${item.pending ? " message--pending" : ""}`}>
            <header className="message__header">
              <strong>{item.title}</strong>
              {item.createdAtMs ? <time>{formatTime(item.createdAtMs)}</time> : null}
            </header>
            <p>{item.body}</p>
            {item.attachments.length ? <AttachmentList attachments={item.attachments} /> : null}
            {item.policyReason ? <small className="message__meta">{item.policyReason}</small> : null}
          </article>
        ))}
        <div ref={threadEnd} />
      </div>
      <form className="composer" onSubmit={handleSubmit}>
        <textarea
          aria-label="Message"
          placeholder="Ask Archon or attach files..."
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
        />
        {attachments.length ? (
          <div className="attachment-list" aria-label="Pending attachments">
            {attachments.map((attachment) => (
              <button
                key={attachment.id}
                className="attachment-chip"
                type="button"
                onClick={() => setAttachments((current) => current.filter((item) => item.id !== attachment.id))}
              >
                <span>{attachment.fileName}</span>
                <small>{formatBytes(attachment.sizeBytes)}</small>
                <X size={14} aria-hidden="true" />
              </button>
            ))}
          </div>
        ) : null}
        <input
          ref={fileInput}
          type="file"
          hidden
          multiple
          accept={uploadPolicy?.acceptedMimeTypes.join(",")}
          onChange={(event: ChangeEvent<HTMLInputElement>) => void handleAttach(event.target.files)}
        />
        <div className="composer__actions">
          <button type="button" onClick={() => fileInput.current?.click()}>
            <Paperclip size={18} />
            Attach
          </button>
          <button type="submit" disabled={!canSend || pending}>
            <SendHorizontal size={18} />
            {pending ? "Sending" : "Send"}
          </button>
        </div>
      </form>
    </section>
  );
}

function AttachmentList({ attachments }: { attachments: WebChatAttachment[] }) {
  return (
    <div className="attachment-list">
      {attachments.map((attachment) => (
        <span key={`${attachment.fileName}:${attachment.sizeBytes}:${attachment.storedPath ?? ""}`} className="attachment-chip attachment-chip--static">
          <span>{attachment.fileName}</span>
          <small>{formatBytes(attachment.sizeBytes)}</small>
          {attachment.storedPath ? <em title={attachment.storedPath}>stored</em> : null}
        </span>
      ))}
    </div>
  );
}

function fromHistoryMessage(item: WebChatHistoryMessage): ChatMessage {
  return {
    id: item.id,
    role: item.role === "assistant" || item.role === "user" ? item.role : "system",
    title: item.title,
    body: item.body,
    attachments: item.attachments,
    createdAtMs: item.createdAtMs,
    policyReason: item.policyReason,
    storedPath: item.storedPath,
  };
}

function message(
  id: string,
  role: ChatMessage["role"],
  title: string,
  body: string,
  attachments: WebChatAttachment[],
  pending = false,
  response?: { createdAtMs: number; policyReason: string; storedPath: string },
): ChatMessage {
  return {
    id,
    role,
    title,
    body,
    attachments,
    pending,
    createdAtMs: response?.createdAtMs,
    policyReason: response?.policyReason,
    storedPath: response?.storedPath,
  };
}

function withoutReadyMessage(messages: ChatMessage[]) {
  return messages.filter((item) => item.id !== readyMessage.id);
}

function formatBytes(value: number) {
  if (value < 1024) {
    return `${value} B`;
  }
  if (value < 1024 * 1024) {
    return `${Math.round(value / 1024)} KB`;
  }
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}

function formatTime(value: number) {
  return new Date(value).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

async function fileToBase64(file: File) {
  const buffer = await file.arrayBuffer();
  const bytes = new Uint8Array(buffer);
  let binary = "";
  const chunkSize = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + chunkSize));
  }
  return btoa(binary);
}
