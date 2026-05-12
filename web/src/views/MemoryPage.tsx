import { useState } from "react";
import { apiClient } from "../api/client";
import { StatusPill } from "../components/StatusPill";
import type { LearningRowPreview, LearningSummary } from "../api/generated/web";
import "./MemoryPage.css";

interface MemoryPageProps {
  learning?: LearningSummary;
}

export function MemoryPage({ learning }: MemoryPageProps) {
  const [filter, setFilter] = useState("all");
  const [actionPreview, setActionPreview] = useState<string | null>(null);
  const filterOptions = ["all", "memory", "learning_event", "proposal", "trust"];

  async function previewProposal(row: LearningRowPreview) {
    const response = await apiClient.evaluateAction({
      actionId: `proposal:${row.label}`,
      actionKind: "behaviour.proposal.approve",
      dryRun: true,
      payloadSummary: row.detail,
      confirmationToken: null,
    });
    setActionPreview(response.decision.policyReason);
  }

  return (
    <section className="memory-layout">
      <div className="panel panel--wide">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Learning</span>
            <h3>Memory and behaviour proposals</h3>
          </div>
          <StatusPill tone={learning?.reasoningStorePresent ? "good" : "warn"}>
            {learning?.reasoningStorePresent ? "reasoning store present" : "reasoning store missing"}
          </StatusPill>
        </div>
        <div className="metric-grid">
          <MemoryMetric label="Sessions" value={learning?.sessionCount ?? 0} />
          <MemoryMetric label="Stores" value={learning?.stores.length ?? 0} />
          <MemoryMetric label="Rows" value={rowCount(learning)} />
        </div>
        <div className="memory-filters" aria-label="Learning row filters">
          {filterOptions.map((option) => (
            <button
              key={option}
              type="button"
              className={filter === option ? "memory-filter memory-filter--active" : "memory-filter"}
              onClick={() => setFilter(option)}
            >
              {option}
            </button>
          ))}
        </div>
        {actionPreview && (
          <div className="memory-action-preview" role="status">
            <strong>Approval preview</strong>
            <span>{actionPreview}</span>
          </div>
        )}
      </div>
      <section className="panel">
        <div className="panel-heading">
          <h3>Learning signals</h3>
        </div>
        <div className="memory-list">
          {(learning?.signals ?? []).map((signal) => (
            <div key={`${signal.kind}:${signal.path}`} className="memory-row">
              <div>
                <strong>{signal.label}</strong>
                <span>{signal.path}</span>
              </div>
              <StatusPill tone={signal.status === "present" ? "good" : "muted"}>
                {signal.count} {signal.kind}
              </StatusPill>
            </div>
          ))}
        </div>
      </section>
      <section className="panel">
        <div className="panel-heading">
          <h3>Recent sessions</h3>
        </div>
        <div className="memory-list">
          {(learning?.recentSessions ?? []).map((session) => (
            <div key={session} className="memory-row">
              <div>
                <strong>{session}</strong>
                <span>activity ledger available for inspection</span>
              </div>
            </div>
          ))}
        </div>
      </section>
      <LearningRows
        title="Memories"
        rows={learning?.memories}
        filter={filter}
        empty="No memory rows found yet."
      />
      <LearningRows
        title="LearningEvents"
        rows={learning?.learningEvents}
        filter={filter}
        empty="No LearningEvent rows found yet."
      />
      <LearningRows
        title="Behaviour proposals"
        rows={learning?.proposals}
        filter={filter}
        onPreview={previewProposal}
        empty="No pending or stored proposals found yet."
      />
      <LearningRows
        title="Trust deltas"
        rows={learning?.trustDeltas}
        filter={filter}
        empty="No self-trust rows found yet."
      />
    </section>
  );
}

function MemoryMetric({ label, value }: { label: string; value: number }) {
  return (
    <div className="metric-tile">
      <span className="metric-tile__label">{label}</span>
      <strong>{value}</strong>
      <span className="metric-tile__detail">read-only learning surface</span>
    </div>
  );
}

function LearningRows({
  title,
  rows = [],
  filter,
  onPreview,
  empty,
}: {
  title: string;
  rows?: LearningRowPreview[];
  filter: string;
  onPreview?: (row: LearningRowPreview) => void;
  empty: string;
}) {
  const visible = filter === "all" ? rows : rows.filter((row) => row.kind === filter);
  return (
    <section className="panel">
      <div className="panel-heading">
        <h3>{title}</h3>
        <StatusPill>{visible.length} rows</StatusPill>
      </div>
      <div className="memory-list">
        {visible.length === 0 ? (
          <div className="memory-row memory-row--empty">
            <div>
              <strong>{empty}</strong>
              <span>read-only learning surface</span>
            </div>
          </div>
        ) : (
          visible.map((row) => (
            <div key={`${row.kind}:${row.path}:${row.label}`} className="memory-row">
              <div>
                <strong>{row.label}</strong>
                <span>{row.detail}</span>
                <small>{row.path}</small>
              </div>
              <span className="memory-row__actions">
                <StatusPill tone={row.status === "missing" ? "warn" : "muted"}>
                  {row.kind}
                </StatusPill>
                {onPreview && (
                  <button type="button" onClick={() => onPreview(row)}>
                    Preview approval
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

function rowCount(learning?: LearningSummary) {
  if (!learning) {
    return 0;
  }
  return (
    learning.memories.length +
    learning.learningEvents.length +
    learning.proposals.length +
    learning.trustDeltas.length
  );
}
