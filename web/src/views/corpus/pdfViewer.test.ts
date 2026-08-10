import { beforeEach, describe, expect, it, vi } from "vitest";

// The point of this file is the security contract of the PDF viewer, and the
// only way to hold that contract is to assert on the options PDF.js actually
// receives. Grepping the source for `enableScripting` would pass on a refactor
// that stopped passing it.
const pdfjs = vi.hoisted(() => ({
  getDocument: vi.fn((_options: Record<string, unknown>) => ({
    promise: Promise.resolve({}),
  })),
  globalWorkerOptions: { workerSrc: "" } as { workerSrc: string },
  annotationMode: { DISABLE: 0, ENABLE: 1, ENABLE_FORMS: 2, ENABLE_STORAGE: 3 },
}));

vi.mock("pdfjs-dist", () => ({
  AnnotationMode: pdfjs.annotationMode,
  GlobalWorkerOptions: pdfjs.globalWorkerOptions,
  getDocument: pdfjs.getDocument,
}));

const {
  PDF_DOCUMENT_OPTIONS,
  PDF_RENDER_OPTIONS,
  loadPdfDocument,
  renderPdfPage,
} = await import("./pdfViewer");

/** Same-origin, however the bundler chose to spell the URL. */
function isLocalUrl(value: string) {
  return new URL(value, window.location.href).origin === window.location.origin;
}

beforeEach(() => {
  pdfjs.getDocument.mockClear();
});

describe("pdf viewer security options", () => {
  it("constructs every document with embedded scripting disabled", () => {
    loadPdfDocument(new ArrayBuffer(8));

    expect(pdfjs.getDocument).toHaveBeenCalledTimes(1);
    const options = pdfjs.getDocument.mock.calls[0]?.[0] ?? {};
    // Strict `false`, not falsy: PDF.js reads these as explicit booleans and
    // an absent key is the default, which is what the CVE turned on.
    expect(options.enableScripting).toBe(false);
    expect(options.enableXfa).toBe(false);
  });

  it("never renders the annotation layer that dispatches document JavaScript", () => {
    const canvas = document.createElement("canvas");
    const viewport = { width: 200, height: 300 };
    const render = vi.fn((_options: Record<string, unknown>) => ({
      promise: Promise.resolve(),
    }));
    const page = { getViewport: vi.fn(() => viewport), render };

    renderPdfPage(page as never, canvas, 1.25);

    expect(render).toHaveBeenCalledTimes(1);
    const options = render.mock.calls[0]?.[0] ?? {};
    expect(options.annotationMode).toBe(pdfjs.annotationMode.DISABLE);
    expect(PDF_RENDER_OPTIONS.annotationMode).toBe(pdfjs.annotationMode.DISABLE);
  });

  it("loads the pdf worker from a local asset rather than a CDN", () => {
    expect(pdfjs.globalWorkerOptions.workerSrc).not.toBe("");
    expect(pdfjs.globalWorkerOptions.workerSrc).not.toMatch(/^(https?:)?\/\//);
    expect(isLocalUrl(pdfjs.globalWorkerOptions.workerSrc)).toBe(true);
  });

  it("resolves cmaps, fonts, wasm and icc profiles from this origin", () => {
    for (const url of [
      PDF_DOCUMENT_OPTIONS.cMapUrl,
      PDF_DOCUMENT_OPTIONS.standardFontDataUrl,
      PDF_DOCUMENT_OPTIONS.wasmUrl,
      PDF_DOCUMENT_OPTIONS.iccUrl,
    ]) {
      expect(url).not.toMatch(/^(https?:)?\/\//);
      expect(isLocalUrl(url)).toBe(true);
    }
  });

  it("freezes the options so a caller cannot re-enable scripting globally", () => {
    expect(Object.isFrozen(PDF_DOCUMENT_OPTIONS)).toBe(true);
    expect(Object.isFrozen(PDF_RENDER_OPTIONS)).toBe(true);
  });
});
