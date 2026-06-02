import { Boxes, FileText, Gauge, Workflow } from "lucide-react";
import { StatusPill } from "../components/StatusPill";
import type { WorkflowWebSummary } from "../api/generated/web";
import "./PipelinePage.css";

interface WorkflowPageProps {
  workflows?: WorkflowWebSummary;
}

export function WorkflowPage({ workflows }: WorkflowPageProps) {
  const runs = workflows?.runs ?? [];
  const events = workflows?.events ?? [];
  const controls = workflows?.controls ?? [];
  const accepted = runs.reduce((sum, run) => sum + run.acceptedCount, 0);
  const failed = runs.reduce((sum, run) => sum + run.failedCount, 0);

  return (
    <section className="pipeline-layout">
      <div className="panel panel--wide">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Dynamic execution</span>
            <h3>Workflow control room</h3>
          </div>
          <StatusPill tone={runs.length > 0 ? "good" : "muted"}>
            {runs.length} runs
          </StatusPill>
        </div>
        <div className="pipeline-metrics">
          <WorkflowMetric
            icon={<Workflow size={18} aria-hidden="true" />}
            label="Runs"
            value={runs.length}
            detail="durable workflow states"
          />
          <WorkflowMetric
            icon={<Gauge size={18} aria-hidden="true" />}
            label="Accepted"
            value={accepted}
            detail="accepted stages"
          />
          <WorkflowMetric
            icon={<Boxes size={18} aria-hidden="true" />}
            label="Failed"
            value={failed}
            detail="visible failed stages"
          />
          <WorkflowMetric
            icon={<FileText size={18} aria-hidden="true" />}
            label="Events"
            value={events.length}
            detail="sanitized event previews"
          />
        </div>
      </div>

      <section className="panel">
        <div className="panel-heading">
          <h3>Recent workflow runs</h3>
          <StatusPill>{runs.length} tracked</StatusPill>
        </div>
        <div className="pipeline-list">
          {runs.length === 0 ? (
            <EmptyRow>No workflow runs found in {workflows?.root ?? ".archon/workflows"}.</EmptyRow>
          ) : (
            runs.map((run) => (
              <div key={run.id} className="pipeline-row">
                <div>
                  <strong>{run.name}</strong>
                  <span>{run.id}</span>
                  <small>
                    {run.acceptedCount}/{run.stageCount} accepted · {run.artifactCount} artifacts
                  </small>
                </div>
                <StatusPill tone={statusTone(run.status)}>{run.status}</StatusPill>
              </div>
            ))
          )}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h3>Controls</h3>
          <StatusPill>{controls.length} actions</StatusPill>
        </div>
        <div className="pipeline-list">
          {controls.map((control) => (
            <div key={control.action} className="pipeline-row">
              <div>
                <strong>{control.action}</strong>
                <span>{control.policyReason}</span>
              </div>
              <StatusPill tone={control.enabled ? "good" : "warn"}>
                {control.enabled ? "gated" : "blocked"}
              </StatusPill>
            </div>
          ))}
        </div>
      </section>

      <section className="panel panel--wide">
        <div className="panel-heading">
          <h3>Sanitized event stream</h3>
          <StatusPill>{events.length} events</StatusPill>
        </div>
        <div className="pipeline-event-grid">
          {events.length === 0 ? (
            <EmptyRow>No workflow events recorded yet.</EmptyRow>
          ) : (
            events.map((event) => (
              <article key={`${event.runId}:${event.seq}`} className="pipeline-event">
                <header>
                  <strong>{event.summary}</strong>
                  <StatusPill tone={statusTone(event.status)}>{event.kind}</StatusPill>
                </header>
                <p>{event.status}</p>
                <small>
                  {event.runId} · {event.createdAt}
                </small>
              </article>
            ))
          )}
        </div>
      </section>
    </section>
  );
}

function WorkflowMetric({
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

function EmptyRow({ children }: { children: React.ReactNode }) {
  return (
    <div className="pipeline-empty">
      <FileText size={18} aria-hidden="true" />
      <span>{children}</span>
    </div>
  );
}

function statusTone(status: string): "good" | "warn" | "muted" {
  if (["accepted", "completed", "gated", "running"].includes(status)) {
    return "good";
  }
  if (["failed", "blocked", "cancelled"].includes(status)) {
    return "warn";
  }
  return "muted";
}
