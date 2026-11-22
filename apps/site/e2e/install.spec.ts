import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { expect, REPO, test } from "../../../e2e/support";

// The install-off build has no `orbe add` line, so the phone-width and Copy checks that depend
// on it run here, against the build `check-site.sh` makes with ORBE_SITE_INSTALL=1.
test.skip(process.env.ORBE_SITE_INSTALL_E2E !== "1", "runs against a build with install lines");

function ids(dir: string, file: string): string[] {
  return readdirSync(join(REPO, dir))
    .filter((name) => existsSync(join(REPO, dir, name, file)))
    .sort();
}

test("the start page shows each command and the Block's scene", async ({ browser, host }) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(`${host.base}/orbe/`);
  const installs = page.locator('[data-testid="install"]');
  await expect(installs).toHaveCount(8);
  const texts = (await installs.locator("[data-copy-text]").allInnerTexts()).map((text) => text.trim());
  expect(texts).toEqual([
    "curl -fsSL https://orbefab.com/orbe/install.sh | sh",
    "orbe skills get core",
    "orbe add nano-button-led",
    "orbe check scene.jsonc",
    "orbe test",
    "orbe open --headed",
    "orbe play",
    "orbe close",
  ]);
  await expect(page.getByTestId("scene-file")).toContainText("breadboard-400");
  await expect(page.getByTestId("cli-coming")).toHaveCount(0);
  const body = await page.locator("main").innerText();
  expect(body).toContain("For Linux x86_64 and macOS arm64.");
  expect(body).toContain("Run this once so your agent knows how to use orbe.");
  expect(body).toContain("A Block is a small circuit that works: a scene file, its firmware and its tests. In an empty folder, orbe add copies one in.");
  expect(body).toContain("The scene file describes the circuit:");
  expect(body).toContain("Runs the firmware and the Block's tests.");
  expect(body).toContain("Opens the circuit in 3D, runs time, then closes the session.");
  expect(body).toContain("Boards:");
  expect(body).not.toContain("Boards today:");
  expect(body).not.toContain("parts.lock");
  expect(body).not.toContain("releases page");
  await context.close();
});

test("every part and Block page fits a phone width with its install line", async ({ browser, host }) => {
  test.skip(host.name !== "vite preview", "one pass covers every page");
  test.setTimeout(600_000);
  const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
  const page = await context.newPage();
  const fits = () => page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1);
  const pages = [
    ...ids("parts", "part.jsonc").map((id) => `/orbe/parts/${id}/`),
    ...ids("blocks", "block.jsonc").map((id) => `/orbe/blocks/${id}/`),
  ];
  for (const path of pages) {
    await page.goto(`${host.base}${path}`);
    const install = page.locator('[data-testid="install"]');
    await expect(install, path).toBeVisible();
    const copy = install.locator('[data-testid="copy"]');
    await expect(copy, path).toBeVisible();
    const box = await copy.boundingBox();
    expect(box, path).not.toBeNull();
    expect(box!.x + box!.width, path).toBeLessThanOrEqual(391);
    expect(await fits(), path).toBe(true);
    if (path.startsWith("/orbe/blocks/")) {
      await page.locator('[data-testid="view-code"]').click();
      expect(await fits(), `${path} code`).toBe(true);
    }
  }
  await context.close();
});

test("Copy says Copied and writes the command", async ({ browser, host }) => {
  const context = await browser.newContext();
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const page = await context.newPage();
  await page.goto(`${host.base}/orbe/parts/capacitor-electrolytic-radial/`);
  const install = page.locator('[data-testid="install"]');
  const command = await install.locator("[data-copy-text]").innerText();
  await install.locator('[data-testid="copy"]').click();
  await expect(install.locator('[data-testid="copy"]')).toHaveText("Copied");
  await expect(install.locator('[data-testid="copy"]')).toHaveAttribute("data-copy-state", "copied");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(command);
  await context.close();
});

async function refused(page: import("@playwright/test").Page, host: string, script: string) {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.addInitScript(script);
  await page.goto(`${host}/orbe/parts/capacitor-electrolytic-radial/`);
  const install = page.locator('[data-testid="install"]');
  const command = await install.locator("[data-copy-text]").innerText();
  await install.locator('[data-testid="copy"]').click();
  const button = install.locator('[data-testid="copy"]');
  await expect(button).toHaveText("Couldn't copy");
  await expect(button).toHaveAttribute("data-copy-state", "failed");
  expect(await page.evaluate(() => window.getSelection()?.toString() ?? "")).toBe(command);
  expect(errors, JSON.stringify(errors)).toEqual([]);
}

test("Copy selects the command when the clipboard is missing", async ({ browser, host }) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  await refused(page, host.base, "Object.defineProperty(navigator, 'clipboard', { configurable: true, get() { return undefined; } });");
  await context.close();
});

test("Copy selects the command when the write is refused", async ({ browser, host }) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  await refused(
    page,
    host.base,
    "Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText() { return Promise.reject(new Error('denied')); } } });",
  );
  await context.close();
});

test("a same-page hash stays on the page when clicked twice", async ({ browser, host }) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  const urls: string[] = [];
  const errors: string[] = [];
  page.on("request", (request) => {
    if (/_serverFn|__tsr\//.test(request.url())) urls.push(request.url());
  });
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`${host.base}/orbe/parts/led-5mm/`);
  await page.waitForFunction(() => {
    for (const node of document.querySelectorAll("body *")) {
      if (Object.keys(node).some((key) => key.startsWith("__reactFiber") || key.startsWith("__reactContainer"))) return true;
    }
    return false;
  });
  const link = page.locator('main a[href="#used"]');
  const title = await page.locator("h1").innerText();
  await link.click();
  await link.click();
  expect(urls, JSON.stringify(urls)).toEqual([]);
  expect(errors, JSON.stringify(errors)).toEqual([]);
  await expect(page.locator("h1")).toHaveText(title);
  await expect(page.locator("body")).not.toContainText("Something went wrong");
  await expect(page.locator("#used")).toBeVisible();
  await context.close();
});

test("a Block page keeps the install line and the short code note", async ({ browser, host }) => {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.goto(`${host.base}/orbe/blocks/nano-button-led/`);
  await expect(page.locator('[data-testid="install"]')).toContainText("orbe add nano-button-led");
  await expect(page.getByText("New to orbe?")).toHaveCount(0);
  await page.locator('[data-testid="view-code"]').click();
  await expect(page.locator('[data-testid="view-code-panel"]')).toContainText("orbe add writes these files.");
  await expect(page.locator('[data-testid="view-code-panel"]')).not.toContainText("REUSE.toml");
  await context.close();
});
