import {
  CircleCheck,
  CircleDashed,
  CircleSlash,
  Eye,
  Inbox,
  Rocket,
  TriangleAlert,
  UserCheck,
  type LucideIcon,
} from "lucide-react";

// Lifecycle order, not alphabetical: the board is read left to right as work
// moving from raised to closed, and a reader scanning for what is outstanding
// wants the unfinished statuses first.
//
// ALL EIGHT, DELIBERATELY. Merging them into a tidier five would erase the
// distinctions the drain gate is built on: `escalated` is not done, and
// `resolved`, `promoted` and `declined` are three different endings — the last
// of which closes an item on an argument rather than on work.
export const STATUS_ORDER = [
  "open",
  "claimed",
  "in_review",
  "gaps_remain",
  "escalated",
  "resolved",
  "promoted",
  "declined",
] as const;

export type BoardStatusName = (typeof STATUS_ORDER)[number];

// The statuses an item can still move out of. The split drives the colouring:
// unfinished work is cool or amber, the three endings are green, lime and rose,
// so "is anything still running" is answered without reading a single label.
export const UNFINISHED: readonly string[] = STATUS_ORDER.slice(0, 5);
export const ENDINGS: readonly string[] = STATUS_ORDER.slice(5);

export const STATUS_LABEL: Record<string, string> = {
  open: "Open",
  claimed: "Claimed",
  in_review: "In review",
  gaps_remain: "Gaps remain",
  escalated: "Escalated",
  resolved: "Resolved",
  promoted: "Promoted",
  declined: "Declined",
};

// An icon per column, chosen so the header reads before the word does: an
// inbox for unowned work, a held badge for claimed, an eye for review, a broken
// ring for gaps, a warning for escalation, and three visibly different endings.
export const STATUS_ICON: Record<string, LucideIcon> = {
  open: Inbox,
  claimed: UserCheck,
  in_review: Eye,
  gaps_remain: CircleDashed,
  escalated: TriangleAlert,
  resolved: CircleCheck,
  promoted: Rocket,
  declined: CircleSlash,
};

// One line under each column header saying what the status means.
//
// The names are internal vocabulary — nobody arriving at this page knows that
// `gaps_remain` is a review outcome rather than a failure — and a column that
// has to be guessed at is a column that gets ignored.
export const STATUS_CAPTION: Record<string, string> = {
  open: "raised, nobody holding it",
  claimed: "an agent has taken it",
  in_review: "work done, being checked",
  gaps_remain: "reviewed, not yet enough",
  escalated: "handed up, still not done",
  resolved: "closed by doing the work",
  promoted: "carried into the next run",
  declined: "closed on a stated reason",
};

// Filter presets, in the order the control offers them.
//
// The lifecycle groupings come first because they are the two questions
// actually asked of a board — what is still running, and how did things end —
// and answering either by ticking five checkboxes is how a filter goes unused.
export const FILTER_PRESETS: { id: string; label: string; statuses: readonly string[] }[] = [
  { id: "all", label: "All statuses", statuses: [] },
  { id: "unfinished", label: "Still open (5 statuses)", statuses: UNFINISHED },
  { id: "endings", label: "Closed out (3 statuses)", statuses: ENDINGS },
  ...STATUS_ORDER.map((status) => ({
    id: status,
    label: statusLabel(status),
    statuses: [status],
  })),
];

export function presetIdFor(statuses: string[]): string {
  if (statuses.length === 0) {
    return "all";
  }
  const match = FILTER_PRESETS.find(
    (preset) =>
      preset.statuses.length === statuses.length &&
      preset.statuses.every((status) => statuses.includes(status)),
  );
  return match?.id ?? "all";
}

export function statusLabel(status: string): string {
  return STATUS_LABEL[status] ?? status.replace(/_/g, " ");
}

// The `StatusPill` tone for a status, kept in step with the column colours.
export function statusTone(status: string): "good" | "warn" | "muted" {
  if (status === "resolved" || status === "promoted") {
    return "good";
  }
  if (status === "gaps_remain" || status === "escalated" || status === "declined") {
    return "warn";
  }
  return "muted";
}

// "15 min ago" from an RFC 3339 stamp.
//
// Relative rather than absolute because the question a board answers is "has
// anything moved recently", and a wall-clock timestamp makes the reader do the
// subtraction. Past about a month the relative form stops carrying meaning and
// the date itself is more useful, so it hands back over.
export function relativeTime(iso: string, now = Date.now()): string {
  const then = Date.parse(iso);
  if (Number.isNaN(then)) {
    return iso;
  }
  const seconds = Math.round((now - then) / 1000);
  // Clock skew between the writing process and this browser can put a stamp
  // slightly in the future; "in 3 seconds" would read as a bug rather than as
  // the rounding it is.
  if (seconds < 45) {
    return "just now";
  }
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) {
    return `${minutes} min ago`;
  }
  const hours = Math.round(minutes / 60);
  if (hours < 24) {
    return `${hours} hr ago`;
  }
  const days = Math.round(hours / 24);
  if (days <= 30) {
    return `${days} ${days === 1 ? "day" : "days"} ago`;
  }
  return new Date(then).toLocaleDateString();
}

// The leading segment of an id, for cross-referencing a card against a row in
// the activity table. Full UUIDs do not fit on a card and are not read whole.
export function shortId(id: string): string {
  return id.length > 8 ? id.slice(0, 8) : id;
}
