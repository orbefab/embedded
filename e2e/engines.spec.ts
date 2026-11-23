import type { Page } from "@playwright/test";
import { collectLogs, expect, test, textOf } from "./support";

const viewerPort = process.env.ORBE_VIEWER_PORT ?? "4174";
const sitePort = process.env.ORBE_SITE_PORT ?? "4175";
const viewer = `http://127.0.0.1:${viewerPort}`;
const site = `http://127.0.0.1:${sitePort}`;

const PAGES = [
  ["/orbe/", "A hardware simulator for the AI agent you already use."],
  ["/orbe/parts/", "Parts"],
  ["/orbe/blocks/", "Blocks"],
] as const;

// Headless Firefox on Linux refuses a WebGL context. The page still starts three.js, which logs
// the first three lines, and the thrown Error is logged as "Error". The Block still runs, paused
// until Play. The site detects the refusal first and draws one sentence instead, so it never
// starts the renderer and logs none of those lines. WebKit creates a WebGL 2 context and draws
// the board on a Block page. The same refusal in WebKit fails the check.
const FIREFOX_WEBGL = [
  "THREE.WebGLRenderer: A WebGL context could not be created. Reason:  WebGL creation failed: \n* AllowWebgl2:false restricts context creation on this system. ()",
  "THREE.WebGLRenderer: A WebGL context could not be created. Reason:  WebGL creation failed: \n* AllowWebgl2:false restricts context creation on this system. ()",
  "THREE.WebGLRenderer: THREE.WebGLRenderer: Error creating WebGL context.",
  "Error",
];

function expectLoaded(response: { status(): number } | null, where: string): void {
  const status = response?.status() ?? 0;
  // Status 0 is a refused page. Playwright's ok() treats it as success, and Firefox
  // answers a refused page with that status. A page that loaded answers 2xx.
  expect(status, where).toBeGreaterThanOrEqual(200);
  expect(status, where).toBeLessThan(300);
}

function drawnOrRefused(page: Page): Promise<"board" | "refused"> {
  return page
    .waitForFunction(
      () => {
        if (window.orbeViewer?.get().scene.getObjectByName("@nano") !== undefined) return "board";
        if (document.querySelector("[data-testid='no-webgl']") !== null) return "refused";
        if (document.body.textContent?.includes("Error creating WebGL context")) return "refused";
        return false;
      },
      null,
      { timeout: 30_000 },
    )
    .then((handle) => handle.jsonValue() as Promise<"board" | "refused">);
}

async function timeMoves(page: Page): Promise<void> {
  await page.getByTestId("state").waitFor({ timeout: 60_000 });
  expect(await textOf(page, "state")).toBe("paused");
  await page.getByTestId("play").click();
  expect(await textOf(page, "state")).toBe("playing");
  const first = await textOf(page, "time");
  expect(first).toMatch(/^t = \d+\.\d+ s$/);
  await page.waitForFunction(
    (seen) => {
      const text = document.querySelector('[data-testid="time"]')?.textContent ?? "";
      return /^t = \d+\.\d+ s$/.test(text) && text !== seen;
    },
    first,
    { timeout: 10_000 },
  );
}

test("the page runs the Block, and WebKit shows the board", async ({ page }, info) => {
  const errors = collectLogs(page, "errors");
  const response = await page.goto(`${viewer}/`);
  expectLoaded(response, "viewer");
  const outcome = await drawnOrRefused(page);
  await timeMoves(page);
  if (outcome === "board") {
    const canvas = page.locator("canvas");
    await expect(canvas).toBeVisible();
    const box = await canvas.boundingBox();
    expect(box !== null && box.width > 100 && box.height > 100).toBe(true);
    expect(errors, JSON.stringify(errors)).toEqual([]);
    return;
  }
  expect(info.project.name).toBe("firefox");
  expect(errors, JSON.stringify(errors)).toEqual(FIREFOX_WEBGL);
});

test("the site's main pages load", async ({ page }, info) => {
  const errors = collectLogs(page, "errors");
  for (const [path, heading] of PAGES) {
    errors.length = 0;
    const response = await page.goto(`${site}${path}`);
    expectLoaded(response, path);
    await expect(page.locator("h1")).toHaveText(heading);
    await page.waitForLoadState("networkidle");
    expect(errors, `${path} ${JSON.stringify(errors)}`).toEqual([]);
  }
});

test("a Block page draws in WebKit", async ({ page }, info) => {
  const errors = collectLogs(page, "errors");
  const response = await page.goto(`${site}/orbe/blocks/nano-button-led/`);
  expectLoaded(response, "block");
  await expect(page.locator("h1")).toHaveText("Button and LED on a Nano");
  const outcome = await drawnOrRefused(page);
  if (outcome === "board") {
    expect(errors, JSON.stringify(errors)).toEqual([]);
    return;
  }
  expect(info.project.name).toBe("firefox");
  await expect(page.getByTestId("no-webgl")).toHaveText("Your browser can't show 3D here.");
  await expect(page.getByTestId("block-caption")).toHaveCount(0);
  await expect(page.getByTestId("hold-to-press")).toHaveCount(0);
  expect(errors, JSON.stringify(errors)).toEqual([]);
});
