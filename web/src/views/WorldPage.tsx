import { Activity, Boxes, GitBranch, Sparkles } from "lucide-react";
import { useState } from "react";
import { apiClient } from "../api/client";
import { StatusPill } from "../components/StatusPill";
import type { WorldInspectionSummary, WorldModelRowPreview } from "../api/generated/web";
import "./WorldPage.css";

interface WorldPageProps {
  world?: WorldInspectionSummary;
}

export function WorldPage({ world }: WorldPageProps) {
  const [actionPreview, setActionPreview] = useState<string | null>(null);
  const [surfaceFilter, setSurfaceFilter] = useState("all");
  const artifacts = world?.artifacts ?? [];
  const advisorEvents = world?.advisorEvents ?? [];
  const signals = world?.signals ?? [];
  const candidates = world?.candidates ?? [];
  const predictions = world?.predictions ?? [];
  const visibleArtifacts = artifacts.filter((item) => matchesSurface(item.kind, surfaceFilter));
  const visibleSignals = signals.filter((item) => matchesSurface(item.label, surfaceFilter));
  const visiblePredictions = predictions.filter((item) => matchesSurface(item.surface, surfaceFilter));
  const visibleAdvisorEvents = advisorEvents.filter((item) => matchesSurface(item.surface, surfaceFilter));
  const visibleCandidates = surfaceFilter === "all" || surfaceFilter === "candidate" ? candidates : [];
  const visibleReasoning =
    surfaceFilter === "all" || surfaceFilter === "reasoning" ? world?.reasoningEvents : [];

  async function previewWorldAction(actionKind: string, summary: string) {
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
    <section className="world-layout">
      <div className="panel panel--wide">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Prediction</span>
            <h3>World model and reasoning quality</h3>
          </div>
          <StatusPill tone={world?.dbPresent ? "good" : "warn"}>
            {world?.dbPresent ? "model store present" : "model store missing"}
          </StatusPill>
        </div>
        <div className="world-metrics">
          <WorldMetric
            icon={<Boxes size={18} aria-hidden="true" />}
            label="Candidates"
            value={candidates.length || world?.candidateCount || 0}
            detail="checkpoint registry"
            active={surfaceFilter === "candidate"}
            onClick={() => setSurfaceFilter("candidate")}
          />
          <WorldMetric
            icon={<Activity size={18} aria-hidden="true" />}
            label="Advisor events"
            value={advisorEvents.length}
            detail="recent availability records"
            active={surfaceFilter === "advisor"}
            onClick={() => setSurfaceFilter("advisor")}
          />
          <WorldMetric
            icon={<Sparkles size={18} aria-hidden="true" />}
            label="Reasoning bridge"
            value={world?.reasoningRootPresent ? "ready" : "missing"}
            detail="first-class reasoning rows"
            active={surfaceFilter === "reasoning"}
            onClick={() => setSurfaceFilter("reasoning")}
          />
        </div>
        <div className="world-controls">
          <button type="button" onClick={() => setSurfaceFilter("all")}>
            Show all
          </button>
          <button
            type="button"
            onClick={() => previewWorldAction("world.candidate.promote", "promote selected candidate")}
          >
            Preview promote
          </button>
          <button
            type="button"
            onClick={() => previewWorldAction("world.active.rollback", "rollback active checkpoint")}
          >
            Preview rollback
          </button>
        </div>
        {actionPreview && (
          <div className="world-action-preview" role="status">
            <strong>Action preview</strong>
            <span>{actionPreview}</span>
          </div>
        )}
      </div>

      <section className="panel">
        <div className="panel-heading">
          <h3>Persisted artifacts</h3>
          <StatusPill>{visibleArtifacts.length} paths</StatusPill>
        </div>
        <div className="world-list">
          {visibleArtifacts.map((artifact) => (
            <button
              key={`${artifact.kind}:${artifact.path}`}
              type="button"
              className="world-row"
              onClick={() => setSurfaceFilter(kindSurface(artifact.kind))}
            >
              <div>
                <strong>{artifact.label}</strong>
                <span>{artifact.path}</span>
                <small>
                  {artifact.files} files · {formatBytes(artifact.bytes)}
                </small>
              </div>
              <StatusPill tone={statusTone(artifact.status)}>
                {artifact.status}
              </StatusPill>
            </button>
          ))}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h3>Promotion gate drilldown</h3>
          <StatusPill>{visibleSignals.length} checks</StatusPill>
        </div>
        <div className="world-list">
          {visibleSignals.map((signal) => (
            <button key={`${signal.label}:${signal.detail}`} type="button" className="world-row">
              <div>
                <strong>{signal.label}</strong>
                <span>{signal.detail}</span>
              </div>
              <StatusPill tone={statusTone(signal.status)}>
                {signal.status}
              </StatusPill>
            </button>
          ))}
        </div>
      </section>

      <section className="panel panel--wide">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Predictions</span>
            <h3>Advisor predictions and candidates</h3>
          </div>
          <StatusPill>{visiblePredictions.length} predictions</StatusPill>
        </div>
        <div className="world-event-grid">
          {visiblePredictions.map((prediction) => (
            <button
              key={`${prediction.sessionId}:${prediction.surface}:${prediction.status}`}
              className="world-event"
              type="button"
              onClick={() => setSurfaceFilter(kindSurface(prediction.surface))}
            >
              <header>
                <strong>{prediction.label}</strong>
                <StatusPill tone={statusTone(prediction.status)}>
                  {prediction.status}
                </StatusPill>
              </header>
              <p>{prediction.detail}</p>
              <footer>
                <span>{prediction.surface}</span>
                <span>{prediction.sessionId || "session unknown"}</span>
              </footer>
            </button>
          ))}
          {visiblePredictions.length === 0 && (
            <div className="world-empty">
              <GitBranch size={18} aria-hidden="true" />
              <span>No prediction rows found yet.</span>
            </div>
          )}
        </div>
      </section>

      <WorldRows
        title="Candidate checkpoints"
        rows={visibleCandidates}
        onPreview={(row) => previewWorldAction("world.candidate.promote", row.detail)}
      />
      <WorldRows title="Reasoning-quality rows" rows={visibleReasoning} />
      <WorldRows title="Shadow reports" rows={world?.shadowReports} />

      <section className="panel panel--wide">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Availability</span>
            <h3>Recent advisor events</h3>
          </div>
          <StatusPill>{visibleAdvisorEvents.length} rows</StatusPill>
        </div>
        <div className="world-event-grid">
          {visibleAdvisorEvents.length === 0 ? (
            <div className="world-empty">
              <GitBranch size={18} aria-hidden="true" />
              <span>No advisor ledger rows found yet.</span>
            </div>
          ) : (
            visibleAdvisorEvents.map((event) => (
              <button
                key={`${event.createdAt}:${event.sessionId}:${event.reason}`}
                className="world-event"
                type="button"
                onClick={() => setSurfaceFilter(kindSurface(event.surface))}
              >
                <header>
                  <strong>{event.surface}</strong>
                  <StatusPill tone={event.reason === "available" ? "good" : "warn"}>
                    {event.reason}
                  </StatusPill>
                </header>
                <p>{event.actionSummary}</p>
                <footer>
                  <span>{event.sessionId || "session unknown"}</span>
                  <span>{event.createdAt || "time unknown"}</span>
                </footer>
              </button>
            ))
          )}
        </div>
      </section>
    </section>
  );
}

function WorldRows({
  title,
  rows = [],
  onPreview,
}: {
  title: string;
  rows?: WorldModelRowPreview[];
  onPreview?: (row: WorldModelRowPreview) => void;
}) {
  return (
    <section className="panel">
      <div className="panel-heading">
        <h3>{title}</h3>
        <StatusPill>{rows.length} rows</StatusPill>
      </div>
      <div className="world-list">
        {rows.length === 0 ? (
          <div className="world-row">
            <div>
              <strong>No rows found yet.</strong>
              <span>read-only world-model surface</span>
            </div>
          </div>
        ) : (
          rows.map((row) => (
            <div key={`${row.kind}:${row.path}:${row.label}`} className="world-row">
              <div>
                <strong>{row.label}</strong>
                <span>{row.detail}</span>
                <small>{row.path}</small>
              </div>
              <span className="world-row__actions">
                <StatusPill>{row.kind}</StatusPill>
                {onPreview && (
                  <button type="button" onClick={() => onPreview(row)}>
                    Preview promote
                  </button>
                )}
              </span>
            </div>
          ))
        )}
      </div>
    </section>
  );
}

function WorldMetric({
  icon,
  label,
  value,
  detail,
  active = false,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  value: string | number;
  detail: string;
  active?: boolean;
  onClick?: () => void;
}) {
  return (
    <button
      type="button"
      className={active ? "world-metric world-metric--active" : "world-metric"}
      aria-label={label}
      onClick={onClick}
    >
      <span className="world-metric__icon">{icon}</span>
      <span className="metric-tile__label">{label}</span>
      <strong>{value}</strong>
      <span className="metric-tile__detail">{detail}</span>
    </button>
  );
}

function matchesSurface(value: string, filter: string) {
  return filter === "all" || kindSurface(value) === filter;
}

function kindSurface(value: string) {
  const normalized = value.toLowerCase();
  if (normalized.includes("candidate")) {
    return "candidate";
  }
  if (normalized.includes("reasoning")) {
    return "reasoning";
  }
  if (normalized.includes("advisor") || normalized.includes("provider")) {
    return "advisor";
  }
  return "all";
}

function statusTone(status: string): "good" | "warn" | "muted" {
  if (["present", "active", "ready", "available"].includes(status)) {
    return "good";
  }
  if (["missing", "cold_start", "stale", "unavailable"].includes(status)) {
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
