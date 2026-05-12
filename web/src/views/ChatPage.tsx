import { Paperclip, SendHorizontal } from "lucide-react";

export function ChatPage() {
  return (
    <section className="chat-layout">
      <div className="chat-thread" aria-live="polite">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Agent session</span>
            <h3>Conversation</h3>
          </div>
        </div>
        <article className="message message--system">
          <strong>Workbench chat foundation</strong>
          <p>
            The chat tab is now part of the React shell. Live agent streaming,
            uploads, and action previews land behind the web action envelope.
          </p>
        </article>
      </div>
      <form className="composer">
        <textarea aria-label="Message" placeholder="Ask Archon or attach files..." />
        <div className="composer__actions">
          <button type="button" disabled title="Uploads land behind policy gates">
            <Paperclip size={18} />
            Attach
          </button>
          <button type="submit" disabled>
            <SendHorizontal size={18} />
            Send
          </button>
        </div>
      </form>
    </section>
  );
}
