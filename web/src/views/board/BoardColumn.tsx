import type { WebBoardItem } from "../../api/generated/web";
import { BoardCard } from "./BoardCard";
import { STATUS_CAPTION, STATUS_ICON, statusLabel } from "./statuses";

// One status, its cards, and nothing else.
//
// The header carries the hue; the body is a plain stack. The column is sized by
// its own contents — `align-self: flex-start` on the strip — so a status holding
// one card stays one card tall instead of stretching to match its busiest
// neighbour, and the body scrolls internally past a point so the tallest column
// cannot push the activity table off the screen.
export function BoardColumn({
  status,
  items,
  openHistoryFor,
  onToggleHistory,
}: {
  status: string;
  items: WebBoardItem[];
  openHistoryFor?: string;
  onToggleHistory: (id: string) => void;
}) {
  const Icon = STATUS_ICON[status];
  return (
    <section className={`bcol bcol--${status}`} aria-label={statusLabel(status)}>
      <header className="bcol__head">
        <span className="bcol__glyph">{Icon && <Icon size={15} aria-hidden="true" />}</span>
        <span className="bcol__name">{statusLabel(status)}</span>
        <span className="bcol__count">{items.length}</span>
      </header>
      {/* The status names are internal vocabulary, so each column says what it
          means once rather than leaving `gaps_remain` to be guessed at. */}
      <p className="bcol__caption">{STATUS_CAPTION[status] ?? " "}</p>
      <div className="bcol__body">
        {items.length === 0 ? (
          <p className="bcol__empty">Nothing here.</p>
        ) : (
          items.map((item) => (
            <BoardCard
              key={item.id}
              item={item}
              historyOpen={openHistoryFor === item.id}
              onToggleHistory={() => onToggleHistory(item.id)}
            />
          ))
        )}
      </div>
    </section>
  );
}
