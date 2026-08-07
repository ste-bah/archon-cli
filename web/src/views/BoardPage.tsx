import { useQuery } from "@tanstack/react-query";
import { Activity, LayoutGrid } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { apiClient } from "../api/client";
import type { WebBoardItem, WebBoardRun } from "../api/generated/web";
import { StatusPill } from "../components/StatusPill";
import { BoardActivity } from "./board/BoardActivity";
import { BoardColumn } from "./board/BoardColumn";
import { BoardHeader } from "./board/BoardHeader";
import { BoardStats } from "./board/BoardStats";
import { STATUS_ORDER } from "./board/statuses";
import "./BoardPage.css";
// Split by concern rather than by convenience — the file-size guard is a hard
// 500 lines and one board stylesheet had already passed it.
import "./board/board.css";
import "./board/board-card.css";
import "./board/board-activity.css";

export function BoardPage() {
  const [selectedRunId, setSelectedRunId] = useState<string | undefined>();
  const [statusFilter, setStatusFilter] = useState<string[]>([]);
  const [openHistoryFor, setOpenHistoryFor] = useState<string | undefined>();

  // A snapshot, not an event stream: statuses change in place on items that
  // already exist, so there is nothing to append. Polled at the same cadence as
  // the agent panel, slower while nothing is on the board.
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

  // The feed is not filtered with the columns: a status filter is a question
  // about what is on the board now, and narrowing the history to match would
  // hide the transitions that put things where they are.
  const activityQuery = useQuery({
    queryKey: ["board-activity", selectedRun?.runId],
    queryFn: () => apiClient.boardRunActivity(selectedRun!.runId),
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
  // A filter narrows the strip to the columns it names. Left at eight, seven of
  // them would stand empty and the filter would look broken rather than applied.
  const columns = statusFilter.length
    ? STATUS_ORDER.filter((status) => statusFilter.includes(status))
    : STATUS_ORDER;

  return (
    <section className="board-page">
      <BoardHeader
        runs={runs}
        selectedRun={selectedRun}
        onSelectRun={(runId) => {
          setSelectedRunId(runId);
          setOpenHistoryFor(undefined);
        }}
        statusFilter={statusFilter}
        onStatusFilter={(statuses) => {
          setStatusFilter(statuses);
          setOpenHistoryFor(undefined);
        }}
        storeAvailable={storeAvailable}
        refreshing={runsQuery.isFetching || itemsQuery.isFetching || activityQuery.isFetching}
        onRefresh={() => {
          void runsQuery.refetch();
          void itemsQuery.refetch();
          void activityQuery.refetch();
        }}
      />

      <BoardStats run={selectedRun} />

      <section className="panel panel--board">
        <div className="panel-heading">
          <div className="board-section-title">
            <LayoutGrid size={16} aria-hidden="true" />
            <h3>{selectedRun ? selectedRun.runId : "Board"}</h3>
          </div>
          <StatusPill>
            {items.length} {items.length === 1 ? "item" : "items"} shown
          </StatusPill>
        </div>

        {runs.length === 0 || !selectedRun ? (
          <p className="board-blank">
            {emptyRunsMessage(storeAvailable, runsQuery.isLoading, runsQuery.isError)}
          </p>
        ) : items.length === 0 ? (
          <p className="board-blank">
            {emptyItemsMessage(selectedRun, statusFilter, itemsQuery.isLoading, itemsQuery.isError)}
          </p>
        ) : (
          // Eight columns will not fit any window, so the strip scrolls inside
          // itself. The `min-width: 0` on the panel is what keeps that overflow
          // here instead of handing the page a horizontal scrollbar.
          <div className="bstrip" role="list" aria-label="Board columns by status">
            {columns.map((status) => (
              <BoardColumn
                key={status}
                status={status}
                items={grouped.get(status) ?? []}
                openHistoryFor={openHistoryFor}
                onToggleHistory={(id) =>
                  setOpenHistoryFor((current) => (current === id ? undefined : id))
                }
              />
            ))}
          </div>
        )}
      </section>

      <section className="panel panel--board">
        <div className="panel-heading">
          <div className="board-section-title">
            <Activity size={16} aria-hidden="true" />
            <h3>Recent activity</h3>
          </div>
          <StatusPill tone={activityQuery.data?.events.length ? "good" : "muted"}>
            {activityQuery.data?.events.length ?? 0} recorded
          </StatusPill>
        </div>
        <BoardActivity
          activity={activityQuery.data}
          items={items}
          loading={activityQuery.isLoading}
          failed={activityQuery.isError}
          runSelected={Boolean(selectedRun)}
        />
      </section>
    </section>
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
  run: WebBoardRun,
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
  if (statusFilter.length) {
    return `Nothing in ${run.runId} is ${statusFilter.join(" or ")}.`;
  }
  return `${run.runId} has no items.`;
}
