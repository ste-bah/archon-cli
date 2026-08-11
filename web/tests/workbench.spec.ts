import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { mockApi } from "./mockApi";

const routes = [
  { id: "overview", path: "./", nav: "Overview", title: "Runtime posture" },
  { id: "chat", path: "./#/chat", nav: "Chat", title: "Conversation" },
  { id: "corpus", path: "./#/corpus", nav: "Corpus", title: "Corpus explorer" },
  { id: "ingest", path: "./#/ingest", nav: "Ingest", title: "Ingest control" },
  { id: "memory", path: "./#/memory", nav: "Memory", title: "Memory and behaviour proposals" },
  { id: "world", path: "./#/world", nav: "World Model", title: "World model and reasoning quality" },
  { id: "jepa", path: "./#/jepa", nav: "JEPA", title: "JEPA candidates and gates" },
  { id: "pipelines", path: "./#/pipelines", nav: "Pipelines", title: "Pipeline control room" },
  { id: "metrics", path: "./#/metrics", nav: "Metrics", title: "Performance metrics" },
  { id: "settings", path: "./#/settings", nav: "Settings", title: "Theme and safe controls" },
  { id: "evidence", path: "./#/evidence", nav: "Evidence", title: "Evidence graph" },
];

for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme} workbench`, () => {
    test.beforeEach(async ({ page }) => {
      await page.addInitScript((mode) => {
        window.localStorage.setItem("archon.theme", mode);
      }, theme);
      await mockApi(page);
    });

    for (const route of routes) {
      test(`${route.id} screenshot`, async ({ page }) => {
        await page.goto(route.path);
        await expect(page.getByRole("heading", { name: route.title })).toBeVisible();
        if (route.id === "evidence") {
          await expect(page.locator(".evidence-graph canvas").first()).toBeVisible();
        }
        await expect(page).toHaveScreenshot(`${route.id}-${theme}.png`, {
          fullPage: true,
          animations: "disabled",
          maxDiffPixels: route.id === "evidence" ? 100 : 0,
        });
      });
    }
  });
}

test("sidebar links load every workbench tab", async ({ page }) => {
  await mockApi(page);
  const assertNoErrors = watchBrowserErrors(page);
  await page.goto("./");
  await page.getByRole("button", { name: "Switch to light theme" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await page.getByRole("button", { name: "Switch to dark theme" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");

  for (const route of routes) {
    await page.getByRole("link", { name: new RegExp(`^${escapeRegExp(route.nav)}\\b`) }).click();
    await expect(page.getByRole("heading", { name: route.title })).toBeVisible();
  }

  assertNoErrors();
});

test("chat send and attach controls are interactive", async ({ page }) => {
  await mockApi(page);
  const assertNoErrors = watchBrowserErrors(page);
  await page.goto("./#/chat");

  const firstChooser = page.waitForEvent("filechooser");
  await page.getByRole("button", { name: "Attach" }).click();
  await (await firstChooser).setFiles({
    name: "notes.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("hello"),
  });
  await expect(page.getByRole("button", { name: /notes\.txt/ })).toBeVisible();
  await page.getByRole("button", { name: /notes\.txt/ }).click();
  await expect(page.getByRole("button", { name: /notes\.txt/ })).toBeHidden();

  const secondChooser = page.waitForEvent("filechooser");
  await page.getByRole("button", { name: "Attach" }).click();
  await (await secondChooser).setFiles({
    name: "context.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("context"),
  });
  await page.getByLabel("Message").fill("Can you inspect the active run?");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByText("Can you inspect the active run?")).toBeVisible();
  await expect(page.getByText("Mock Archon reply from live session")).toBeVisible();
  await expect(page.getByText("context.txt")).toBeVisible();
  await expect(page.getByText("ingested as doc_mock_0")).toBeVisible();
  await page.goto("./#/corpus");
  await page.goto("./#/chat");
  await expect(page.getByText("Can you inspect the active run?")).toBeVisible();
  await expect(page.getByText("Mock Archon reply from live session")).toBeVisible();
  await expect(page.getByText("ingested as doc_mock_0")).toBeVisible();
  assertNoErrors();
});

test("memory, world, corpus, and settings buttons perform visible actions", async ({ page }) => {
  await mockApi(page);
  const assertNoErrors = watchBrowserErrors(page);

  await page.goto("./#/memory");
  const previewTitles = {
    memory: "Memory rows",
    learning_event: "Learning events",
    proposal: "Behaviour proposals",
    trust: "Trust deltas",
    all: "All learning rows",
  };
  for (const filter of ["memory", "learning_event", "proposal", "trust", "all"] as const) {
    await page.getByRole("button", { name: filter, exact: true }).click();
    await expect(page.getByRole("button", { name: filter, exact: true })).toHaveClass(/active/);
    await expect(page.locator(".memory-filter-preview")).toContainText(previewTitles[filter]);
  }
  await page.getByRole("button", { name: "proposal", exact: true }).click();
  await page.getByRole("button", { name: "Preview approval" }).click();
  await expect(page.getByRole("status")).toContainText("behaviour.proposal.approve");

  await page.goto("./#/corpus");
  await page.locator(".corpus-results").getByRole("button", { name: /World model PRD/ }).click();
  await expect(page.getByRole("heading", { name: "World model PRD" })).toBeVisible();
  await expect(page.getByText("Latent next-state prediction")).toBeVisible();
  await page.getByLabel("Ranked corpus chunks").getByRole("button", { name: /README\.md/ }).click();
  await expect(page.getByRole("heading", { name: "README.md" })).toBeVisible();
  await page.getByRole("button", { name: /Repository docs/ }).click();
  await expect(page.getByRole("status")).toContainText("repo/docs");

  await page.goto("./#/ingest");
  await page.getByPlaceholder("/path/file.pdf or https://...").fill("/repo/hld/design.pdf");
  await page.getByRole("button", { name: "Run" }).first().click();
  await expect(page.getByText("Store items")).toBeVisible();
  await page.getByRole("button", { name: "videos" }).click();
  await expect(page.getByRole("button", { name: "Architecture walkthrough" })).toBeVisible();
  await page.getByPlaceholder("project evidence").fill("new kb");
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page.getByText("Recent ingest runs")).toBeVisible();

  await page.goto("./#/world");
  await page.getByRole("button", { name: "Candidates", exact: true }).click();
  await expect(page.getByRole("button", { name: "Show all" })).toBeVisible();
  await page.getByRole("button", { name: "Preview promote" }).first().click();
  await expect(page.getByRole("status")).toContainText("world.candidate.promote");
  await page.getByRole("button", { name: "Preview rollback" }).click();
  await expect(page.getByRole("status")).toContainText("world.active.rollback");
  await page.getByRole("button", { name: "Preview promote" }).nth(1).click();
  await expect(page.getByRole("status")).toContainText("world.candidate.promote");

  await page.goto("./#/jepa");
  await page.getByRole("button", { name: "eval", exact: true }).click();
  await expect(page.getByRole("button", { name: /eval-001\.json/ }).first()).toBeVisible();

  await page.goto("./#/metrics");
  await page.getByRole("button", { name: "Bundle files" }).click();
  await expect(page.getByLabel("Selected metric detail")).toContainText("Bundle files");

  await page.goto("./#/settings");
  await page.getByRole("button", { name: "Light", exact: true }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await page.getByRole("button", { name: "Blue" }).click();
  await expect(page.getByRole("button", { name: "Blue" })).toHaveClass(/active/);
  await page.getByRole("button", { name: "compact" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-density", "compact");
  await page.getByRole("button", { name: "Export current profile" }).click();
  await expect(page.getByLabel("Theme profile JSON")).toContainText('"accentId": "blue"');
  await page.getByLabel("Theme profile JSON").fill(JSON.stringify({
    themeMode: "light",
    densityMode: "comfortable",
    accentId: "rose",
    accentHex: "#f0a0b6",
    accentStrongHex: "#cf5578",
    updatedAtMs: 1770000002,
  }, null, 2));
  await page.getByRole("button", { name: "Import profile" }).click();
  await expect(page.getByRole("button", { name: "Rose" })).toHaveClass(/active/);
  await expect(page.locator("html")).toHaveAttribute("data-density", "comfortable");

  assertNoErrors();
});

test("a live event refetches the surfaces it maps to, and only those", async ({ page }) => {
  // Cursor 8 is at the snapshot's `nextCursor`, so the client treats it as an
  // event that arrived while it was watching rather than as backlog.
  await mockApi(page, {
    liveStreamFrame: {
      events: [
        {
          cursor: 8,
          eventType: "web.chat.completed",
          summary: "web chat turn msg-1 completed",
          createdAtMs: 2,
        },
      ],
      nextCursor: 9,
      compacted: false,
    },
  });
  // `/api/cognitive/summary` is deliberately absent: this mock does not serve
  // it, and the client's `retry: 1` would inflate its count on its own. The
  // other two surfaces a completed chat turn maps to are enough to prove the
  // wiring, and the mapping itself is covered in `liveQueryInvalidation.test`.
  const requests = countApiRequests(page, [
    "/api/metrics/summary",
    "/api/learning/summary",
    "/api/pipelines/summary",
    "/api/world/summary",
  ]);

  await page.goto("./#/metrics");
  await expect(page.getByRole("heading", { name: "Performance metrics" })).toBeVisible();
  await expect.poll(() => requests["/api/metrics/summary"]).toBe(1);

  // A completed chat turn writes the metrics and learning stores, so both
  // refetch off the event with no timer involved.
  await expect
    .poll(() => requests["/api/metrics/summary"], { timeout: 10_000 })
    .toBeGreaterThan(1);
  await expect.poll(() => requests["/api/learning/summary"]).toBeGreaterThan(1);

  // Pipelines and world are written by other processes the event says nothing
  // about. They must stay at their single page-load fetch.
  expect(requests["/api/pipelines/summary"]).toBe(1);
  expect(requests["/api/world/summary"]).toBe(1);
});

function countApiRequests(page: Page, paths: string[]) {
  const counts: Record<string, number> = Object.fromEntries(paths.map((path) => [path, 0]));
  page.on("request", (request) => {
    const { pathname } = new URL(request.url());
    if (pathname in counts) {
      counts[pathname] = (counts[pathname] ?? 0) + 1;
    }
  });
  return counts;
}

function watchBrowserErrors(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") {
      errors.push(message.text());
    }
  });
  return () => expect(errors).toEqual([]);
}

function escapeRegExp(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
