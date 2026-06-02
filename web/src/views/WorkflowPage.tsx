import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Boxes, FileText, Gauge, Pause, Play, RotateCcw, ShieldCheck, Workflow, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { apiClient } from "../api/client";
import type { WorkflowRunSummary, WorkflowStageView, WorkflowWebSummary } from "../api/generated/web";
import { StatusPill } from "../components/StatusPill";
import "./PipelinePage.css";

interface WorkflowPageProps {
  workflows?: WorkflowWebSummary;
}

export function WorkflowPage({ workflows }: WorkflowPageProps) {
  const queryClient = useQueryClient();
  const runs = workflows?.runs ?? [];
  const [selectedRunId, setSelectedRunId] = useState<string | undefined>(runs[0]?.id);
  const selectedRun = useMemo(
    () => runs.find((run) => run.id === selectedRunId) ?? runs[0],
    [runs, selectedRunId],
  );
  const detail = useQuery({
    queryKey: ["workflow-detail", selectedRun?.id],
    queryFn: () => apiClient.workflowDetail(selectedRun!.id),
    enabled: Boolean(selectedRun?.id),
  });
  const control = useMutation({
    mutationFn: apiClient.workflowControl,
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["workflows"] });
      void queryClient.invalidateQueries({ queryKey: ["workflow-detail"] });
    },
  });

  useEffect(() => {
    if (!selectedRunId && runs[0]?.id) {
      setSelectedRunId(runs[0].id);
    }
  }, [runs, selectedRunId]);

  useEffect(() => {
    if (!selectedRun?.id) {
      return;
    }
    const controller = new AbortController();
    apiClient
      .workflowEventStream(
        selectedRun.id,
        0,
        (incoming) => {
          if (incoming.length === 0) {
            return;
          }
          void queryClient.invalidateQueries({ queryKey: ["workflow-detail", selectedRun.id] });
          void queryClient.invalidateQueries({ queryKey: ["workflows"] });
        },
        controller.signal,
      )
      .catch((error: unknown) => {
        if (!controller.signal.aborted) {
          console.warn("workflow event stream failed", error);
        }
      });
    return () => controller.abort();
  }, [queryClient, selectedRun?.id]);

  const accepted = runs.reduce((sum, run) => sum + run.acceptedCount, 0);
  const failed = runs.reduce((sum, run) => sum + run.failedCount, 0);
  const events = detail.data?.events ?? workflows?.events ?? [];

  return (
    <section className="pipeline-layout">
      <div className="panel panel--wide">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Dynamic execution</span>
            <h3>Workflow control room</h3>
          </div>
          <StatusPill tone={runs.length > 0 ? "good" : "muted"}>{runs.length} runs</StatusPill>
        </div>
        <div className="pipeline-metrics">
          <WorkflowMetric icon={<Workflow size={18} />} label="Runs" value={runs.length} detail="durable states" />
          <WorkflowMetric icon={<Gauge size={18} />} label="Accepted" value={accepted} detail="accepted stages" />
          <WorkflowMetric icon={<Boxes size={18} />} label="Failed" value={failed} detail="failed stages" />
          <WorkflowMetric icon={<FileText size={18} />} label="Events" value={events.length} detail="live event view" />
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
              <button
                key={run.id}
                className="pipeline-row"
                onClick={() => setSelectedRunId(run.id)}
                type="button"
              >
                <RunSummary run={run} />
                <StatusPill tone={statusTone(run.status)}>{run.status}</StatusPill>
              </button>
            ))
          )}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h3>Run controls</h3>
          <StatusPill tone={control.isError ? "warn" : "muted"}>{control.isPending ? "busy" : "gated"}</StatusPill>
        </div>
        <div className="pipeline-list">
          <ControlButton icon={<Play size={16} />} label="Resume" onClick={() => submitControl("resume")} />
          <ControlButton icon={<Pause size={16} />} label="Pause" onClick={() => submitControl("pause")} />
          <ControlButton icon={<X size={16} />} label="Cancel" onClick={() => submitControl("cancel")} />
        </div>
        {control.data && <small>{control.data.policyReason}</small>}
      </section>

      <section className="panel panel--wide">
        <div className="panel-heading">
          <h3>{selectedRun?.name ?? "Workflow detail"}</h3>
          <StatusPill tone={statusTone(selectedRun?.status ?? "missing")}>{selectedRun?.status ?? "missing"}</StatusPill>
        </div>
        <div className="pipeline-list">
          {detail.data?.stages.map((stage) => (
            <StageRow key={stage.id} stage={stage} onAction={submitStageControl} />
          )) ?? <EmptyRow>{detail.isLoading ? "Loading workflow detail." : "Select a run."}</EmptyRow>}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h3>Artifacts</h3>
          <StatusPill>{detail.data?.artifacts.length ?? 0} files</StatusPill>
        </div>
        <div className="pipeline-list">
          {detail.data?.artifacts.length ? (
            detail.data.artifacts.map((artifact) => (
              <div key={artifact.id} className="pipeline-row">
                <div>
                  <strong>{artifact.producingStage}</strong>
                  <span>{artifact.path}</span>
                </div>
                <StatusPill>hash</StatusPill>
              </div>
            ))
          ) : (
            <EmptyRow>No artifacts for this run.</EmptyRow>
          )}
        </div>
      </section>

      <section className="panel panel--wide">
        <div className="panel-heading">
          <h3>Live event stream</h3>
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
                <small>{event.createdAt}</small>
              </article>
            ))
          )}
        </div>
      </section>
    </section>
  );

  function submitControl(action: string) {
    if (!selectedRun?.id) {
      return;
    }
    const confirmationToken = window.prompt(`Confirm ${action} for ${selectedRun.id}`);
    if (!confirmationToken) {
      return;
    }
    control.mutate({
      runId: selectedRun.id,
      action,
      stageId: null,
      rationale: null,
      confirmationToken,
    });
  }

  function submitStageControl(action: string, stage: WorkflowStageView) {
    if (!selectedRun?.id) {
      return;
    }
    const rationale = action === "force-accept" ? window.prompt(`Rationale for ${stage.id}`) : null;
    if (action === "force-accept" && !rationale) {
      return;
    }
    const confirmationToken = window.prompt(`Confirm ${action} for ${stage.id}`);
    if (!confirmationToken) {
      return;
    }
    control.mutate({ runId: selectedRun.id, action, stageId: stage.id, rationale, confirmationToken });
  }
}

function RunSummary({ run }: { run: WorkflowRunSummary }) {
  return (
    <div>
      <strong>{run.name}</strong>
      <span>{run.id}</span>
      <small>{run.acceptedCount}/{run.stageCount} accepted · {run.artifactCount} artifacts</small>
    </div>
  );
}

function StageRow({ stage, onAction }: { stage: WorkflowStageView; onAction: (action: string, stage: WorkflowStageView) => void }) {
  return (
    <div className="pipeline-row">
      <div>
        <strong>{stage.id}</strong>
        <span>attempts={stage.attempt} artifacts={stage.artifacts}</span>
        {stage.error && <small>{stage.error}</small>}
      </div>
      <div className="pipeline-row__actions">
        <StatusPill tone={statusTone(stage.status)}>{stage.status}</StatusPill>
        <button type="button" onClick={() => onAction("restart-stage", stage)} aria-label={`Restart ${stage.id}`}>
          <RotateCcw size={15} />
        </button>
        <button type="button" onClick={() => onAction("force-accept", stage)} aria-label={`Force accept ${stage.id}`}>
          <ShieldCheck size={15} />
        </button>
      </div>
    </div>
  );
}

function ControlButton({ icon, label, onClick }: { icon: React.ReactNode; label: string; onClick: () => void }) {
  return (
    <button className="pipeline-row" onClick={onClick} type="button">
      <div>
        <strong>{label}</strong>
        <span>policy-gated workflow action</span>
      </div>
      {icon}
    </button>
  );
}

function WorkflowMetric({ icon, label, value, detail }: { icon: React.ReactNode; label: string; value: string | number; detail: string }) {
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
  if (["accepted", "completed", "gated", "running", "forcedaccepted"].includes(status)) {
    return "good";
  }
  if (["failed", "blocked", "cancelled"].includes(status)) {
    return "warn";
  }
  return "muted";
}
