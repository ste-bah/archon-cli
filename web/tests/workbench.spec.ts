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
  await page.goto("./#/corpus");
  await page.goto("./#/chat");
  await expect(page.getByText("Can you inspect the active run?")).toBeVisible();
  await expect(page.getByText("Mock Archon reply from live session")).toBeVisible();
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
