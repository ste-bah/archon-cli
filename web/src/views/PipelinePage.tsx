import { Boxes, FileText, Gauge, Workflow } from "lucide-react";
import { StatusPill } from "../components/StatusPill";
import type { PipelineSummary } from "../api/generated/web";
import "./PipelinePage.css";

interface PipelinePageProps {
  pipelines?: PipelineSummary;
}

export function PipelinePage({ pipelines }: PipelinePageProps) {
  const stages = pipelines?.stages ?? [];
  const agents = pipelines?.agents ?? [];
  const runs = pipelines?.runs ?? [];
  const outputs = pipelines?.outputs ?? [];
  const liveEvents = pipelines?.liveEvents ?? [];

  return (
    <section className="pipeline-layout">
      <div className="panel panel--wide">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Execution</span>
            <h3>Pipeline control room</h3>
          </div>
          <StatusPill tone={runs.length > 0 ? "good" : "muted"}>
            {runs.length} recent runs
          </StatusPill>
        </div>
        <div className="pipeline-metrics">
          <PipelineMetric
            icon={<Workflow size={18} aria-hidden="true" />}
            label="Stages"
            value={stages.length}
            detail="visible swimlane groups"
          />
          <PipelineMetric
            icon={<Gauge size={18} aria-hidden="true" />}
            label="Sessions"
            value={pipelines?.sessionCount ?? 0}
            detail="recent activity ledgers"
          />
          <PipelineMetric
            icon={<Boxes size={18} aria-hidden="true" />}
            label="Agents"
            value={agents.length}
            detail="sampled prompt files"
          />
          <PipelineMetric
            icon={<FileText size={18} aria-hidden="true" />}
            label="Live events"
            value={liveEvents.length}
            detail="activity tail rows"
          />
        </div>
      </div>

      <section className="panel panel--wide">
        <div className="panel-heading">
          <h3>Stage swimlane</h3>
          <StatusPill>{stages.length} stages</StatusPill>
        </div>
        <div className="stage-lane">
          {stages.map((stage) => (
            <article key={`${stage.family}:${stage.label}`} className="stage-card">
              <header>
                <strong>{stage.label}</strong>
                <StatusPill tone={statusTone(stage.status)}>{stage.status}</StatusPill>
              </header>
              <span>{stage.family}</span>
              <small>{stage.agentCount} agents</small>
            </article>
          ))}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h3>Recent runs</h3>
          <StatusPill>{runs.length} tracked</StatusPill>
        </div>
        <div className="pipeline-list">
          {runs.map((run) => (
            <div key={run.path} className="pipeline-row">
              <div>
                <strong>{run.runId}</strong>
                <span>{run.path}</span>
                <small>
                  {run.family} · updated {run.updatedAt}
                </small>
              </div>
              <StatusPill tone={statusTone(run.status)}>{run.status}</StatusPill>
            </div>
          ))}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h3>Agent responsibilities</h3>
          <StatusPill>{agents.length} visible</StatusPill>
        </div>
        <div className="pipeline-list">
          {agents.map((agent) => (
            <div key={agent.path} className="pipeline-row">
              <div>
                <strong>{agent.name}</strong>
                <span>{agent.responsibility}</span>
                <small>{agent.family}</small>
              </div>
              <StatusPill tone={statusTone(agent.status)}>{agent.status}</StatusPill>
            </div>
          ))}
        </div>
      </section>

      <section className="panel panel--wide">
        <div className="panel-heading">
          <h3>Active run stream</h3>
          <StatusPill>{liveEvents.length} events</StatusPill>
        </div>
        <div className="pipeline-event-grid">
          {liveEvents.length === 0 ? (
            <div className="pipeline-empty">
              <FileText size={18} aria-hidden="true" />
              <span>No activity ledger events found yet.</span>
            </div>
          ) : (
            liveEvents.map((event) => (
              <article key={`${event.sessionId}:${event.path}`} className="pipeline-event">
                <header>
                  <strong>{event.sessionId}</strong>
                  <StatusPill tone={statusTone(event.status)}>{event.eventType}</StatusPill>
                </header>
                <p>{event.summary}</p>
                <small>{event.path}</small>
              </article>
            ))
          )}
        </div>
      </section>

      <section className="panel panel--wide">
        <div className="panel-heading">
          <h3>Artifacts and outputs</h3>
          <StatusPill>{outputs.length} files</StatusPill>
        </div>
        <div className="output-grid">
          {outputs.length === 0 ? (
            <div className="pipeline-empty">
              <FileText size={18} aria-hidden="true" />
              <span>No pipeline artifacts found yet.</span>
            </div>
          ) : (
            outputs.map((output) => (
              <article key={output.path} className="output-card">
                <strong>{output.label}</strong>
                <span>{output.path}</span>
                <p>{output.tail}</p>
                <footer>
                  <StatusPill>{output.kind}</StatusPill>
                  <small>
                    {formatBytes(output.bytes)} · {output.updatedAt}
                  </small>
                </footer>
              </article>
            ))
          )}
        </div>
      </section>
    </section>
  );
}

function PipelineMetric({
  icon,
  label,
  value,
  detail,
}: {
  icon: React.ReactNode;
  label: string;
  value: string | number;
  detail: string;
}) {
  return (
    <section className="pipeline-metric" aria-label={label}>
      <span className="pipeline-metric__icon">{icon}</span>
      <span className="metric-tile__label">{label}</span>
      <strong>{value}</strong>
      <span className="metric-tile__detail">{detail}</span>
    </section>
  );
}

function statusTone(status: string): "good" | "warn" | "muted" {
  if (["ready", "completed", "active", "present"].includes(status)) {
    return "good";
  }
  if (["failed", "missing", "blocked"].includes(status)) {
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
