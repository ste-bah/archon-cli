import { ArrowRight } from "lucide-react";
import type { WebBoardActivity, WebBoardItem } from "../../api/generated/web";
import { relativeTime, shortId, statusLabel } from "./statuses";

// The run's transitions, newest first.
//
// Run-scoped rather than assembled from per-item histories: `/api/board/runs/
// {run_id}/activity` reads through the `board_item_events:by_run` index in one
// query, and the alternative is a request per card on every poll.
//
// Only decisions appear here. Claims and releases are ownership churn and the
// store deliberately does not record them, so an item taken and swept by a lease
// sweeper leaves no row — which is why a busy board can still have a short feed.
export function BoardActivity({
  activity,
  items,
  loading,
  failed,
  runSelected,
}: {
  activity?: WebBoardActivity;
  items: WebBoardItem[];
  loading: boolean;
  failed: boolean;
  runSelected: boolean;
}) {
  const events = activity?.events ?? [];
  // Titles come from the items already on screen. A status filter can hide the
  // item a row refers to, and the short id it falls back to is the same prefix
  // printed on every card, so the row is still traceable rather than anonymous.
  const titles = new Map(items.map((item) => [item.id, item.title]));

  if (events.length === 0) {
    return (
      <p className="bactivity__empty">
        {emptyActivityMessage(activity, loading, failed, runSelected)}
      </p>
    );
  }

  return (
    <>
      <div className="bactivity">
        <table>
          <thead>
            <tr>
              <th scope="col">Item</th>
              <th scope="col">Actor</th>
              <th scope="col">Transition</th>
              <th scope="col">Round</th>
              <th scope="col">Note</th>
              <th scope="col">When</th>
            </tr>
          </thead>
          <tbody>
            {events.map((event) => (
              <tr key={`${event.itemId}-${event.seq}`}>
                <td>
                  <span className="bactivity__item">
                    {titles.get(event.itemId) ?? shortId(event.itemId)}
                  </span>
                  <small>{shortId(event.itemId)}</small>
                </td>
                <td>{event.actor ?? <span className="bactivity__none">no actor</span>}</td>
                <td>
                  <span className="bactivity__move">
                    <span className={`bchip bchip--${event.fromStatus}`}>
                      {statusLabel(event.fromStatus)}
                    </span>
                    <ArrowRight size={12} aria-hidden="true" />
                    <span className={`bchip bchip--${event.toStatus}`}>
                      {statusLabel(event.toStatus)}
                    </span>
                  </span>
                </td>
                <td>{event.round}</td>
                <td className="bactivity__note">
                  {event.note || <span className="bactivity__none">—</span>}
                </td>
                <td className="bactivity__when">{relativeTime(event.at)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {/* A feed that has been cut has to say so, or the oldest row on screen
          reads as the beginning of the run. */}
      {activity?.truncated && (
        <p className="bactivity__truncated">
          Showing the newest {activity.limit} transitions. Older ones are on the
          record but not read here.
        </p>
      )}
    </>
  );
}

// A store that is not there, a run that has not moved, and a read that failed
// are three different facts, and only two of them are worth acting on.
function emptyActivityMessage(
  activity: WebBoardActivity | undefined,
  loading: boolean,
  failed: boolean,
  runSelected: boolean,
): string {
  if (failed) {
    return "The run's activity could not be read.";
  }
  if (loading) {
    return "Reading the run's transitions.";
  }
  if (!runSelected) {
    return "Select a run to see what has happened in it.";
  }
  if (activity && !activity.storeAvailable) {
    return "No memory database yet, so there is nothing to have happened.";
  }
  return "Nothing has moved in this run yet. Only decisions are recorded — claims and releases are not.";
}
