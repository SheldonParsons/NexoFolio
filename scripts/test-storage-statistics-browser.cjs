const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || '/Users/sheldon/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright');
const [dir, stage] = process.argv.slice(2);
assert(path.resolve(dir).startsWith('/tmp/nexofolio-step04/run.'));
const m = JSON.parse(fs.readFileSync(path.join(dir, 'browser.json')));
(async () => {
  const browser = await chromium.launch({ channel: 'chrome', headless: true });
  try {
    const context = await browser.newContext({ viewport: { width: 1512, height: 1100 } });
    await context.addInitScript(token => localStorage.setItem('nexofolio.session', JSON.stringify({ token, expiresAt: '2099-01-01T00:00:00Z' })), m.token);
    const page = await context.newPage(), errors = [], requests = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('request', request => {
      if (request.url().includes('/storage')) requests.push({ path: new URL(request.url()).pathname, method: request.method() });
    });
    await page.goto(m.front + '/management/storage');
    await page.getByRole('combobox', { name: '项目' }).fill('统计修复 3000');
    await page.getByRole('option', { name: '统计修复 3000', exact: true }).click();
    const row = page.getByRole('row').filter({ hasText: '正式知识 · 字段与接口说明' });
    await row.waitFor();
    assert.match(await row.innerText(), /6,000条统计记录 · 完整覆盖/);
    if (stage === 'complete') {
      const cells = page.locator('.storage-table tbody tr');
      const format = bytes => bytes < 1024 ? `${bytes} B` : `${new Intl.NumberFormat('zh-CN', { maximumFractionDigits: 1 }).format(bytes / (bytes < 1048576 ? 1024 : 1048576))} ${bytes < 1048576 ? 'KiB' : 'MiB'}`;
      assert.equal(await cells.count(), 6);
      for (let i = 0; i < 6; i++) {
        assert.equal(await cells.nth(i).locator('td').nth(1).innerText(), format(m.expected[i][1]));
        assert.equal(await cells.nth(i).locator('td').nth(3).innerText(), new Intl.NumberFormat('zh-CN').format(m.expected[i][0]) + '条统计记录 · 完整覆盖');
      }
      assert(!((await page.locator('body').innerText()).includes('超过1000条')));
    } else {
      const sample = page.locator('.storage-sample').nth(1);
      await sample.locator('.ops-stale').waitFor();
      await sample.locator('summary').click();
      await sample.getByText(/最近采样未成功：采样超时或暂不可用/).waitFor();
      assert.match(await sample.innerText(), /保留值仍属于原采样时间/);
      assert.equal(await page.locator('.storage-sample').first().locator('.ops-stale').count(), 0);
    }
    await row.scrollIntoViewIfNeeded();
    await page.waitForTimeout(300);
    await page.screenshot({ path: path.join(m.out, `statistics-${stage}.png`), animations: 'disabled' });
    assert.deepEqual(errors, []);
    assert(!requests.some(r => r.path.endsWith('/maintenance')));
    fs.writeFileSync(path.join(m.out, `browser-${stage}.json`), JSON.stringify({ stage, errors, requests, checked_rows: 6, faults: stage === 'timeout' ? 'real PostgreSQL table lock' : null }, null, 2) + '\n');
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
