import { ChevronDown, RefreshCw } from "lucide-react";
import type { WebBoardRun } from "../../api/generated/web";
import { StatusPill } from "../../components/StatusPill";
import { FILTER_PRESETS, presetIdFor, relativeTime } from "./statuses";

// Title, run selector, status filter, manual refresh.
//
// The run selector is not decoration: the board is partitioned by `run_id`, and
// every read below this row is scoped to whatever it holds. It carries each
// run's item count and how long ago it moved, because "which run" is otherwise
// a choice between opaque identifiers.
//
// The filter narrows which COLUMNS are shown rather than which cards — on a
// kanban board the statuses are the columns, so a per-status filter that left
// eight empty columns standing would be filtering nothing a reader can see.
export function BoardHeader({
  runs,
  selectedRun,
  onSelectRun,
  statusFilter,
  onStatusFilter,
  storeAvailable,
  refreshing,
  onRefresh,
}: {
  runs: WebBoardRun[];
  selectedRun?: WebBoardRun;
  onSelectRun: (runId: string) => void;
  statusFilter: string[];
  onStatusFilter: (statuses: string[]) => void;
  storeAvailable: boolean;
  refreshing: boolean;
  onRefresh: () => void;
}) {
  return (
    <header className="bhead">
      <div className="bhead__title">
        <span className="eyebrow">Agent handoffs</span>
        <h3>Task Board</h3>
        <p>
          Raised, claimed and closed by agents. This view is read-only — every
          board endpoint is a <code>GET</code>.
        </p>
      </div>

      <div className="bhead__controls">
        <label className="bfield">
          <span>Run</span>
          <div className="bselect">
            <select
              value={selectedRun?.runId ?? ""}
              disabled={runs.length === 0}
              onChange={(event) => onSelectRun(event.target.value)}
            >
              {runs.length === 0 && <option value="">no runs yet</option>}
              {runs.map((run) => (
                <option key={run.runId} value={run.runId}>
                  {run.runId} — {run.total} {run.total === 1 ? "item" : "items"} ·{" "}
                  {relativeTime(run.lastUpdatedAt)}
                </option>
              ))}
            </select>
            <ChevronDown size={14} aria-hidden="true" />
          </div>
        </label>

        <label className="bfield">
          <span>Showing</span>
          <div className="bselect">
            <select
              value={presetIdFor(statusFilter)}
              onChange={(event) => {
                const preset = FILTER_PRESETS.find((entry) => entry.id === event.target.value);
                onStatusFilter([...(preset?.statuses ?? [])]);
              }}
            >
              {FILTER_PRESETS.map((preset) => (
                <option key={preset.id} value={preset.id}>
                  {preset.label}
                </option>
              ))}
            </select>
            <ChevronDown size={14} aria-hidden="true" />
          </div>
        </label>

        <div className="bhead__actions">
          <StatusPill tone={storeAvailable ? "good" : "warn"}>
            {storeAvailable ? `${runs.length} ${runs.length === 1 ? "run" : "runs"}` : "no memory database"}
          </StatusPill>
          {/* The page polls, so this is not the only way the board updates —
              it is the way to stop waiting for the next tick. */}
          <button
            className="bhead__refresh"
            type="button"
            onClick={onRefresh}
            aria-label="Refresh the board now"
          >
            <RefreshCw size={15} className={refreshing ? "is-spinning" : undefined} />
          </button>
        </div>
      </div>
    </header>
  );
}
