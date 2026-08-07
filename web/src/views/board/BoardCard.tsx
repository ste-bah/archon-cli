import { useQuery } from "@tanstack/react-query";
import { Clock, History, RotateCw } from "lucide-react";
import { apiClient } from "../../api/client";
import type { WebBoardItem } from "../../api/generated/web";
import { relativeTime, shortId, statusLabel } from "./statuses";

// One item on the board.
//
// EVERY FIELD HERE IS A REAL COLUMN. The obvious kanban card carries a priority
// badge, a progress bar and a due date, and this board has none of the three —
// no `priority`, no percentage, no deadline. Inventing them would make the card
// look right and read false, so the three slots are filled by the fields that
// actually exist and actually matter:
//
//   priority  -> `kind`. issue vs note is the real distinction: an issue is work
//                that must happen and the drain gate counts it, a note is
//                context for whoever next touches the area.
//   progress  -> `round`. The attempt counter. "round 2" says this came back,
//                which is the thing a progress bar was being asked to imply.
//   due date  -> `updated_at`, relative. When it last moved, not when it is due.
//
// No action buttons either: all three board endpoints are `GET`. The board is
// written by agents, and a button that cannot do anything is worse than none.
export function BoardCard({
  item,
  historyOpen,
  onToggleHistory,
}: {
  item: WebBoardItem;
  historyOpen: boolean;
  onToggleHistory: () => void;
}) {
  const owner = item.claimedBy ?? item.raisedBy;
  return (
    <article className={`bcard bcard--${item.kind}`}>
      <div className="bcard__top">
        <span className={`bchip bchip--${item.kind}`}>{item.kind}</span>
        {/* Round 0 is the first attempt and says nothing; from round 1 the
            number is the fact that this item came back, which is worth a badge
            of its own rather than a line of body text. */}
        {item.round > 0 && (
          <span className="bchip bchip--round">
            <RotateCw size={11} aria-hidden="true" />
            round {item.round}
          </span>
        )}
        <button
          className="bcard__history-toggle"
          type="button"
          aria-expanded={historyOpen}
          aria-label={`Transitions for ${item.title}`}
          onClick={onToggleHistory}
        >
          <History size={14} aria-hidden="true" />
        </button>
      </div>

      <h4 className="bcard__title">{item.title}</h4>
      <p className="bcard__owner">
        {item.claimedBy ? "held by" : "raised by"} {owner}
      </p>

      <dl className="bcard__fields">
        <div>
          <dt>Evidence</dt>
          <dd>{item.evidence}</dd>
        </div>
        <div>
          <dt>Acceptance</dt>
          <dd>{item.acceptance}</dd>
        </div>
      </dl>

      {/* Only a declined item has one, and the store refuses to record a decline
          without it, so its absence here would be a real loss. */}
      {item.declineReason && (
        <p className="bcard__decline">
          <strong>Declined</strong> {item.declineReason}
        </p>
      )}

      <footer className="bcard__foot">
        <span className="bcard__when">
          <Clock size={12} aria-hidden="true" />
          {relativeTime(item.updatedAt)}
        </span>
        {/* The same prefix the activity table shows, so a row down there can be
            matched back to a card up here without opening anything. */}
        <span className="bcard__id">{shortId(item.id)}</span>
      </footer>

      {historyOpen && <CardHistory itemId={item.id} />}
    </article>
  );
}

// One item's ladder, oldest first — the opposite order to the run-wide feed
// below the board, which is read from the top like any other feed.
function CardHistory({ itemId }: { itemId: string }) {
  const history = useQuery({
    queryKey: ["board-history", itemId],
    queryFn: () => apiClient.boardItemHistory(itemId),
  });
  if (history.isLoading) {
    return <div className="bcard__history bcard__history--empty">Loading transitions.</div>;
  }
  const events = history.data?.events ?? [];
  if (events.length === 0) {
    return (
      <div className="bcard__history bcard__history--empty">
        Nothing recorded yet. Claims and releases are ownership churn and are
        deliberately not part of an item&apos;s history.
      </div>
    );
  }
  return (
    <ol className="bcard__history">
      {events.map((event) => (
        <li key={event.seq}>
          <span>
            {statusLabel(event.fromStatus)} → {statusLabel(event.toStatus)}
          </span>
          <small>
            {event.actor ?? "no actor"} · {relativeTime(event.at)}
          </small>
          {event.note && <small className="bcard__history-note">{event.note}</small>}
        </li>
      ))}
    </ol>
  );
}
