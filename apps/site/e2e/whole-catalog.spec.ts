import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { collectLogs, expect, REPO, test } from "../../../e2e/support";

// This file sorts after site.spec.ts. With fullyParallel off, a shard takes whole
// files, so this test is the second half and the other site files are the first.
const bodyText = (page: Page) => page.evaluate(() => document.body.innerText);

test("every part page draws with no viewer error", async ({ browser, host }) => {
  test.setTimeout(600_000);
  const ids = readdirSync(join(REPO, "parts"))
    .filter((id) => existsSync(join(REPO, "parts", id, "part.jsonc")))
    .sort();
  expect(ids.length).toBeGreaterThan(0);
  const context = await browser.newContext();
  const page = await context.newPage();
  const errors = collectLogs(page, "errors");
  for (const id of ids) {
    errors.length = 0;
    const response = await page.goto(`${host.base}/embedded/parts/${id}/`);
    expect(response?.ok(), id).toBe(true);
    const ref = `@${id.replaceAll("-", "_")}`;
    await page.waitForFunction(
      (name) => {
        if (document.querySelector("[data-testid='no-webgl']") !== null) return true;
        if (document.querySelector("[data-testid='errors']") !== null) return true;
        return window.orbeViewer?.get().scene.getObjectByName(name) !== undefined;
      },
      ref,
      { timeout: 60_000 },
    );
    expect(await page.locator('[data-testid="errors"]').count(), id).toBe(0);
    expect(errors, `${id} ${JSON.stringify(errors)}`).toEqual([]);
    if (id === "resistor-axial") {
      await expect(page.getByTestId("shown-at")).toHaveText("Shown at 220 Ω.");
      const sentence = await bodyText(page);
      expect(sentence, id).toContain("above 0 Ω (required)");
      await expect(page.getByRole("heading", { name: "Settings" }), id).toBeVisible();
      expect(sentence, id).not.toContain("0 pΩ");
      expect(sentence, id).not.toContain(", required");
      const bands = await page.evaluate(() => {
        const root = window.orbeViewer.get().scene.getObjectByName("@resistor_axial");
        return ["band1-brown", "band2-black", "band3-orange", "band4-gold"].map((node) => {
          const mesh = root?.getObjectByName(node);
          const material = mesh?.material;
          return material && "color" in material ? `#${material.color.getHexString()}` : "";
        });
      });
      expect(bands, id).toEqual(["#d12b2b", "#d12b2b", "#6b3a1f", "#d4af37"]);
    }
    if (id === "led-5mm") await expect(page.getByTestId("shown-at")).toHaveCount(0);
    if (id === "trimmer-3362p") await expect(page.getByTestId("shown-at")).toHaveCount(0);
  }
  await context.close();
});
