import { chromium } from 'playwright';
const out = '/Volumes/Workspace/Github/compass-hierarchy-check/shots-repos';
const url = 'file:///Volumes/Workspace/Github/compass-hierarchy-check/zod/compass-out/graph.html';
const browser = await chromium.launch();
const shoot = async (name, level) => {
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2 });
  const started = Date.now();
  await page.goto(url);
  await page.waitForSelector('#compass-viewer-root canvas', { timeout: 60000 });
  await page.waitForTimeout(5000);
  const load = Date.now() - started;
  if (level !== undefined) {
    const button = page.locator(`[aria-label="Level ${level}"]`).first();
    if (await button.count()) {
      await button.click();
      await page.waitForTimeout(6000);
    }
  }
  await page.screenshot({ path: `${out}/${name}.png` });
  const status = await page.locator('.compass-viewer-status-text').first().innerText().catch(() => '');
  console.log(name, '| load', load, 'ms |', status.replace(/\n/g, ' '));
  await page.close();
};
await shoot('zod-level0');
await shoot('zod-level2', 2);
await shoot('zod-level3', 3);
await browser.close();
