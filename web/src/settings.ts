/// Settings panel — model, provider, effort controls.

export interface SettingsValues {
  model: string;
  provider: string;
  effort: string;
}

export type SettingsSaveHandler = (settings: SettingsValues) => void;

export class SettingsPanel {
  private readonly panel: HTMLElement;
  private readonly modelInput: HTMLInputElement;
  private readonly providerSelect: HTMLSelectElement;
  private readonly effortSelect: HTMLSelectElement;
  private onSaveHandler: SettingsSaveHandler | null = null;

  constructor(
    panel: HTMLElement,
    openBtn: HTMLButtonElement,
  ) {
    this.panel = panel;
    this.modelInput = panel.querySelector<HTMLInputElement>("#settings-model")!;
    this.providerSelect = panel.querySelector<HTMLSelectElement>("#settings-provider")!;
    this.effortSelect = panel.querySelector<HTMLSelectElement>("#settings-effort")!;

    openBtn.addEventListener("click", () => this.show());

    panel.querySelector<HTMLButtonElement>("#settings-close-btn")!
      .addEventListener("click", () => this.hide());

    panel.querySelector<HTMLButtonElement>("#settings-save-btn")!
      .addEventListener("click", () => {
        this.onSaveHandler?.(this.current());
        this.hide();
      });
  }

  onSave(handler: SettingsSaveHandler): void {
    this.onSaveHandler = handler;
  }

  current(): SettingsValues {
    return {
      model: this.modelInput.value.trim(),
      provider: this.providerSelect.value,
      effort: this.effortSelect.value,
    };
  }

  load(values: Partial<SettingsValues>): void {
    if (values.model) this.modelInput.value = values.model;
    if (values.provider) this.providerSelect.value = values.provider;
    if (values.effort) this.effortSelect.value = values.effort;
  }

  private show(): void {
    this.panel.hidden = false;
  }

  private hide(): void {
    this.panel.hidden = true;
  }
}
