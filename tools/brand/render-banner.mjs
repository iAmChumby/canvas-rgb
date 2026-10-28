// Renders tools/brand/banner.html to docs/assets/banner-dark.png and banner-light.png.
// Needs app/web/node_modules (the fonts) and Playwright (tools/poison-compare/js).
// Usage: node tools/brand/render-banner.mjs
import path from "node:path";
import { pathToFileURL, fileURLToPath } from "node:url";
import { createRequire } from "node:module";
const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, "../..");
const require = createRequire(path.join(repo, "tools/poison-compare/js/package.json"));
const { chromium } = require("playwright");
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1280, height: 360 }, deviceScaleFactor: 2 });
for (const theme of ["dark", "light"]) {
  await page.goto(pathToFileURL(path.join(here, "banner.html")).href);
  await page.evaluate((t) => (document.documentElement.dataset.theme = t), theme);
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: path.join(repo, "docs/assets", `banner-${theme}.png`) });
}
await browser.close();
console.log("rendered docs/assets/banner-dark.png and banner-light.png");
