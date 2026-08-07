import {
  CircleCheck,
  CircleSlash,
  Eye,
  Inbox,
  Layers,
  UserCheck,
  type LucideIcon,
} from "lucide-react";
import type { WebBoardRun } from "../../api/generated/web";

// The stat strip above the board.
//
// Every number here is a per-status count the runs endpoint already returns, so
// the strip is a projection of the same fact the columns show rather than a
// second source that can disagree with them. Scoped to the selected run for the
// same reason the columns are: the board is partitioned by `run_id`, and a total
// mixing runs would answer a question nobody on this page asked.
//
// Six tiles and not eight: this is the shape of the run at a glance, and the
// columns below are where every status is accounted for. `gaps_remain`,
// `escalated` and `promoted` are the rarest and are the ones a tile would spend
// its width reading zero.
interface StatSpec {
  key: string;
  label: string;
  icon: LucideIcon;
  caption: string;
  value: (run: WebBoardRun | undefined) => number;
}

const STATS: StatSpec[] = [
  {
    key: "total",
    label: "Total items",
    icon: Layers,
    caption: "raised in this run",
    value: (run) => run?.total ?? 0,
  },
  {
    key: "open",
    label: "Open",
    icon: Inbox,
    caption: "nobody has claimed",
    value: (run) => countOf(run, "open"),
  },
  {
    key: "claimed",
    label: "Claimed",
    icon: UserCheck,
    caption: "an agent is holding",
    value: (run) => countOf(run, "claimed"),
  },
  {
    key: "in_review",
    label: "In review",
    icon: Eye,
    caption: "waiting on a check",
    value: (run) => countOf(run, "in_review"),
  },
  {
    key: "resolved",
    label: "Resolved",
    icon: CircleCheck,
    caption: "closed by doing the work",
    value: (run) => countOf(run, "resolved"),
  },
  {
    key: "declined",
    label: "Declined",
    icon: CircleSlash,
    caption: "closed on an argument",
    value: (run) => countOf(run, "declined"),
  },
];

export function BoardStats({ run }: { run?: WebBoardRun }) {
  return (
    <div className="bstats">
      {STATS.map((stat) => {
        const Icon = stat.icon;
        return (
          <section key={stat.key} className={`bstat bstat--${stat.key}`} aria-label={stat.label}>
            <span className="bstat__icon">
              <Icon size={19} aria-hidden="true" />
            </span>
            <div className="bstat__body">
              <span className="bstat__label">{stat.label}</span>
              <strong className="bstat__value">{stat.value(run)}</strong>
              <span className="bstat__caption">{stat.caption}</span>
            </div>
          </section>
        );
      })}
    </div>
  );
}

// A status with no items is absent from `counts` rather than present as zero,
// so a missing key is the answer rather than a hole.
function countOf(run: WebBoardRun | undefined, status: string): number {
  return run?.counts.find((count) => count.status === status)?.count ?? 0;
}
