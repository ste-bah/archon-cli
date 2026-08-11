import { AlertTriangle } from "lucide-react";

interface KbWarningsProps {
  warnings: string[];
}

/**
 * Why the knowledge-base list may be short.
 *
 * An unreadable store and a store with no knowledge bases both render as an
 * empty list, so without this the two are the same screen and a reader has no
 * way to tell "you have none" from "we could not look". It renders inside the
 * `kbs` tab rather than only in the jobs panel, next to the list it qualifies.
 */
export function KbWarnings({ warnings }: KbWarningsProps) {
  if (warnings.length === 0) return null;
  return (
    <div className="ingest-kb-warnings" role="status">
      {warnings.map((warning) => (
        <p key={warning}>
          <AlertTriangle size={14} aria-hidden="true" /> {warning}
        </p>
      ))}
    </div>
  );
}
