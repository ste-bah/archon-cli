import { ChevronLeft, ChevronRight } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { RefObject } from "react";
import type { PDFDocumentLoadingTask, PDFDocumentProxy } from "pdfjs-dist";
import { apiClient } from "../../api/client";
import { StatusPill } from "../../components/StatusPill";
import { loadPdfDocument, renderPdfPage } from "./pdfViewer";

const SCALES = [0.75, 1, 1.5, 2] as const;

type DocumentState =
  | { status: "loading" }
  | { status: "ready"; document: PDFDocumentProxy }
  | { status: "error"; message: string };

/**
 * Page-by-page PDF viewer for a corpus source.
 *
 * One page is rasterised at a time rather than the whole document: a corpus
 * PDF can be hundreds of pages, and a canvas per page would cost more memory
 * than the document itself.
 */
export function PdfSourceView({ path }: { path: string }) {
  const state = usePdfDocument(path);
  const [page, setPage] = useState(1);
  const [scale, setScale] = useState<number>(1);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const pageCount = state.status === "ready" ? state.document.numPages : 0;

  useEffect(() => {
    setPage(1);
  }, [path]);

  const renderError = usePageRender(
    state.status === "ready" ? state.document : undefined,
    canvasRef,
    page,
    scale,
  );

  if (state.status === "error") {
    return <div className="corpus-preview corpus-pdf-message">{state.message}</div>;
  }

  return (
    <div className="corpus-pdf">
      <div className="corpus-pdf-controls">
        <button
          type="button"
          aria-label="Previous page"
          disabled={page <= 1}
          onClick={() => setPage((current) => Math.max(1, current - 1))}
        >
          <ChevronLeft size={16} aria-hidden="true" />
        </button>
        <StatusPill>
          {state.status === "loading" ? "loading" : `page ${page} of ${pageCount}`}
        </StatusPill>
        <button
          type="button"
          aria-label="Next page"
          disabled={pageCount === 0 || page >= pageCount}
          onClick={() => setPage((current) => Math.min(pageCount, current + 1))}
        >
          <ChevronRight size={16} aria-hidden="true" />
        </button>
        <select
          aria-label="Zoom"
          value={String(scale)}
          onChange={(event) => setScale(Number(event.target.value))}
        >
          {SCALES.map((value) => (
            <option key={value} value={String(value)}>
              {Math.round(value * 100)}%
            </option>
          ))}
        </select>
      </div>
      <div className="corpus-preview corpus-pdf-page">
        {renderError ? (
          <span className="corpus-pdf-message">{renderError}</span>
        ) : (
          <canvas ref={canvasRef} aria-label={`PDF page ${page}`} />
        )}
      </div>
    </div>
  );
}

/**
 * Fetch the bytes and open the document.
 *
 * The loading task, not the document proxy, is what owns the worker, so it is
 * the thing that has to be destroyed when `path` changes — otherwise every
 * source click leaks a worker.
 */
function usePdfDocument(path: string): DocumentState {
  const [state, setState] = useState<DocumentState>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;
    let task: PDFDocumentLoadingTask | undefined;
    setState({ status: "loading" });

    apiClient
      .corpusSourceBytes(path)
      .then((bytes) => {
        if (cancelled) {
          return undefined;
        }
        task = loadPdfDocument(bytes);
        return task.promise;
      })
      .then((document) => {
        if (document && !cancelled) {
          setState({ status: "ready", document });
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          setState({ status: "error", message: describe(error) });
        }
      });

    return () => {
      cancelled = true;
      void task?.destroy();
    };
  }, [path]);

  return state;
}

/** Draw the current page, cancelling an in-flight render on every change. */
function usePageRender(
  document: PDFDocumentProxy | undefined,
  canvasRef: RefObject<HTMLCanvasElement | null>,
  page: number,
  scale: number,
): string | undefined {
  const [error, setError] = useState<string>();

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!document || !canvas) {
      return;
    }
    let cancelled = false;
    setError(undefined);

    const task = document.getPage(page).then((loaded) => {
      if (cancelled) {
        return undefined;
      }
      const render = renderPdfPage(loaded, canvas, scale);
      return render.promise.then(
        () => render,
        (reason: unknown) => {
          // A cancelled render is the expected outcome of paging quickly.
          if (!cancelled) {
            setError(describe(reason));
          }
          return render;
        },
      );
    });

    return () => {
      cancelled = true;
      void task.then((render) => render?.cancel()).catch(() => undefined);
    };
  }, [document, canvasRef, page, scale]);

  return error;
}

function describe(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
