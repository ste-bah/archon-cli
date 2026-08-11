import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { WebKnowledgeBaseItem } from "../../api/generated/web";
import { itemSubtitle } from "../IngestPage";
import { KbWarnings } from "./KbWarnings";

function kb(overrides: Partial<WebKnowledgeBaseItem> = {}): WebKnowledgeBaseItem {
  return {
    name: "alpha notes",
    scope: "store",
    path: "",
    files: 0,
    bytes: 0,
    exists: false,
    origin: "db",
    documents: 4,
    ...overrides,
  };
}

describe("knowledge base listing", () => {
  it("says why the list may be short when the store could not be read", () => {
    render(<KbWarnings warnings={["the store is not readable"]} />);

    expect(screen.getByText(/the store is not readable/)).toBeTruthy();
  });

  it("says nothing when there is nothing to say", () => {
    // The contrast that makes the warning meaningful: a store with no
    // knowledge bases must not carry the same message as an unreadable one.
    const { container } = render(<KbWarnings warnings={[]} />);

    expect(container.querySelector(".ingest-kb-warnings")).toBeNull();
  });

  it("shows the exact name to pass to --kb", () => {
    // The row is for a knowledge base whose stored name is not its directory
    // slug, which is precisely when guessing from the slug fails.
    expect(itemSubtitle(kb())).toContain("--kb alpha notes");
  });

  it("names the origin without letting it hide anything", () => {
    expect(itemSubtitle(kb({ origin: "dir", documents: 0 }))).toContain("dir");
    expect(itemSubtitle(kb({ origin: "both", documents: 2 }))).toContain("both");
  });
});
