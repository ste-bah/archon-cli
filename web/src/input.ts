/// Input area — handles text entry, Ctrl+Enter submit, and file upload.

export type SubmitHandler = (text: string, files: File[]) => void;

export class InputArea {
  private readonly textarea: HTMLTextAreaElement;
  private readonly fileInput: HTMLInputElement;
  private readonly sendBtn: HTMLButtonElement;
  private onSubmitHandler: SubmitHandler | null = null;
  private pendingFiles: File[] = [];

  constructor(form: HTMLFormElement) {
    this.textarea = form.querySelector<HTMLTextAreaElement>("#chat-input")!;
    this.fileInput = form.querySelector<HTMLInputElement>("#file-upload")!;
    this.sendBtn = form.querySelector<HTMLButtonElement>("#send-btn")!;

    this.textarea.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" && ev.ctrlKey) {
        ev.preventDefault();
        this.submit();
      }
    });

    form.addEventListener("submit", (ev) => {
      ev.preventDefault();
      this.submit();
    });

    this.fileInput.addEventListener("change", () => {
      if (this.fileInput.files) {
        this.pendingFiles.push(...Array.from(this.fileInput.files));
      }
      this.fileInput.value = "";
    });
  }

  onSubmit(handler: SubmitHandler): void {
    this.onSubmitHandler = handler;
  }

  setEnabled(enabled: boolean): void {
    this.textarea.disabled = !enabled;
    this.sendBtn.disabled = !enabled;
  }

  clear(): void {
    this.textarea.value = "";
    this.pendingFiles = [];
  }

  focus(): void {
    this.textarea.focus();
  }

  private submit(): void {
    const text = this.textarea.value.trim();
    if (!text && this.pendingFiles.length === 0) return;
    const files = [...this.pendingFiles];
    this.clear();
    this.onSubmitHandler?.(text, files);
  }
}
