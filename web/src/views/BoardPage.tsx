import { useQuery } from "@tanstack/react-query";
import { ClipboardList, FileText, History, StickyNote, TriangleAlert } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { apiClient } from "../api/client";
import type { WebBoardEvent, WebBoardItem, WebBoardRun } from "../api/generated/web";
import { StatusPill } from "../components/StatusPill";
import "./BoardPage.css";

// Lifecycle order, not alphabetical: the board is read top to bottom as work
// moving from raised to closed, and a reader scanning for what is outstanding
// wants the unfinished statuses first.
const STATUS_ORDER = [
  "open",
  "claimed",
  "in_review",
  "gaps_remain",
  "escalated",
  "resolved",
  "promoted",
  "declined",
];

export function BoardPage() {
  const [selectedRunId, setSelectedRunId] = useState<string | undefined>();
  const [statusFilter, setStatusFilter] = useState<string[]>([]);
  const [openHistoryFor, setOpenHistoryFor] = useState<string | undefined>();

  // A snapshot, not an event stream: statuses change in place on items that
  // already exist, so there is nothing to append. Polled at the same cadence
  // as the agent panel, slower while nothing is on the board.
  const runsQuery = useQuery({
    queryKey: ["board-runs"],
    queryFn: apiClient.boardRuns,
    refetchInterval: (query) => (query.state.data?.runs.length ? 5000 : 10000),
  });
  const runs = useMemo(() => runsQuery.data?.runs ?? [], [runsQuery.data]);
  const selectedRun = runs.find((run) => run.runId === selectedRunId) ?? runs[0];

  const itemsQuery = useQuery({
    queryKey: ["board-items", selectedRun?.runId, statusFilter],
    queryFn: () => apiClient.boardItems(selectedRun!.runId, statusFilter),
    enabled: Boolean(selectedRun?.runId),
    refetchInterval: 5000,
  });

  useEffect(() => {
    if (!selectedRunId && runs[0]?.runId) {
      setSelectedRunId(runs[0].runId);
    }
  }, [runs, selectedRunId]);

  const items = itemsQuery.data?.items ?? [];
  const grouped = groupByStatus(items);
  const storeAvailable = runsQuery.data?.storeAvailable ?? true;
  const totals = runs.reduce(
    (sum, run) => ({
      items: sum.items + run.total,
      open: sum.open + countOf(run, "open"),
      declined: sum.declined + countOf(run, "declined"),
    }),
    { items: 0, open: 0, declined: 0 },
  );

  return (
    <section className="board-layout">
      <div className="panel panel--wide">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Agent handoffs</span>
            <h3>Task board</h3>
          </div>
          <StatusPill tone={storeAvailable ? "good" : "warn"}>
            {storeAvailable ? `${runs.length} runs` : "no memory database"}
          </StatusPill>
        </div>
        <div className="board-metrics">
          <BoardMetric icon={<ClipboardList size={18} />} label="Runs" value={runs.length} detail="with items raised" />
          <BoardMetric icon={<FileText size={18} />} label="Items" value={totals.items} detail="across every run" />
          <BoardMetric icon={<StickyNote size={18} />} label="Open" value={totals.open} detail="nobody has claimed" />
          <BoardMetric icon={<TriangleAlert size={18} />} label="Declined" value={totals.declined} detail="closed on an argument" />
        </div>
      </div>

      <section className="panel">
        <div className="panel-heading">
          <h3>Runs</h3>
          <StatusPill>{runs.length} tracked</StatusPill>
        </div>
        <div className="board-list">
          {runs.length === 0 ? (
            <EmptyRow>{emptyRunsMessage(storeAvailable, runsQuery.isLoading, runsQuery.isError)}</EmptyRow>
          ) : (
            runs.map((run) => (
              <button
                key={run.runId}
                className={`board-row board-row--selectable${
                  run.runId === selectedRun?.runId ? " board-row--active" : ""
                }`}
                onClick={() => {
                  setSelectedRunId(run.runId);
                  setStatusFilter([]);
                  setOpenHistoryFor(undefined);
                }}
                type="button"
              >
                <div>
                  <strong>{run.runId}</strong>
                  <span>{run.total} items · updated {run.lastUpdatedAt}</span>
                  <small className="board-counts">
                    {orderedCounts(run).map((count) => (
                      <span key={count.status} className="board-count">
                        {count.status.replace("_", " ")} {count.count}
                      </span>
                    ))}
                  </small>
                </div>
              </button>
            ))
          )}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h3>Status filter</h3>
          <StatusPill tone={statusFilter.length ? "good" : "muted"}>
            {statusFilter.length ? `${statusFilter.length} selected` : "all statuses"}
          </StatusPill>
        </div>
        <div className="board-filters" aria-label="Board status filters">
          <button
            className={`board-filter${statusFilter.length === 0 ? " board-filter--active" : ""}`}
            onClick={() => setStatusFilter([])}
            type="button"
          >
            all
          </button>
          {STATUS_ORDER.map((status) => (
            <button
              key={status}
              className={`board-filter${statusFilter.includes(status) ? " board-filter--active" : ""}`}
              onClick={() => setStatusFilter(toggle(statusFilter, status))}
              type="button"
            >
              {status.replace("_", " ")}
              {selectedRun ? ` ${countOf(selectedRun, status)}` : ""}
            </button>
          ))}
        </div>
      </section>

      <section className="panel panel--wide">
        <div className="panel-heading">
          <h3>{selectedRun ? `Items in ${selectedRun.runId}` : "Board items"}</h3>
          <StatusPill>{items.length} shown</StatusPill>
        </div>
        {items.length === 0 ? (
          <EmptyRow>{emptyItemsMessage(selectedRun, statusFilter, itemsQuery.isLoading, itemsQuery.isError)}</EmptyRow>
        ) : (
          STATUS_ORDER.filter((status) => grouped.has(status)).map((status) => (
            <div key={status} className="board-group">
              <div className="board-group__heading">
                <StatusPill tone={statusTone(status)}>{status.replace("_", " ")}</StatusPill>
                <span>{grouped.get(status)!.length} items</span>
              </div>
              <div className="board-list">
                {grouped.get(status)!.map((item) => (
                  <BoardItemCard
                    key={item.id}
                    item={item}
                    historyOpen={openHistoryFor === item.id}
                    onToggleHistory={() =>
                      setOpenHistoryFor(openHistoryFor === item.id ? undefined : item.id)
                    }
                  />
                ))}
              </div>
            </div>
          ))
        )}
      </section>
    </section>
  );
}

function BoardItemCard({
  item,
  historyOpen,
  onToggleHistory,
}: {
  item: WebBoardItem;
  historyOpen: boolean;
  onToggleHistory: () => void;
}) {
  return (
    <article className="board-item">
      <header>
        <div>
          <strong>{item.title}</strong>
          <span>
            {item.kind} · raised by {item.raisedBy} · round {item.round}
          </span>
        </div>
        <div className="board-item__actions">
          <StatusPill tone={statusTone(item.status)}>{item.status.replace("_", " ")}</StatusPill>
          <button type="button" onClick={onToggleHistory} aria-label={`History for ${item.title}`}>
            <History size={15} />
          </button>
        </div>
      </header>
      <dl className="board-item__fields">
        <BoardField label="Evidence" value={item.evidence} />
        <BoardField label="Acceptance" value={item.acceptance} />
        <BoardField label="Claimed by" value={item.claimedBy ?? "unclaimed"} />
        <BoardField label="Updated" value={item.updatedAt} />
        {/* Only a declined item has one, and the store refuses to record a
            decline without it, so its absence here would be a real loss. */}
        {item.declineReason && <BoardField label="Declined because" value={item.declineReason} />}
      </dl>
      {historyOpen && <ItemHistory itemId={item.id} />}
    </article>
  );
}

function ItemHistory({ itemId }: { itemId: string }) {
  const history = useQuery({
    queryKey: ["board-history", itemId],
    queryFn: () => apiClient.boardItemHistory(itemId),
  });
  const events = history.data?.events ?? [];
  if (history.isLoading) {
    return <div className="board-history">Loading transitions.</div>;
  }
  if (events.length === 0) {
    return (
      <div className="board-history">
        No transitions recorded. Claims and releases are ownership churn and are
        deliberately not part of an item's history.
      </div>
    );
  }
  return (
    <ol className="board-history">
      {events.map((event) => (
        <li key={event.seq}>
          <TransitionRow event={event} />
        </li>
      ))}
    </ol>
  );
}

function TransitionRow({ event }: { event: WebBoardEvent }) {
  return (
    <div className="board-transition">
      <span>
        {event.fromStatus.replace("_", " ")} → {event.toStatus.replace("_", " ")}
      </span>
      <span>
        round {event.round} · {event.actor ?? "no actor"} · {event.at}
      </span>
      {event.note && <small>{event.note}</small>}
    </div>
  );
}

function BoardField({ label, value }: { label: string; value: string }) {
  return (
    <div className="board-field">
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  );
}

function BoardMetric({
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
    <section className="board-metric" aria-label={label}>
      <span className="board-metric__icon">{icon}</span>
      <span className="metric-tile__label">{label}</span>
      <strong>{value}</strong>
      <span className="metric-tile__detail">{detail}</span>
    </section>
  );
}

function EmptyRow({ children }: { children: React.ReactNode }) {
  return (
    <div className="board-empty">
      <FileText size={18} aria-hidden="true" />
      <span>{children}</span>
    </div>
  );
}

function groupByStatus(items: WebBoardItem[]) {
  const grouped = new Map<string, WebBoardItem[]>();
  for (const item of items) {
    const bucket = grouped.get(item.status);
    if (bucket) {
      bucket.push(item);
    } else {
      grouped.set(item.status, [item]);
    }
  }
  return grouped;
}

function orderedCounts(run: WebBoardRun) {
  return [...run.counts].sort(
    (left, right) => statusRank(left.status) - statusRank(right.status),
  );
}

function statusRank(status: string) {
  const index = STATUS_ORDER.indexOf(status);
  return index === -1 ? STATUS_ORDER.length : index;
}

function countOf(run: WebBoardRun, status: string) {
  return run.counts.find((count) => count.status === status)?.count ?? 0;
}

function toggle(selected: string[], status: string) {
  return selected.includes(status)
    ? selected.filter((entry) => entry !== status)
    : [...selected, status];
}

// An unavailable store and an empty board are different facts, and only the
// first is worth acting on, so they do not share a message.
function emptyRunsMessage(storeAvailable: boolean, loading: boolean, failed: boolean) {
  if (failed) {
    return "The board could not be read. Check that the memory database is reachable.";
  }
  if (loading) {
    return "Reading the board.";
  }
  if (!storeAvailable) {
    return "No memory database yet. The board appears once a session has run and raised something.";
  }
  return "No run has raised a board item yet.";
}

function emptyItemsMessage(
  run: WebBoardRun | undefined,
  statusFilter: string[],
  loading: boolean,
  failed: boolean,
) {
  if (failed) {
    return "The run's items could not be read.";
  }
  if (loading) {
    return "Loading items.";
  }
  if (!run) {
    return "Select a run to see its items.";
  }
  if (statusFilter.length) {
    return `Nothing in ${run.runId} is ${statusFilter.join(" or ")}.`;
  }
  return `${run.runId} has no items.`;
}

function statusTone(status: string): "good" | "warn" | "muted" {
  if (["resolved", "promoted"].includes(status)) {
    return "good";
  }
  if (["gaps_remain", "escalated", "declined"].includes(status)) {
    return "warn";
  }
  return "muted";
}
