import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { WebChatAttachment } from "../api/generated/web";
import { AttachmentList } from "./ChatPage";

function attachment(overrides: Partial<WebChatAttachment> = {}): WebChatAttachment {
  return {
    fileName: "report.pdf",
    sizeBytes: 2048,
    mimeType: "application/pdf",
    accepted: true,
    policyReason: "stored and forwarded to live session",
    dataBase64: null,
    storedPath: "~/.archon/web/uploads/webmsg_test/report.pdf",
    documentId: null,
    ...overrides,
  };
}

describe("chat attachment chips", () => {
  it("shows the docs id the ingest assigned", () => {
    render(<AttachmentList attachments={[attachment({ documentId: "doc_9f2c" })]} />);

    expect(screen.getByText("report.pdf")).toBeTruthy();
    expect(screen.getByText(/ingested as doc_9f2c/)).toBeTruthy();
  });

  it("shows no id for an attachment that was stored but never ingested", () => {
    render(<AttachmentList attachments={[attachment()]} />);

    expect(screen.getByText("stored")).toBeTruthy();
    expect(screen.queryByText(/ingested as/)).toBeNull();
  });

  it("does not render an empty id as if it were a real one", () => {
    // An empty string is absence, not a document. It must not reach the chip.
    render(<AttachmentList attachments={[attachment({ documentId: "" })]} />);

    expect(screen.queryByText(/ingested as/)).toBeNull();
  });
});
