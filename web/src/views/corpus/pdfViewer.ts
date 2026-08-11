import { AnnotationMode, GlobalWorkerOptions, getDocument } from "pdfjs-dist";
import type {
  PDFDocumentLoadingTask,
  PDFPageProxy,
  RenderTask,
} from "pdfjs-dist";
// `?url` makes Vite emit the worker as a hashed asset of this bundle, so it is
// served from our own origin by the same handler that serves the app. There is
// no CDN in this path and `worker-src 'self'` in the shell CSP is what stops
// one being reintroduced.
import pdfWorkerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";

/**
 * Root for the PDF.js runtime data (CMaps, standard fonts, WASM image
 * decoders, ICC profiles). `vite.config.ts` copies these out of `node_modules`
 * into `dist/pdfjs/` at build time; nothing is fetched from the network.
 */
export const PDFJS_ASSET_BASE = `${import.meta.env.BASE_URL}pdfjs/`;

GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

/**
 * Document options for a PDF the user ingested from an arbitrary URL.
 *
 * `enableScripting` is the flag PDF.js reads to decide whether a document's
 * embedded JavaScript may run (`AnnotationLayer` in `pdf.mjs` gates every JS
 * action on it). It is set here, at document construction, so it travels with
 * every consumer of these options rather than depending on a call site
 * remembering it. The version bump that closed CVE-2026-16633 is not a
 * substitute for it.
 *
 * `enableXfa` is the same decision for XFA, which is a second scripted form
 * engine inside the PDF spec.
 */
export const PDF_DOCUMENT_OPTIONS = Object.freeze({
  enableScripting: false,
  enableXfa: false,
  cMapUrl: `${PDFJS_ASSET_BASE}cmaps/`,
  cMapPacked: true,
  standardFontDataUrl: `${PDFJS_ASSET_BASE}standard_fonts/`,
  wasmUrl: `${PDFJS_ASSET_BASE}wasm/`,
  iccUrl: `${PDFJS_ASSET_BASE}iccs/`,
});

/**
 * Page render options.
 *
 * `AnnotationMode.DISABLE` is the other half of `enableScripting: false`: it
 * stops PDF.js drawing annotation appearance streams at all, so no widget,
 * link action or form field from an untrusted document reaches the DOM. The
 * viewer therefore never constructs an `AnnotationLayer`, which is the only
 * thing in PDF.js that can dispatch a document's JavaScript actions.
 */
export const PDF_RENDER_OPTIONS = Object.freeze({
  annotationMode: AnnotationMode.DISABLE,
});

export function loadPdfDocument(data: ArrayBuffer): PDFDocumentLoadingTask {
  return getDocument({ ...PDF_DOCUMENT_OPTIONS, data });
}

/**
 * Draw one page onto `canvas` at `scale`, sized for the device pixel ratio so
 * text is not resampled on a HiDPI display.
 */
export function renderPdfPage(
  page: PDFPageProxy,
  canvas: HTMLCanvasElement,
  scale: number,
): RenderTask {
  const ratio = window.devicePixelRatio || 1;
  const viewport = page.getViewport({ scale: scale * ratio });
  canvas.width = Math.floor(viewport.width);
  canvas.height = Math.floor(viewport.height);
  canvas.style.width = `${Math.floor(viewport.width / ratio)}px`;
  canvas.style.height = `${Math.floor(viewport.height / ratio)}px`;
  return page.render({ ...PDF_RENDER_OPTIONS, canvas, viewport });
}
