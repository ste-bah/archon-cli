import { Search } from "lucide-react";
import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../api/client";
import { StatusPill } from "../components/StatusPill";
import type { CorpusChunkHit, CorpusSource, CorpusSummary } from "../api/generated/web";
import "./CorpusPage.css";

interface CorpusPageProps {
  corpus?: CorpusSummary;
}

export function CorpusPage({ corpus }: CorpusPageProps) {
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState("");
  const [rootFilter, setRootFilter] = useState<string | undefined>();
  const [selectedPath, setSelectedPath] = useState<string | undefined>();
  const kinds = useMemo(() => uniqueKinds(corpus?.sources ?? []), [corpus]);
  const search = useQuery({
    queryKey: ["corpus-search", query, kind],
    queryFn: () => apiClient.corpusSearch(query, kind),
  });
  const rawResults = search.data?.results ?? corpus?.sources ?? [];
  const results = useMemo(
    () => filterByRoot(rawResults, rootFilter),
    [rawResults, rootFilter],
  );
  const chunks = useMemo(
    () => filterChunksByRoot(search.data?.chunkMatches ?? [], rootFilter),
    [search.data?.chunkMatches, rootFilter],
  );
  const selected = results.find((source) => source.path === selectedPath) ?? results[0];
  const preview = useQuery({
    queryKey: ["corpus-preview", selected?.path],
    queryFn: () => apiClient.corpusSourcePreview(selected!.path),
    enabled: Boolean(selected?.path),
  });

  return (
    <section className="corpus-layout">
      <div className="panel corpus-list-panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Docs and KB</span>
            <h3>Corpus explorer</h3>
          </div>
          <StatusPill tone={corpus?.degraded ? "warn" : "good"}>
            {corpus?.degraded ? "degraded" : "indexed"}
          </StatusPill>
        </div>
        <div className="corpus-controls">
          <label className="corpus-search">
            <Search size={16} aria-hidden="true" />
            <input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search source names, paths, and text previews"
            />
          </label>
          <select value={kind} onChange={(event) => setKind(event.target.value)}>
            <option value="">All types</option>
            {kinds.map((item) => (
              <option key={item} value={item}>
                {item}
              </option>
            ))}
          </select>
        </div>
        <div className="corpus-ranking">
          <span>{search.data?.rankingMode ?? "source listing"}</span>
          <StatusPill>{search.data?.totalMatches ?? results.length} matches</StatusPill>
        </div>
        <div className="corpus-roots">
          {(corpus?.roots ?? []).map((root) => (
            <button
              key={root.path}
              type="button"
              className={
                rootFilter === root.path ? "corpus-root corpus-root--active" : "corpus-root"
              }
              onClick={() => {
                setRootFilter((current) => (current === root.path ? undefined : root.path));
                setSelectedPath(undefined);
              }}
            >
              <strong>{root.label}</strong>
              <span>{root.exists ? `${root.files} files` : "missing"}</span>
            </button>
          ))}
        </div>
        {rootFilter && (
          <div className="corpus-root-filter" role="status">
            <span>{shortPath(rootFilter)}</span>
            <button type="button" onClick={() => setRootFilter(undefined)}>
              Clear
            </button>
          </div>
        )}
        <div className="corpus-results" aria-label="Corpus source results">
          {results.map((source) => (
            <button
              key={source.path}
              type="button"
              className={
                source.path === selected?.path
                  ? "corpus-result corpus-result--active"
                  : "corpus-result"
              }
              onClick={() => setSelectedPath(source.path)}
            >
              <span>
                <strong>{source.label}</strong>
                <small>{source.path}</small>
                {source.excerpt && <em>{source.excerpt}</em>}
              </span>
              <span className="corpus-result__meta">
                <StatusPill>{source.kind}</StatusPill>
                <small>{source.matchKind} · {source.score.toFixed(2)}</small>
              </span>
            </button>
          ))}
        </div>
        <ChunkHits chunks={chunks} onSelect={setSelectedPath} />
      </div>
      <SourcePreview
        source={selected}
        preview={preview.data}
        loading={preview.isLoading}
        resultCount={results.length}
      />
    </section>
  );
}

function ChunkHits({
  chunks,
  onSelect,
}: {
  chunks: CorpusChunkHit[];
  onSelect: (path: string) => void;
}) {
  return (
    <section className="corpus-chunks" aria-label="Ranked corpus chunks">
      <div className="corpus-section-heading">
        <strong>Top chunks</strong>
        <StatusPill>{chunks.length} chunks</StatusPill>
      </div>
      {chunks.length === 0 ? (
        <div className="corpus-chunk corpus-chunk--empty">
          <span>No ranked chunks for this query yet.</span>
        </div>
      ) : (
        chunks.map((chunk) => (
          <button
            key={`${chunk.sourcePath}:${chunk.chunkLabel}:${chunk.lineStart}`}
            type="button"
            className="corpus-chunk"
            onClick={() => onSelect(chunk.sourcePath)}
          >
            <span>
              <strong>{chunk.sourceLabel}</strong>
              <small>
                {chunk.chunkLabel} · line {chunk.lineStart} · {chunk.embeddingStatus}
              </small>
              <em>{chunk.excerpt}</em>
            </span>
            <StatusPill>{chunk.score.toFixed(2)}</StatusPill>
          </button>
        ))
      )}
    </section>
  );
}

function SourcePreview({
  source,
  preview,
  loading,
  resultCount,
}: {
  source?: CorpusSource;
  preview?: {
    content: string;
    lineCount: number;
    truncated: boolean;
    previewAvailable: boolean;
    policyReason: string;
  };
  loading: boolean;
  resultCount: number;
}) {
  return (
    <aside className="panel corpus-preview-panel">
      <div className="panel-heading">
        <div>
          <span className="eyebrow">Source viewer</span>
          <h3>{source?.label ?? "No source selected"}</h3>
        </div>
        <StatusPill tone={preview?.previewAvailable ? "good" : "muted"}>
          {resultCount} results
        </StatusPill>
      </div>
      <div className="corpus-meta">
        <span>{source?.kind ?? "type"}</span>
        <span>{formatBytes(source?.bytes)}</span>
        <span>{preview?.lineCount ?? 0} lines</span>
      </div>
      <p className="summary">{preview?.policyReason ?? "Select a corpus source to inspect it."}</p>
      <pre className="corpus-preview">
        {loading
          ? "Loading preview..."
          : preview?.previewAvailable
            ? preview.content
            : "Preview is not available for this source type yet."}
      </pre>
      {preview?.truncated && <StatusPill tone="warn">preview truncated</StatusPill>}
    </aside>
  );
}

function uniqueKinds(sources: CorpusSource[]) {
  return [...new Set(sources.map((source) => source.kind))].sort();
}

function filterByRoot(sources: CorpusSource[], rootPath?: string) {
  if (!rootPath) {
    return sources;
  }
  return sources.filter((source) => pathMatchesRoot(source.path, rootPath));
}

function filterChunksByRoot(chunks: CorpusChunkHit[], rootPath?: string) {
  if (!rootPath) {
    return chunks;
  }
  return chunks.filter((chunk) => pathMatchesRoot(chunk.sourcePath, rootPath));
}

function pathMatchesRoot(path: string, rootPath: string) {
  const cleanPath = trimTrailingSlash(path);
  const cleanRoot = trimTrailingSlash(rootPath);
  return cleanPath === cleanRoot || cleanPath.startsWith(`${cleanRoot}/`);
}

function trimTrailingSlash(value: string) {
  return value.replace(/\/+$/, "");
}

function shortPath(value: string) {
  const parts = value.split("/").filter(Boolean);
  return parts.slice(-3).join("/") || value;
}

function formatBytes(value?: number) {
  if (value === undefined) {
    return "0 B";
  }
  if (value < 1024) {
    return `${value} B`;
  }
  if (value < 1024 * 1024) {
    return `${Math.round(value / 1024)} KB`;
  }
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}
