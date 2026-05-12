import { Check, Moon, Paintbrush, Sun } from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { apiClient } from "../api/client";
import { StatusPill } from "../components/StatusPill";
import type { SettingsSummary, WebThemeProfile } from "../api/generated/web";
import "./SettingsPage.css";

type ThemeMode = "dark" | "light";

interface SettingsPageProps {
  settings?: SettingsSummary;
  authRequired?: boolean;
  uploadsEnabled?: boolean;
  theme: ThemeMode;
  onThemeToggle: () => void;
}

const accents = [
  { id: "mint", label: "Mint", accent: "#87d8b4", strong: "#2fbc86" },
  { id: "blue", label: "Blue", accent: "#83b7ff", strong: "#3677d6" },
  { id: "amber", label: "Amber", accent: "#f4c471", strong: "#be7429" },
  { id: "rose", label: "Rose", accent: "#f0a0b6", strong: "#cf5578" },
];

export function SettingsPage({
  settings,
  authRequired = false,
  uploadsEnabled = false,
  theme,
  onThemeToggle,
}: SettingsPageProps) {
  const queryClient = useQueryClient();
  const themeProfile = useQuery({
    queryKey: ["theme-profile"],
    queryFn: apiClient.themeProfile,
  });
  const [density, setDensity] = useState(() =>
    window.localStorage.getItem("archon.density") === "compact" ? "compact" : "comfortable",
  );
  const [accentId, setAccentId] = useState(() =>
    window.localStorage.getItem("archon.accent") ?? "mint",
  );
  const selectedAccent = useMemo(
    () => accents.find((accent) => accent.id === accentId) ?? accents[0]!,
    [accentId],
  );
  const [importJson, setImportJson] = useState("");
  const [importError, setImportError] = useState("");

  const saveProfile = useMutation({
    mutationFn: apiClient.saveThemeProfile,
    onSuccess: (envelope) => {
      queryClient.setQueryData(["theme-profile"], envelope);
      setImportJson(envelope.exportJson);
      applyProfile(envelope.profile);
      setImportError("");
    },
  });

  useEffect(() => {
    document.documentElement.dataset.density = density;
    window.localStorage.setItem("archon.density", density);
  }, [density]);

  useEffect(() => {
    document.documentElement.style.setProperty("--accent", selectedAccent.accent);
    document.documentElement.style.setProperty("--accent-strong", selectedAccent.strong);
    window.localStorage.setItem("archon.accent", selectedAccent.id);
  }, [selectedAccent]);

  useEffect(() => {
    if (themeProfile.data?.exportJson) {
      setImportJson(themeProfile.data.exportJson);
    }
  }, [themeProfile.data?.exportJson]);

  function currentProfile(): WebThemeProfile {
    return {
      themeMode: theme,
      densityMode: density,
      accentId: selectedAccent.id,
      accentHex: selectedAccent.accent,
      accentStrongHex: selectedAccent.strong,
      updatedAtMs: Date.now(),
    };
  }

  function applyProfile(profile: WebThemeProfile) {
    setDensity(profile.densityMode === "compact" ? "compact" : "comfortable");
    setAccentId(profile.accentId);
    if ((profile.themeMode === "light" || profile.themeMode === "dark") && profile.themeMode !== theme) {
      onThemeToggle();
    }
  }

  function importProfile() {
    try {
      saveProfile.mutate({ profile: JSON.parse(importJson) as WebThemeProfile });
    } catch {
      setImportError("Import JSON could not be parsed.");
    }
  }

  return (
    <section className="settings-layout">
      <div className="panel panel--wide">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Operator</span>
            <h3>Theme and safe controls</h3>
          </div>
          <StatusPill tone={authRequired ? "warn" : "good"}>
            {authRequired ? "auth required" : "loopback mode"}
          </StatusPill>
        </div>
        <div className="settings-metrics">
          <SettingsMetric label="Theme modes" value={settings?.themeModes.length ?? 0} />
          <SettingsMetric label="Density modes" value={settings?.densityModes.length ?? 0} />
          <SettingsMetric label="Policy editing" value={settings?.policyEditingEnabled ? "on" : "off"} />
        </div>
      </div>

      <section className="panel">
        <div className="panel-heading">
          <h3>Theme</h3>
          <StatusPill>{theme}</StatusPill>
        </div>
        <button type="button" className="theme-switch" onClick={onThemeToggle}>
          {theme === "dark" ? <Sun size={18} aria-hidden="true" /> : <Moon size={18} aria-hidden="true" />}
          <span>{theme === "dark" ? "Light" : "Dark"}</span>
        </button>
        <div className="accent-grid" aria-label="Accent swatches">
          {accents.map((accent) => (
            <button
              key={accent.id}
              type="button"
              className={accent.id === accentId ? "accent-swatch accent-swatch--active" : "accent-swatch"}
              onClick={() => setAccentId(accent.id)}
              style={{ "--swatch": accent.accent } as React.CSSProperties}
            >
              <span aria-hidden="true" />
              <strong>{accent.label}</strong>
              {accent.id === accentId && <Check size={16} aria-hidden="true" />}
            </button>
          ))}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h3>Density</h3>
          <StatusPill>{density}</StatusPill>
        </div>
        <div className="segmented-control">
          {(settings?.densityModes ?? ["comfortable", "compact"]).map((mode) => (
            <button
              key={mode}
              type="button"
              className={density === mode ? "segment segment--active" : "segment"}
              onClick={() => setDensity(mode)}
            >
              {mode}
            </button>
          ))}
        </div>
        <div className="density-preview">
          <Paintbrush size={18} aria-hidden="true" />
          <span>Preview · {density}</span>
        </div>
      </section>

      <section className="panel panel--wide">
        <div className="panel-heading">
          <h3>Theme profile</h3>
          <StatusPill tone={themeProfile.data?.persisted ? "good" : "muted"}>
            {themeProfile.data?.persisted ? "persisted" : "default"}
          </StatusPill>
        </div>
        <div className="settings-profile-grid">
          <button
            type="button"
            className="settings-action"
            onClick={() => saveProfile.mutate({ profile: currentProfile() })}
          >
            Export current profile
          </button>
          <button type="button" className="settings-action" onClick={importProfile}>
            Import profile
          </button>
        </div>
        <textarea
          className="profile-export"
          value={importJson}
          onChange={(event) => setImportJson(event.target.value)}
          spellCheck={false}
          aria-label="Theme profile JSON"
        />
        <small className="settings-profile-path">
          {saveProfile.isPending
            ? "saving profile"
            : importError || themeProfile.data?.storagePath || "server profile not loaded"}
        </small>
      </section>

      <section className="panel panel--wide">
        <div className="panel-heading">
          <h3>Read-only policy posture</h3>
          <StatusPill tone="good">inspection only</StatusPill>
        </div>
        <div className="settings-policy-grid">
          <PolicyRow label="Uploads" value={uploadsEnabled ? "enabled" : "disabled"} good={uploadsEnabled} />
          <PolicyRow label="Policy editing" value={settings?.policyEditingEnabled ? "enabled" : "disabled"} good={!settings?.policyEditingEnabled} />
          <PolicyRow label="Filesystem open" value={settings?.directFilesystemOpenEnabled ? "enabled" : "disabled"} good={!settings?.directFilesystemOpenEnabled} />
          <PolicyRow label="Authentication" value={authRequired ? "required" : "loopback"} good={!authRequired} />
        </div>
      </section>
    </section>
  );
}

function SettingsMetric({ label, value }: { label: string; value: string | number }) {
  return (
    <section className="settings-metric" aria-label={label}>
      <span className="metric-tile__label">{label}</span>
      <strong>{value}</strong>
      <span className="metric-tile__detail">effective web setting</span>
    </section>
  );
}

function PolicyRow({ label, value, good }: { label: string; value: string; good: boolean }) {
  return (
    <div className="policy-row">
      <strong>{label}</strong>
      <StatusPill tone={good ? "good" : "warn"}>{value}</StatusPill>
    </div>
  );
}
