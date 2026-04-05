/// Session list sidebar — create, resume, and switch sessions.

export interface SessionInfo {
  id: string;
  name: string;
  createdAt: number;
}

export type SessionChangeHandler = (sessionId: string) => void;

export class SessionList {
  private sessions: SessionInfo[] = [];
  private activeId: string | null = null;
  private onChangeHandler: SessionChangeHandler | null = null;

  constructor(
    private readonly listEl: HTMLUListElement,
    private readonly newBtn: HTMLButtonElement,
  ) {
    this.newBtn.addEventListener("click", () => {
      this.createSession();
    });
  }

  onChange(handler: SessionChangeHandler): void {
    this.onChangeHandler = handler;
  }

  setSessions(sessions: SessionInfo[]): void {
    this.sessions = sessions;
    this.render();
  }

  addSession(session: SessionInfo): void {
    this.sessions.unshift(session);
    this.render();
  }

  setActive(id: string): void {
    this.activeId = id;
    this.render();
  }

  exportConversation(): string {
    return JSON.stringify(
      { sessionId: this.activeId, exportedAt: Date.now() },
      null,
      2,
    );
  }

  private createSession(): void {
    const id = crypto.randomUUID();
    const session: SessionInfo = {
      id,
      name: `Session ${this.sessions.length + 1}`,
      createdAt: Date.now(),
    };
    this.addSession(session);
    this.activateSession(id);
  }

  private activateSession(id: string): void {
    this.activeId = id;
    this.render();
    this.onChangeHandler?.(id);
  }

  private render(): void {
    this.listEl.innerHTML = "";
    for (const session of this.sessions) {
      const li = document.createElement("li");
      li.textContent = session.name;
      li.title = session.id;
      if (session.id === this.activeId) li.classList.add("active");
      li.addEventListener("click", () => this.activateSession(session.id));
      this.listEl.appendChild(li);
    }
  }
}
