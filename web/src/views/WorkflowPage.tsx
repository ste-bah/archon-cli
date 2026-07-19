import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Boxes, Code2, FileText, Gauge, Pause, Play, RotateCcw, ShieldCheck, Workflow, Wrench, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { apiClient } from "../api/client";
import type { WorkflowAgentView, WorkflowApprovalView, WorkflowBundleView, WorkflowRunSummary, WorkflowStageView, WorkflowV2BranchView, WorkflowV2ResultView, WorkflowWebSummary } from "../api/generated/web";
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
          <ControlButton icon={<ShieldCheck size={16} />} label="Run Once" onClick={() => submitControl("approve-run-once")} />
          <ControlButton icon={<ShieldCheck size={16} />} label="Always" onClick={() => submitControl("approve-always")} />
          <ControlButton icon={<X size={16} />} label="Deny" onClick={() => submitControl("deny-workflow")} />
          <ControlButton icon={<Play size={16} />} label="Continue" onClick={() => submitControl("continue")} />
          <ControlButton icon={<Wrench size={16} />} label="Repair" onClick={() => submitControl("repair")} />
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
        {detail.data?.bundle && <BundleSummary bundle={detail.data.bundle} />}
        {detail.data?.approval && <ApprovalSummary approval={detail.data.approval} />}
        <div className="pipeline-list">
          {detail.data?.stages.map((stage) => (
            <StageRow key={stage.id} stage={stage} onAction={submitStageControl} />
          )) ?? <EmptyRow>{detail.isLoading ? "Loading workflow detail." : "Select a run."}</EmptyRow>}
        </div>
      </section>

      <section className="panel panel--wide">
        <div className="panel-heading">
          <h3>Generated harness</h3>
          <StatusPill tone={detail.data?.harness ? "good" : "muted"}>{detail.data?.harness ? "available" : "missing"}</StatusPill>
        </div>
        {detail.data?.harness ? (
          <pre className="pipeline-code">{detail.data.harness}</pre>
        ) : (
          <EmptyRow>No harness recorded for this run.</EmptyRow>
        )}
      </section>

      <section className="panel panel--wide">
        <div className="panel-heading">
          <h3>Compiled spec</h3>
          <StatusPill tone={detail.data?.compiledSpec ? "good" : "muted"}>{detail.data?.compiledSpec ? "available" : "missing"}</StatusPill>
        </div>
        {detail.data?.compiledSpec ? (
          <pre className="pipeline-code">{detail.data.compiledSpec}</pre>
        ) : (
          <EmptyRow>No compiled workflow spec recorded for this run.</EmptyRow>
        )}
      </section>

      <section className="panel panel--wide">
        <div className="panel-heading">
          <h3>Agent details</h3>
          <StatusPill>{detail.data?.agents.length ?? 0} records</StatusPill>
        </div>
        <div className="pipeline-list">
          {detail.data?.agents.length ? (
            detail.data.agents.map((agent) => (
              <AgentRow key={`${agent.stageId}:${agent.itemId}`} agent={agent} onAction={submitAgentControl} />
            ))
          ) : (
            <EmptyRow>No agent records for this run.</EmptyRow>
          )}
        </div>
      </section>

      <section className="panel panel--wide">
        <div className="panel-heading">
          <h3>Generated calls</h3>
          <StatusPill>{detail.data?.v2Results.length ?? 0} calls</StatusPill>
        </div>
        <div className="pipeline-list">
          {detail.data?.v2Results.length ? (
            detail.data.v2Results.map((result) => <V2ResultRow key={result.callId} result={result} />)
          ) : (
            <EmptyRow>No generated call records for this run.</EmptyRow>
          )}
        </div>
      </section>

      <section className="panel panel--wide">
        <div className="panel-heading">
          <h3>Generated branches</h3>
          <StatusPill>{detail.data?.v2Branches.length ?? 0} branches</StatusPill>
        </div>
        <div className="pipeline-list">
          {detail.data?.v2Branches.length ? (
            detail.data.v2Branches.map((branch) => (
              <V2BranchRow key={`${branch.callId}:${branch.itemId}`} branch={branch} onAction={submitV2BranchControl} />
            ))
          ) : (
            <EmptyRow>No generated branch records for this run.</EmptyRow>
          )}
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
      itemId: null,
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
    control.mutate({ runId: selectedRun.id, action, stageId: stage.id, itemId: null, rationale, confirmationToken });
  }

  function submitAgentControl(action: string, agent: WorkflowAgentView) {
    if (!selectedRun?.id) {
      return;
    }
    const confirmationToken = window.prompt(`Confirm ${action} for ${agent.stageId}/${agent.itemId}`);
    if (!confirmationToken) {
      return;
    }
    control.mutate({
      runId: selectedRun.id,
      action,
      stageId: agent.stageId,
      itemId: agent.itemId,
      rationale: null,
      confirmationToken,
    });
  }

  function submitV2BranchControl(action: string, branch: WorkflowV2BranchView) {
    if (!selectedRun?.id) {
      return;
    }
    const confirmationToken = window.prompt(`Confirm ${action} for ${branch.callId}/${branch.itemId}`);
    if (!confirmationToken) {
      return;
    }
    control.mutate({
      runId: selectedRun.id,
      action,
      stageId: branch.callId,
      itemId: branch.itemId,
      rationale: null,
      confirmationToken,
    });
  }
}

function BundleSummary({ bundle }: { bundle: WorkflowBundleView }) {
  return (
    <div className="workflow-bundle">
      <Code2 size={16} aria-hidden="true" />
      <span>{bundle.workflowPath}</span>
      <span>{bundle.compiledSpecPath}</span>
      <span>{bundle.phaseCount} phases</span>
      <span>{bundle.maxParallelism} parallel</span>
      <span>{bundle.writeCapableStages.length} write stages</span>
    </div>
  );
}

function ApprovalSummary({ approval }: { approval: WorkflowApprovalView }) {
  const writeStages = approval.writeCapableStages.length ? approval.writeCapableStages.join(", ") : "none";
  const external = approval.externalRequirements.length ? approval.externalRequirements.join(", ") : "none";
  return (
    <div className="workflow-bundle">
      <ShieldCheck size={16} aria-hidden="true" />
      <span>{approval.decision ?? "pending"}</span>
      <span>{approval.decidedBy ?? "no decision"}</span>
      <span>{approval.phaseCount} phases</span>
      <span>{approval.maxAgents} agents</span>
      <span>write: {writeStages}</span>
      <span>external: {external}</span>
    </div>
  );
}

function AgentRow({ agent, onAction }: { agent: WorkflowAgentView; onAction: (action: string, agent: WorkflowAgentView) => void }) {
  const promptMeta = [
    agent.inputHash ? `input ${shortHash(agent.inputHash)}` : null,
    agent.promptHash ? `prompt ${shortHash(agent.promptHash)}` : null,
    agent.promptPath ?? null,
  ].filter(Boolean).join(" · ");
  return (
    <div className="pipeline-row">
      <div>
        <strong>{agent.stageId} / {agent.itemId}</strong>
        <span>{agent.provider ?? "provider"} · {agent.model ?? "model"} · {agent.tokensIn + agent.tokensOut} tokens</span>
        {promptMeta && <small>{promptMeta}</small>}
        {agent.resultPreview && <small>{agent.resultPreview}</small>}
        {agent.error && <small>{agent.error}</small>}
        {agent.recentPublicToolCalls.length > 0 && (
          <small>{agent.recentPublicToolCalls.length} recent public tool calls captured</small>
        )}
      </div>
      <div className="pipeline-row__actions">
        <StatusPill tone={statusTone(agent.status)}>{agent.status}</StatusPill>
        <button type="button" onClick={() => onAction("restart-item", agent)} aria-label={`Restart ${agent.stageId}/${agent.itemId}`}>
          <RotateCcw size={15} />
        </button>
      </div>
    </div>
  );
}

function V2ResultRow({ result }: { result: WorkflowV2ResultView }) {
  return (
    <div className="pipeline-row">
      <div>
        <strong>{result.callId}</strong>
        <span>{result.branchCount} branches · {result.artifactCount} artifacts</span>
        {result.summary && <small>{result.summary}</small>}
        <small>{result.resultPath}</small>
      </div>
      <StatusPill tone={statusTone(result.status)}>{result.status}</StatusPill>
    </div>
  );
}

function V2BranchRow({ branch, onAction }: { branch: WorkflowV2BranchView; onAction: (action: string, branch: WorkflowV2BranchView) => void }) {
  return (
    <div className="pipeline-row">
      <div>
        <strong>{branch.callId} / {branch.itemId}</strong>
        <span>{branch.role}</span>
        {branch.summary && <small>{branch.summary}</small>}
        {branch.error && <small>{branch.error}</small>}
        <small>{branch.outputPath}</small>
      </div>
      <div className="pipeline-row__actions">
        <StatusPill tone={statusTone(branch.status)}>{branch.status}</StatusPill>
        <button type="button" onClick={() => onAction("restart-item", branch)} aria-label={`Restart ${branch.callId}/${branch.itemId}`}>
          <RotateCcw size={15} />
        </button>
      </div>
    </div>
  );
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
  const timing = [
    stage.startedAt ? `started ${stage.startedAt}` : null,
    stage.completedAt ? `completed ${stage.completedAt}` : null,
  ].filter(Boolean).join(" · ");
  return (
    <div className="pipeline-row">
      <div>
        <strong>{stage.id}</strong>
        <span>attempts={stage.attempt} artifacts={stage.artifacts}</span>
        {timing && <small>{timing}</small>}
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

function shortHash(value: string) {
  return value.slice(0, 12);
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
  if (["accepted", "completed", "gated", "running", "forcedaccepted", "forced_accepted"].includes(status)) {
    return "good";
  }
  if (["failed", "blocked", "cancelled"].includes(status)) {
    return "warn";
  }
  return "muted";
}
