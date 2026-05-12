import { expect, test } from "@playwright/test";
import { mockApi } from "./mockApi";

const routes = [
  { id: "overview", path: "./", title: "Runtime posture" },
  { id: "chat", path: "./#/chat", title: "Conversation" },
  { id: "corpus", path: "./#/corpus", title: "Corpus explorer" },
  { id: "memory", path: "./#/memory", title: "Memory and behaviour proposals" },
  { id: "world", path: "./#/world", title: "World model and reasoning quality" },
  { id: "pipelines", path: "./#/pipelines", title: "Pipeline control room" },
  { id: "metrics", path: "./#/metrics", title: "Performance metrics" },
  { id: "settings", path: "./#/settings", title: "Theme and safe controls" },
  { id: "evidence", path: "./#/evidence", title: "Evidence graph" },
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
        });
      });
    }
  });
}
