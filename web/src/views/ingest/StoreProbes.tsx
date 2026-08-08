import { HardDrive } from "lucide-react";
import { StatusPill } from "../../components/StatusPill";
import type { PathProbe } from "../../api/generated/web";

/**
 * Where the stores live on disk, and whether they exist yet.
 *
 * `/api/ingest/summary` has always carried these four probes — the document
 * store, `.archon/docs`, `.archon/kb` and the video artifact directory — and
 * nothing rendered them. On a fresh project that is exactly the question a
 * reader has: not "how many documents" (zero) but "where would they go, and has
 * anything been created?"
 */
export function StoreProbes({ stores }: { stores: PathProbe[] }) {
  const present = stores.filter((store) => store.exists).length;
  return (
    <div className="panel ingest-stores">
      <div className="panel-heading">
        <div>
          <span className="eyebrow">On disk</span>
          <h3>Store locations</h3>
        </div>
        <StatusPill tone={present > 0 ? "good" : "muted"}>
          {present} of {stores.length} present
        </StatusPill>
      </div>
      <div className="ingest-store-list">
        {stores.map((store) => (
          <div key={store.path} className="ingest-store-row">
            <HardDrive size={15} aria-hidden="true" />
            <div>
              <strong>{store.label}</strong>
              <small>{store.path}</small>
            </div>
            <span>
              {store.exists ? `${store.files} file(s) · ${formatBytes(store.bytes)}` : "not created yet"}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

function formatBytes(value: number) {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${Math.round(value / 1024)} KB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}
