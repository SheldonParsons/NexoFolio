/* One isolated Chrome context observing the real final bundle/API throughout the window. */
const fs = require('node:fs'), path = require('node:path'), assert = require('node:assert/strict');
const { chromium } = require('/Users/sheldon/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright');
const m = JSON.parse(fs.readFileSync(process.argv[2]));
const sleep = ms => new Promise(r => setTimeout(r, ms));
(async () => {
  const browser = await chromium.launch({ channel: 'chrome', headless: true });
  const context = await browser.newContext({ viewport: { width: 1512, height: 1050 } });
  await context.addInitScript(token => localStorage.setItem('nexofolio.session', JSON.stringify({ token, expiresAt: '2099-01-01T00:00:00Z' })), m.token);
  const page = await context.newPage(), errors = [], requests = [], phases = [], apply = [];
  const starts = new Map(); let applied = false, turns = 0;
  page.setDefaultTimeout(25000);
  page.on('pageerror', e => errors.push({ at: Date.now(), message: e.message }));
  page.on('request', r => starts.set(r, performance.now()));
  page.on('response', async r => {
    const url = new URL(r.url()); if (!url.pathname.startsWith('/api/')) return;
    const row = { at: Date.now(), path: url.pathname, method: r.request().method(), status: r.status(), ttfb_ms: performance.now()-starts.get(r.request()) };
    try {
      const body = await r.body(); row.bytes = body.length; row.total_ms = performance.now()-starts.get(r.request());
      if (url.pathname.endsWith('/apply-status')) { const v=JSON.parse(body); phases.push({ at: row.at, state: v.candidate?.state, progress: v.progress, operation: v.operation }); }
      if (row.method==='POST' && url.pathname.endsWith('/apply')) apply.push(row);
    } catch { row.body_unknown = true; }
    requests.push(row);
    if (requests.length%100===0) fs.writeFileSync(path.join(m.out,'browser-network.json'), JSON.stringify({ requests, errors, phases }));
  });
  const review = `${m.front}/projects/${m.project}/interfaces/curation/${m.candidate}`;
  const shot = name => page.screenshot({ path:path.join(m.out,name+'.png'),fullPage:true,animations:'disabled' });
  try {
    while (!fs.existsSync(path.join(m.tmp,'stop-browser'))) {
      if (fs.existsSync(path.join(m.tmp,'restarting'))) { await sleep(1000); continue; }
      if (!applied && fs.existsSync(path.join(m.tmp,'apply-now'))) {
        await page.goto(review); await page.locator('.candidate-tree-heading').waitFor();
        await page.getByRole('button',{name:'本轮操作',exact:true}).click();
        await page.getByRole('menuitem',{name:'应用到现行文档',exact:true}).click();
        await page.getByRole('button',{name:'确认应用',exact:true}).click();
        await page.locator('.curation-state-tag').filter({hasText:'已应用'}).waitFor({timeout:90000});
        await page.getByRole('alertdialog').waitFor({state:'hidden'}); await shot('browser-applied');
        await sleep(300); assert.equal(apply.length,1); assert.equal(apply[0].status,200);
        fs.writeFileSync(path.join(m.out,'browser-apply.json'),JSON.stringify({passed:true,apply,phases,errors},null,2));applied=true;
      } else {
        await page.goto(turns%2===0 ? review : `${m.front}/management/operations`);
        if (turns%2===0) await page.locator('.candidate-tree-heading').waitFor();
        else await page.getByRole('heading',{name:'运行状态',exact:true}).waitFor();
        if (turns<2) await shot(turns===0?'browser-review':'browser-operations');
        turns++;
        if (turns===1) fs.writeFileSync(path.join(m.tmp,'browser-ready'),'ready');
      }
      // Alternate opening/closing the observation page with genuine page navigation.
      for(let i=0;i<30&&!fs.existsSync(path.join(m.tmp,'stop-browser'))&&!(!applied&&fs.existsSync(path.join(m.tmp,'apply-now')));i++) await sleep(1000);
    }
  } catch(error) {
    await shot('browser-failure').catch(()=>{});
    fs.writeFileSync(path.join(m.out,'browser-apply.json'),JSON.stringify({passed:false,error:error.message,apply,phases,errors},null,2));
    throw error;
  } finally {
    fs.writeFileSync(path.join(m.out,'browser-network.json'),JSON.stringify({requests,errors,phases,turns},null,2));
    await browser.close();
  }
})().catch(e=>{console.error(e.message);process.exitCode=1;});
