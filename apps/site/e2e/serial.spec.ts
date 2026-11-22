import type { Page } from "@playwright/test";
import { expect, shot, test, waitState } from "../../../e2e/support";

test.describe("serial output", () => {
  test("the comparator shows dark at 10 lx and light at 300 lx", async ({ page, host }) => {
    const shotName = host.name.replaceAll(" ", "-");
    await page.goto(`${host.base}/orbe/blocks/nano-light-comparator/`);
    await waitState(page, "playing");
    const strip = page.locator('[data-testid="serial"]');
    await expect(strip).toContainText("light", { timeout: 30_000 });
    await expect(strip).not.toContainText("dark");
    await shot(page, `${shotName}-serial-300-light`);

    await setLight(page, "10");
    await expect(page.locator('[data-testid="world-light-value"]')).toHaveAttribute("data-frame", "10", { timeout: 15_000 });
    await expect(strip).toContainText("dark", { timeout: 30_000 });
    await shot(page, `${shotName}-serial-10-dark`);

    const dark = await strip.innerText();
    await setLight(page, "300");
    await expect(page.locator('[data-testid="world-light-value"]')).toHaveAttribute("data-frame", "300", { timeout: 15_000 });
    await expect.poll(async () => strip.innerText(), { timeout: 30_000 }).toMatch(/dark[\s\S]*light/);
    expect(await strip.innerText()).not.toBe(dark);
    await shot(page, `${shotName}-serial-300-again`);
  });

  test("a reload shows the new session's text", async ({ page, host }) => {
    await page.goto(`${host.base}/orbe/blocks/nano-light-comparator/`);
    await waitState(page, "playing");
    await setLight(page, "10");
    await expect(page.locator('[data-testid="serial"]')).toContainText("dark", { timeout: 30_000 });
    await page.reload();
    await waitState(page, "playing");
    const strip = page.locator('[data-testid="serial"]');
    await expect(strip).toContainText("light", { timeout: 30_000 });
    await expect(strip).not.toContainText("dark");
    await shot(page, `${host.name.replaceAll(" ", "-")}-serial-reloaded`);
  });

  test("a Block that prints nothing shows no strip", async ({ page, host }) => {
    await page.goto(`${host.base}/orbe/blocks/nano-button-led/`);
    await waitState(page, "playing");
    await page.waitForFunction(
      () => {
        const time = document.querySelector('[data-testid="time"]')?.textContent ?? "";
        return time !== "" && time !== "t = 0.000000 s";
      },
      null,
      { timeout: 30_000 },
    );
    await expect(page.locator('[data-testid="serial"]')).toHaveCount(0);
    await shot(page, `${host.name.replaceAll(" ", "-")}-serial-silent`);
  });

  test("the strip is in view at 375 by 812 without scrolling", async ({ page, host }) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await page.goto(`${host.base}/orbe/blocks/nano-light-comparator/`);
    await waitState(page, "playing");
    const strip = page.locator('[data-testid="serial"]');
    await expect(strip).toContainText("light", { timeout: 30_000 });
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
    const box = await strip.boundingBox();
    expect(box, "the strip has a box").not.toBeNull();
    if (box === null) return;
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.y).toBeGreaterThanOrEqual(0);
    expect(box.height).toBeGreaterThan(0);
    expect(box.x + box.width).toBeLessThanOrEqual(375 + 1);
    expect(box.y + box.height).toBeLessThanOrEqual(812);
    await shot(page, `${host.name.replaceAll(" ", "-")}-serial-375`);
  });

  test("the strip stays on a phone-width screen", async ({ page, host }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(`${host.base}/orbe/blocks/nano-light-comparator/`);
    await waitState(page, "playing");
    const strip = page.locator('[data-testid="serial"]');
    await expect(strip).toContainText("light", { timeout: 30_000 });
    await strip.scrollIntoViewIfNeeded();
    const box = await strip.boundingBox();
    const viewport = page.viewportSize();
    expect(box, "the strip has a box").not.toBeNull();
    expect(viewport, "the viewport is set").not.toBeNull();
    if (box === null || viewport === null) return;
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.y).toBeGreaterThanOrEqual(0);
    expect(box.height).toBeGreaterThan(0);
    expect(box.x + box.width).toBeLessThanOrEqual(viewport.width + 1);
    await shot(page, `${host.name.replaceAll(" ", "-")}-serial-phone`);
  });

  test("the strip follows both themes", async ({ page, host }) => {
    const shotName = host.name.replaceAll(" ", "-");
    await page.goto(`${host.base}/orbe/blocks/nano-light-comparator/`);
    await waitState(page, "playing");
    const strip = page.locator('[data-testid="serial"]');
    await expect(strip).toContainText("light", { timeout: 30_000 });
    const dark = await ink(strip);
    expect(luma(dark.color)).toBeGreaterThan(luma(dark.background));
    await shot(page, `${shotName}-serial-theme-dark`);
    await page.evaluate(() => {
      localStorage.setItem("theme", "light");
      document.documentElement.classList.remove("dark");
      document.documentElement.classList.add("light");
    });
    const light = await ink(strip);
    expect(luma(light.color)).toBeLessThan(luma(light.background));
    await shot(page, `${shotName}-serial-theme-light`);
  });
});

async function setLight(page: Page, value: string) {
  const field = page.locator('[data-testid="world-light-value"]');
  await field.click();
  await field.fill(value);
  await field.press("Enter");
}

function ink(strip: ReturnType<Page["locator"]>) {
  return strip.evaluate((element) => {
    const style = getComputedStyle(element);
    return { color: style.color, background: style.backgroundColor };
  });
}

function luma(rgb: string): number {
  const [red, green, blue] = rgb.match(/\d+/g)?.slice(0, 3).map(Number) ?? [0, 0, 0];
  return 0.2126 * (red ?? 0) + 0.7152 * (green ?? 0) + 0.0722 * (blue ?? 0);
}
