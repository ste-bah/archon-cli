import { BrainCircuit, CheckCircle2, GitPullRequest, RotateCw } from "lucide-react";
import type { ReactNode } from "react";
import { StatusPill } from "../components/StatusPill";
import type { CognitiveRowPreview, CognitiveWebSummary } from "../api/generated/web";
import "./MemoryPage.css";

interface CognitivePageProps {
  cognitive?: CognitiveWebSummary;
}

export function CognitivePage({ cognitive }: CognitivePageProps) {
  const latestTick = cognitive?.latestTick;
  return (
    <section className="memory-layout">
      <div className="panel panel--wide">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Executive loop</span>
            <h3>Cognitive state and autonomous learning</h3>
          </div>
          <StatusPill tone={cognitive?.storePresent ? "good" : "warn"}>
            {cognitive?.storePresent ? "cognitive store present" : "cognitive store missing"}
          </StatusPill>
        </div>
        <div className="metric-grid">
          <CognitiveMetric
            icon={<BrainCircuit size={18} aria-hidden="true" />}
            label="Situations"
            value={cognitive?.situationCount ?? 0}
            detail="classified turn records"
          />
          <CognitiveMetric
            icon={<CheckCircle2 size={18} aria-hidden="true" />}
            label="Decisions"
            value={cognitive?.executiveDecisionCount ?? 0}
            detail={`${cognitive?.toolDecisionCount ?? 0} tool gate outcomes`}
          />
          <CognitiveMetric
            icon={<GitPullRequest size={18} aria-hidden="true" />}
            label="Proposals"
            value={cognitive?.proposalCount ?? 0}
            detail={`${cognitive?.applyResultCount ?? 0} apply results`}
          />
        </div>
        <div className="memory-action-preview" role="status">
          <strong>Store</strong>
          <span>{cognitive?.store.path ?? "No cognitive store path reported yet"}</span>
        </div>
        {latestTick && (
          <div className="memory-action-preview" role="status">
            <strong>Latest tick</strong>
            <span>
              {latestTick.proposalsEvaluated} evaluated, {latestTick.proposalsAutoApplied} applied,
              {latestTick.proposalsDenied} denied, {latestTick.errorCount} errors
            </span>
          </div>
        )}
      </div>

      <CognitiveRows title="Recent decisions" rows={cognitive?.decisions} />
      <CognitiveRows title="Reflections and lessons" rows={cognitive?.reflections} />
      <CognitiveRows title="Governed proposals" rows={cognitive?.proposals} />

      <section className="panel">
        <div className="panel-heading">
          <h3>Self-model</h3>
          <StatusPill>{cognitive?.selfModelFactCount ?? 0} facts</StatusPill>
        </div>
        <div className="memory-list">
          <div className="memory-row">
            <div>
              <strong>Reflections</strong>
              <span>{cognitive?.reflectionCount ?? 0} safe lesson summaries stored</span>
            </div>
          </div>
          <div className="memory-row">
            <div>
              <strong>Maintenance</strong>
              <span>{latestTick ? `last tick ${latestTick.createdAt}` : "no tick recorded yet"}</span>
            </div>
            <RotateCw size={16} aria-hidden="true" />
          </div>
        </div>
      </section>
    </section>
  );
}

function CognitiveMetric({
  icon,
  label,
  value,
  detail,
}: {
  icon: ReactNode;
  label: string;
  value: number;
  detail: string;
}) {
  return (
    <div className="metric-tile" aria-label={`${label}: ${value}`}>
      <span className="metric-tile__label">{icon}{label}</span>
      <strong>{value}</strong>
      <span className="metric-tile__detail">{detail}</span>
    </div>
  );
}

function CognitiveRows({
  title,
  rows = [],
}: {
  title: string;
  rows?: CognitiveRowPreview[];
}) {
  return (
    <section className="panel">
      <div className="panel-heading">
        <h3>{title}</h3>
        <StatusPill>{rows.length} rows</StatusPill>
      </div>
      <div className="memory-list">
        {rows.length === 0 ? (
          <div className="memory-row memory-row--empty">
            <div>
              <strong>No rows found yet.</strong>
              <span>read-only cognitive surface</span>
            </div>
          </div>
        ) : (
          rows.map((row) => (
            <div key={`${row.id}:${row.createdAt}`} className="memory-row">
              <div>
                <strong>{row.label}</strong>
                <span>{row.detail}</span>
              </div>
              <StatusPill>{row.status}</StatusPill>
            </div>
          ))
        )}
      </div>
    </section>
  );
}
