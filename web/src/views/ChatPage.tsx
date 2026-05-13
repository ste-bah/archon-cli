import { Paperclip, SendHorizontal, X } from "lucide-react";
import { type ChangeEvent, type FormEvent, useRef, useState } from "react";
import { apiClient } from "../api/client";
import type { WebChatAttachment, WebUploadPolicy } from "../api/generated/web";

interface ChatPageProps {
  uploadPolicy?: WebUploadPolicy;
}

type ChatMessage = {
  id: string;
  role: "assistant" | "system" | "user";
  title: string;
  body: string;
  attachments?: WebChatAttachment[];
};

type PendingAttachment = WebChatAttachment & { id: string };

export function ChatPage({ uploadPolicy }: ChatPageProps) {
  const fileInput = useRef<HTMLInputElement | null>(null);
  const [draft, setDraft] = useState("");
  const [pending, setPending] = useState(false);
  const [attachments, setAttachments] = useState<PendingAttachment[]>([]);
  const [messages, setMessages] = useState<ChatMessage[]>([
    {
      id: "system:init",
      role: "system",
      title: "Workbench chat",
      body: "Ready for local web-session messages.",
    },
  ]);
  const canSend = draft.trim().length > 0 || attachments.length > 0;

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
          fileName: file.name,
          sizeBytes: file.size,
          mimeType,
          accepted: result.accepted,
          policyReason: result.decision.policyReason,
          dataBase64: result.accepted ? await fileToBase64(file) : null,
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
    setPending(true);
    const text = draft.trim();
    const outgoing = attachments.map(({ id: _id, ...attachment }) => attachment);
    try {
      const response = await apiClient.submitChat({ message: text, attachments: outgoing });
      if (response.accepted) {
        const displayedAttachments = outgoing.map(({ dataBase64: _dataBase64, ...attachment }) => ({
          ...attachment,
          dataBase64: null,
        }));
        setMessages((current) => [
          ...current,
          {
            id: response.messageId,
            role: "user",
            title: "You",
            body: text || "Attachments",
            attachments: displayedAttachments,
          },
          {
            id: `${response.messageId}:assistant`,
            role: "assistant",
            title: "Archon",
            body: response.reply || response.policyReason,
          },
        ]);
        setDraft("");
        setAttachments([]);
      } else {
        addSystem("Message blocked", response.policyReason);
      }
    } catch (error) {
      addSystem("Send failed", error instanceof Error ? error.message : "Chat submit failed.");
    } finally {
      setPending(false);
    }
  }

  function addSystem(title: string, body: string) {
    setMessages((current) => [
      ...current,
      { id: `system:${crypto.randomUUID()}`, role: "system", title, body },
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
        </div>
        {messages.map((message) => (
          <article key={message.id} className={`message message--${message.role}`}>
            <strong>{message.title}</strong>
            <p>{message.body}</p>
            {message.attachments?.length ? <AttachmentList attachments={message.attachments} /> : null}
          </article>
        ))}
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
        <span key={`${attachment.fileName}:${attachment.sizeBytes}`} className="attachment-chip attachment-chip--static">
          {attachment.fileName}
          <small>{formatBytes(attachment.sizeBytes)}</small>
        </span>
      ))}
    </div>
  );
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
