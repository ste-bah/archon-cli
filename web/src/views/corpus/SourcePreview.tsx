import { lazy, Suspense } from "react";
import { StatusPill } from "../../components/StatusPill";
import type { CorpusSource, CorpusSourcePreview } from "../../api/generated/web";

// PDF.js and its worker are the largest thing in the bundle by a wide margin,
// and most corpus sources are Markdown. Loading it only when a PDF is selected
// keeps the workbench's first paint where it is today.
const PdfSourceView = lazy(() =>
  import("./PdfSourceView").then((module) => ({ default: module.PdfSourceView })),
);

export function SourcePreview({
  source,
  preview,
  loading,
  resultCount,
}: {
  source?: CorpusSource;
  preview?: CorpusSourcePreview;
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
      <PreviewBody source={source} preview={preview} loading={loading} />
      {preview?.truncated && <StatusPill tone="warn">preview truncated</StatusPill>}
    </aside>
  );
}

function PreviewBody({
  source,
  preview,
  loading,
}: {
  source?: CorpusSource;
  preview?: CorpusSourcePreview;
  loading: boolean;
}) {
  if (loading || !preview) {
    return <pre className="corpus-preview">Loading preview...</pre>;
  }
  if (preview.previewMode === "pdf" && source) {
    return (
      <Suspense fallback={<pre className="corpus-preview">Loading PDF viewer...</pre>}>
        <PdfSourceView path={source.path} />
      </Suspense>
    );
  }
  if (preview.previewMode === "text") {
    return <pre className="corpus-preview">{preview.content}</pre>;
  }
  return <pre className="corpus-preview">{preview.policyReason}</pre>;
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
