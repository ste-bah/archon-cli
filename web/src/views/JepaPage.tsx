import { Boxes, CheckCircle2, Cpu, Gauge } from "lucide-react";
import { useState } from "react";
import { apiClient } from "../api/client";
import { StatusPill } from "../components/StatusPill";
import type {
  JepaInspectionSummary,
  WorldInspectionSummary,
  WorldModelRowPreview,
} from "../api/generated/web";
import "./JepaPage.css";

interface JepaPageProps {
  world?: WorldInspectionSummary;
}

type JepaRow = WorldModelRowPreview & { group: string };

export function JepaPage({ world }: JepaPageProps) {
  const jepa = world?.jepa;
  const [filter, setFilter] = useState("all");
  const [selected, setSelected] = useState<JepaRow | undefined>();
  const [actionPreview, setActionPreview] = useState<string | null>(null);
  const rows = rowsFor(jepa);
  const visibleRows = filter === "all" ? rows : rows.filter((row) => row.kind === filter);
  const active = selected ?? visibleRows[0];

  async function previewAction(actionKind: string, summary: string) {
    const response = await apiClient.evaluateAction({
      actionId: `${actionKind}:${summary}`,
      actionKind,
      dryRun: true,
      payloadSummary: summary,
      confirmationToken: null,
    });
    setActionPreview(response.decision.policyReason);
  }

  return (
    <section className="jepa-layout">
      <div className="panel panel--wide">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Representation learning</span>
            <h3>JEPA candidates and gates</h3>
          </div>
          <StatusPill tone={jepa?.root.exists ? "good" : "warn"}>
            {jepa?.root.exists ? "JEPA store present" : "JEPA store missing"}
          </StatusPill>
        </div>
        <div className="jepa-metrics">
          <JepaMetric icon={<Boxes size={18} />} label="Candidates" value={jepa?.candidateCount ?? 0} />
          <JepaMetric icon={<CheckCircle2 size={18} />} label="Evals" value={jepa?.evalCount ?? 0} />
          <JepaMetric icon={<Cpu size={18} />} label="Training runs" value={jepa?.trainingRunCount ?? 0} />
          <JepaMetric icon={<Gauge size={18} />} label="Comparisons" value={jepa?.comparisonCount ?? 0} />
        </div>
        <div className="jepa-controls" aria-label="JEPA row filters">
          {["all", "candidate", "eval", "training", "comparison"].map((option) => (
            <button
              key={option}
              type="button"
              className={filter === option ? "jepa-filter jepa-filter--active" : "jepa-filter"}
              onClick={() => {
                setFilter(option);
                setSelected(undefined);
              }}
            >
              {option}
            </button>
          ))}
        </div>
        <div className="jepa-controls">
          <button type="button" onClick={() => previewAction("world.jepa.train", "train JEPA candidate")}>
            Preview train
          </button>
          <button type="button" onClick={() => previewAction("world.jepa.eval", active?.label ?? "eval JEPA candidate")}>
            Preview eval
          </button>
          <button type="button" onClick={() => previewAction("world.jepa.promote", active?.label ?? "promote JEPA candidate")}>
            Preview promote
          </button>
        </div>
        {actionPreview && (
          <div className="jepa-detail" role="status">
            <strong>Action preview</strong>
            <span>{actionPreview}</span>
          </div>
        )}
      </div>

      <section className="panel">
        <div className="panel-heading">
          <h3>Gate signals</h3>
          <StatusPill>{jepa?.signals.length ?? 0} checks</StatusPill>
        </div>
        <div className="jepa-list">
          {(jepa?.signals ?? []).map((signal) => (
            <button key={signal.label} type="button" className="jepa-row">
              <span>
                <strong>{signal.label}</strong>
                <small>{signal.detail}</small>
              </span>
              <StatusPill tone={statusTone(signal.status)}>{signal.status}</StatusPill>
            </button>
          ))}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h3>JEPA artifacts</h3>
          <StatusPill>{jepa?.artifacts.length ?? 0} paths</StatusPill>
        </div>
        <div className="jepa-list">
          {(jepa?.artifacts ?? []).map((artifact) => (
            <button
              key={`${artifact.kind}:${artifact.path}`}
              type="button"
              className="jepa-row"
              onClick={() => setFilter(kindFilter(artifact.kind))}
            >
              <span>
                <strong>{artifact.label}</strong>
                <small>{artifact.path}</small>
                <small>{artifact.files} files · {formatBytes(artifact.bytes)}</small>
              </span>
              <StatusPill tone={statusTone(artifact.status)}>{artifact.status}</StatusPill>
            </button>
          ))}
        </div>
      </section>

      <section className="panel panel--wide">
        <div className="panel-heading">
          <h3>{filter === "all" ? "JEPA rows" : `${filter} rows`}</h3>
          <StatusPill>{visibleRows.length} rows</StatusPill>
        </div>
        <div className="jepa-grid">
          <div className="jepa-list">
            {visibleRows.length === 0 ? (
              <div className="jepa-row jepa-row--empty">
                <span>No JEPA rows match this filter yet.</span>
              </div>
            ) : (
              visibleRows.map((row) => (
                <button
                  key={`${row.kind}:${row.path}:${row.label}`}
                  type="button"
                  className={active?.path === row.path ? "jepa-row jepa-row--active" : "jepa-row"}
                  onClick={() => setSelected(row)}
                >
                  <span>
                    <strong>{row.label}</strong>
                    <small>{row.detail}</small>
                    <small>{row.path}</small>
                  </span>
                  <StatusPill>{row.group}</StatusPill>
                </button>
              ))
            )}
          </div>
          <aside className="jepa-detail">
            <strong>{active?.label ?? "No JEPA row selected"}</strong>
            <span>{active?.detail ?? "Train or evaluate a JEPA candidate to populate this surface."}</span>
            <small>{active?.path ?? jepa?.root.path ?? "JEPA store unavailable"}</small>
          </aside>
        </div>
      </section>
    </section>
  );
}

function rowsFor(jepa?: JepaInspectionSummary): JepaRow[] {
  if (!jepa) {
    return [];
  }
  return [
    ...tag(jepa.candidates, "candidate"),
    ...tag(jepa.evals, "eval"),
    ...tag(jepa.trainingRuns, "training"),
    ...tag(jepa.comparisons, "comparison"),
  ];
}

function tag(rows: WorldModelRowPreview[], group: string): JepaRow[] {
  return rows.map((row) => ({ ...row, group }));
}

function kindFilter(kind: string) {
  return ["candidate", "eval", "training", "comparison"].includes(kind) ? kind : "all";
}

function JepaMetric({
  icon,
  label,
  value,
}: {
  icon: React.ReactNode;
  label: string;
  value: string | number;
}) {
  return (
    <section className="jepa-metric" aria-label={label}>
      <span>{icon}</span>
      <small>{label}</small>
      <strong>{value}</strong>
    </section>
  );
}

function statusTone(status: string): "good" | "warn" | "muted" {
  if (["present", "active", "ready", "available", "passed"].includes(status)) {
    return "good";
  }
  if (["missing", "failed", "stale", "unavailable"].includes(status)) {
    return "warn";
  }
  return "muted";
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
