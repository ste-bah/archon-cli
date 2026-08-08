import { useEffect, useRef, useState } from "react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import "./TerminalPage.css";
import { uploadDroppedFiles } from "./terminalUploads";

type ConnectionState = "connecting" | "open" | "closed" | "refused";

/**
 * The real TUI, in the browser.
 *
 * The server spawns `archon` under a pseudo-terminal and relays its bytes over
 * a WebSocket; this component is the other end of that pipe. It is an emulator,
 * not a renderer — nothing here knows what a ratatui widget is, which is why
 * the pane looks exactly like the terminal app and needs no upkeep as the TUI
 * changes.
 *
 * This is a *new* session. It does not attach to the one you may already be
 * sitting in: archon does not own the terminal it was launched from, so there
 * is no stream to join.
 */
export function TerminalPage() {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const socketRef = useRef<WebSocket | null>(null);
  const [state, setState] = useState<ConnectionState>("connecting");
  const [detail, setDetail] = useState("");
  const [dragging, setDragging] = useState(false);
  const [uploadNote, setUploadNote] = useState("");

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const term = new Terminal({
      convertEol: false,
      cursorBlink: true,
      fontFamily: '"Cascadia Mono", "JetBrains Mono", Menlo, Consolas, monospace',
      fontSize: 13,
      // The TUI paints its own background; letting xterm default to black would
      // put a hard rectangle inside a themed page.
      theme: terminalTheme(),
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    fit.fit();

    const socket = new WebSocket(
      socketUrl(term.cols, term.rows),
      // Sub-protocol rather than a header: the browser WebSocket constructor
      // cannot set Authorization, and a query-string token would land in
      // access logs. Empty array when no auth is configured.
      bearerProtocols(),
    );
    socket.binaryType = "arraybuffer";
    socketRef.current = socket;

    const encoder = new TextEncoder();

    socket.onopen = () => {
      setState("open");
      // Re-announce the fitted size: the query string carried the size at
      // construction, but a slow handshake can outlive a layout change.
      sendResize(socket, term.cols, term.rows);
    };
    socket.onmessage = (event) => {
      if (event.data instanceof ArrayBuffer) {
        term.write(new Uint8Array(event.data));
      } else if (typeof event.data === "string") {
        term.write(event.data);
      }
    };
    socket.onerror = () => {
      // `onerror` fires without a reason by design (the spec withholds it to
      // avoid leaking cross-origin information), so the useful message comes
      // from `onclose` right after. Only note that something went wrong.
      setDetail((current) => current || "connection failed");
    };
    socket.onclose = (event) => {
      // 1006 with no prior open is what a refused upgrade looks like from
      // script: the route is absent because policy or the bind address says so.
      const refused = event.code === 1006 && socket.readyState === WebSocket.CLOSED;
      setState(refused ? "refused" : "closed");
      setDetail(closeDetail(event));
      term.write("\r\n\x1b[2m[session ended]\x1b[0m\r\n");
    };

    const input = term.onData((data) => {
      if (socket.readyState === WebSocket.OPEN) {
        socket.send(encoder.encode(data));
      }
    });
    const binary = term.onBinary((data) => {
      if (socket.readyState === WebSocket.OPEN) {
        socket.send(Uint8Array.from(data, (char) => char.charCodeAt(0) & 0xff));
      }
    });

    // A full-screen ratatui app does not reflow on its own: it draws to the
    // size it was told about. Without propagating the new size the pane shows
    // torn frames rather than a smaller layout.
    const observer = new ResizeObserver(() => {
      fit.fit();
      sendResize(socket, term.cols, term.rows);
    });
    observer.observe(host);

    return () => {
      observer.disconnect();
      input.dispose();
      binary.dispose();
      // Closing the socket is what kills the child: the server drops the PTY
      // session when the connection ends, so an unmount must not leave one.
      socket.close();
      term.dispose();
      socketRef.current = null;
    };
  }, []);

  // Dropping a file types `@<path>` at the prompt rather than sending bytes:
  // see terminalUploads.ts for why the round trip through the server is the
  // whole mechanism.
  const onDrop = async (event: React.DragEvent) => {
    event.preventDefault();
    setDragging(false);
    const files = Array.from(event.dataTransfer.files);
    if (files.length === 0) return;

    setUploadNote(`uploading ${describe(files)}…`);
    const { injection, error } = await uploadDroppedFiles(files);
    if (error) {
      setUploadNote(error);
      return;
    }
    const socket = socketRef.current;
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      setUploadNote("uploaded, but the terminal is not connected");
      return;
    }
    socket.send(new TextEncoder().encode(injection));
    setUploadNote(`attached ${describe(files)}`);
  };

  return (
    <section className="terminal-page">
      <header className="terminal-page__header">
        <div>
          <span className="eyebrow">Terminal</span>
          <h2>archon session</h2>
        </div>
        <div className="terminal-page__status" data-state={state}>
          {label(state)}
          {detail ? <small>{detail}</small> : null}
        </div>
      </header>
      {state === "refused" ? (
        <p className="terminal-page__note">
          The terminal route is not served by this process. It requires
          <code> policy.web.allow_web_terminal = true</code> and a loopback bind;
          a non-loopback bind removes it unconditionally.
        </p>
      ) : (
        <p className="terminal-page__note">
          A new <code>archon</code> process, not the session you launched the
          workbench from. Closing this tab ends it. Drop a file anywhere on the
          pane to attach it.
          {uploadNote ? <> — {uploadNote}</> : null}
        </p>
      )}
      <div
        className={
          dragging
            ? "terminal-page__surface terminal-page__surface--dragging"
            : "terminal-page__surface"
        }
        ref={hostRef}
        onDragOver={(event) => {
          // Without preventDefault the browser navigates to the dropped file,
          // discarding the session.
          event.preventDefault();
          setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={onDrop}
      />
    </section>
  );
}

function describe(files: File[]): string {
  const only = files.length === 1 ? files[0] : undefined;
  return only ? only.name : `${files.length} files`;
}

function socketUrl(cols: number, rows: number): string {
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  const params = new URLSearchParams({ cols: String(cols), rows: String(rows) });
  return `${protocol}//${window.location.host}/api/terminal/ws?${params}`;
}

function bearerProtocols(): string[] {
  const token = new URLSearchParams(window.location.search).get("token");
  return token ? [`archon.bearer.${token}`] : [];
}

function sendResize(socket: WebSocket, cols: number, rows: number) {
  if (socket.readyState !== WebSocket.OPEN) return;
  socket.send(JSON.stringify({ type: "resize", cols, rows }));
}

function label(state: ConnectionState): string {
  switch (state) {
    case "connecting":
      return "connecting";
    case "open":
      return "connected";
    case "refused":
      return "unavailable";
    case "closed":
      return "ended";
  }
}

function closeDetail(event: CloseEvent): string {
  if (event.reason) return event.reason;
  return event.code === 1000 ? "" : `close code ${event.code}`;
}

/**
 * Transparent background so the pane inherits the workbench surface; the rest
 * is xterm's default palette, which is what the TUI is developed against.
 */
function terminalTheme() {
  const styles = getComputedStyle(document.documentElement);
  const foreground = styles.getPropertyValue("--text").trim();
  return {
    background: "#00000000",
    foreground: foreground || "#e6e6e6",
  };
}
