const fs = require('node:fs'), path = require('node:path'), assert = require('node:assert/strict');
const { execFileSync, spawn } = require('node:child_process');
const { chromium } = require('/Users/sheldon/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright');
const dir = process.argv[2], stage = process.argv[3];
assert(path.resolve(dir).includes('/nexofolio-step04/run.'));
const m = JSON.parse(fs.readFileSync(path.join(dir, 'browser.json')));
assert(m.db.startsWith('nexofolio_step04_'));
const args = ['-XqAt', '-v', 'ON_ERROR_STOP=1', '-h', '127.0.0.1', '-p', String(m.pg_port), '-d', m.db];
const lockChildren = new Set();
const sql = text => execFileSync(m.psql, args, { input: text, encoding: 'utf8' }).trim();
async function lock(query) {
  const child = spawn(m.psql, args, { stdio: ['pipe', 'pipe', 'pipe'] });
  lockChildren.add(child); child.on('exit', () => lockChildren.delete(child));
  await new Promise((resolve, reject) => {
    let output = '';
    child.stdout.on('data', d => { output += d; if (output.includes('LOCK_READY')) resolve(); });
    child.on('exit', code => { if (code) reject(new Error('fixture lock failed')); });
    child.stdin.write(`BEGIN; ${query}; SELECT 'LOCK_READY';\n`);
  });
  return () => { child.stdin.end('COMMIT;\n'); return new Promise(resolve => child.on('exit', resolve)); };
}
(async () => {
  const browser = await chromium.launch({ channel: 'chrome', headless: true });
  const context = await browser.newContext({ viewport: { width: 1512, height: 1050 } });
  await context.addInitScript(token => localStorage.setItem('nexofolio.session', JSON.stringify({ token, expiresAt: '2099-01-01T00:00:00Z' })), m.token);
  const page = await context.newPage(), errors = [], posts = [], checks = [], samples = [], reads = [];
  page.on('request', r => { if (r.method() === 'GET') reads.push(r.url()); });
  page.on('response', async r => { if (r.url().includes('/apply-status')) { try { const b = await r.json(); samples.push({ state: b.candidate?.state, operation: b.operation, progress: b.progress }); } catch {} } });
  page.setDefaultTimeout(20000);
  page.on('pageerror', e => errors.push(e.message));
  page.on('request', r => { if (r.method() === 'POST' && new URL(r.url()).pathname.endsWith('/apply')) posts.push(r.url()); });
  const url = `${m.front}/projects/${m.project}/interfaces/curation/${m.candidate}`;
  const screenshot = name => page.screenshot({ path: path.join(m.out, name), fullPage: true, animations: 'disabled' });
  const open = async () => { await page.goto(url); await page.locator('.candidate-tree-heading').waitFor(); };
  const confirm = async () => {
    await page.getByRole('button', { name: '本轮操作', exact: true }).click();
    await page.getByRole('menuitem', { name: '应用到现行文档', exact: true }).click();
    await page.getByRole('button', { name: '确认应用', exact: true }).click();
  };
  const reset = () => sql("DELETE FROM knowledge.field_notes;DELETE FROM knowledge.revisions;DELETE FROM knowledge.rounds;UPDATE knowledge.candidates SET state='ready',applied_rounds='[]',operations='[]';");
  try {
    const started = Date.now(); await open();
    const firstScreenMs = Date.now() - started;
    await page.waitForTimeout(150);
    const resources = await page.evaluate(() => performance.getEntriesByType('resource').filter(r => r.name.includes('/api/')).map(r => ({ url: new URL(r.name).pathname + new URL(r.name).search, bytes: r.decodedBodySize, duration: r.duration, ttfb: r.responseStart-r.requestStart })));
    if (['baseline', 'optimized'].includes(stage)) {
      await screenshot(`browser-${stage}.png`);
      if (stage === 'optimized') {
        assert(!resources.some(r => r.url.endsWith('/candidates/'+m.candidate)));
        assert(resources.find(r => r.url.endsWith('/review')).bytes < 100000);
        await page.getByRole('button', { name: '变化对比', exact: true }).click();
        await page.getByRole('button', { name: /字段变化/ }).click();
        await page.locator('.curation-change-list > button').nth(1).waitFor();
        assert.match(await page.locator('.curation-change-detail').innerText(), /合成字段/);
        await page.getByRole('button', { name: /继续显示/ }).click();
        await page.waitForFunction(() => document.querySelectorAll('.curation-change-list > button').length > 61);
        checks.push('paged diff bodies visible');
      }
      fs.writeFileSync(path.join(m.out, `browser-${stage}.json`), JSON.stringify({ firstScreenMs, resources, errors, checks }, null, 2));
      assert.deepEqual(errors, []); return;
    }
    sql("UPDATE knowledge.candidates SET state='running',updated_at=now()"); await open();
    await page.locator('.candidate-overview-entry').first().click();
    await page.locator('.document-field-group').first().waitFor();
    await page.getByRole('button', { name: '查看观测值', exact: true }).first().click();
    await page.getByText(/合成历史观测/).first().waitFor();
    await screenshot('browser-historical-evidence.png');
    const heads = () => reads.filter(x => new URL(x).pathname.endsWith('/review') && !new URL(x).search).length;
    const endpoints = () => reads.filter(x => x.includes('part=endpoint')).length;
    const beforeHeads = heads(), beforeEndpoints = endpoints();
    sql("UPDATE knowledge.candidates SET updated_at=now(),progress='{\"completed\":1}'");
    await page.waitForTimeout(5400);
    assert.equal(heads(), beforeHeads); assert.equal(endpoints(), beforeEndpoints);
    assert(!reads.some(x => new URL(x).pathname.endsWith('/candidates/'+m.candidate)));
    sql("UPDATE knowledge.candidates SET view=jsonb_set(view,'{fields,0,text}','\"changed content\"')");
    for (let n=0;n<30 && endpoints()===beforeEndpoints;n++) await page.waitForTimeout(200);
    assert.equal(heads(), beforeHeads + 1); assert.equal(endpoints(), beforeEndpoints + 1);
    sql("UPDATE knowledge.candidates SET state='ready',view=jsonb_set(view,'{fields,0,text}',batches#>'{0,commands,0,text}')");
    await open(); checks.push('historical v3 field evidence visible; heartbeat reads no head/body; content change reloads only selected interface');
    // Hold a real project advisory lock, then a real frozen-input table lock.
    const releaseProject = await lock(`SELECT pg_advisory_xact_lock(1802371073,hashtext('${m.project}'))`);
    const releaseFrozen = await lock('LOCK TABLE knowledge.frozen_input_chunks IN ACCESS EXCLUSIVE MODE');
    sql("CREATE FUNCTION knowledge.synthetic_slow() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(0.001);RETURN NEW;END $$;CREATE TRIGGER synthetic_slow BEFORE INSERT ON knowledge.field_notes FOR EACH ROW EXECUTE FUNCTION knowledge.synthetic_slow()");
    sql("CREATE FUNCTION knowledge.synthetic_commit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(1.5);RETURN NEW;END $$;CREATE CONSTRAINT TRIGGER synthetic_commit AFTER UPDATE ON knowledge.candidates DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN (NEW.state='applied') EXECUTE FUNCTION knowledge.synthetic_commit()");
    await confirm();
    await page.getByText('等待项目操作完成…', { exact: true }).waitFor();
    assert.equal(sql('SELECT count(*) FROM knowledge.field_notes'), '0');
    await screenshot('browser-apply-waiting.png'); await releaseProject();
    await page.getByText('正在校验候选、冻结证据与现行文档…', { exact: true }).waitFor();
    await releaseFrozen();
    await page.getByText(/正在写入变更：已处理/).waitFor();
    assert.equal(sql('SELECT count(*) FROM knowledge.field_notes'), '0');
    await screenshot('browser-apply-writing.png');
    await page.getByText('正在确认事务提交，变更尚未确认生效…', { exact: true }).waitFor();
    assert.equal(sql('SELECT count(*) FROM knowledge.field_notes'), '0');
    await screenshot('browser-apply-committing.png');
    await page.locator('.curation-state-tag').filter({ hasText: '已应用' }).waitFor();
    await page.getByRole('alertdialog').waitFor({ state: 'hidden' });
    assert.equal(sql('SELECT count(*) FROM knowledge.field_notes'), '2020');
    sql('DROP TRIGGER synthetic_slow ON knowledge.field_notes;DROP FUNCTION knowledge.synthetic_slow()');
    sql('DROP TRIGGER synthetic_commit ON knowledge.candidates;DROP FUNCTION knowledge.synthetic_commit()');
    checks.push('real lock waiting/validation/writing/COMMIT visible; deferred PG commit has zero visible rows until success');
    if (stage === 'commit') {
      fs.writeFileSync(path.join(m.out, 'browser-commit.json'), JSON.stringify({ checks, posts: posts.length, errors, samples }, null, 2));
      assert.deepEqual(errors, []); return;
    }
    // Successful POST plus failed list refresh must retain committed success.
    reset(); await open(); let denyRefresh = false;
    await page.route('**/curate/candidates', route => denyRefresh ? route.fulfill({ status: 503, json: { error: { message: 'synthetic refresh failure' } } }) : route.continue());
    await page.route('**/apply?*', async route => { const response = await route.fetch(); denyRefresh = true; await route.fulfill({ response }); });
    await confirm(); await page.getByText(/决定已确认保存，页面刷新失败/).waitFor();
    assert.equal(await page.locator('.curation-state-tag').innerText(), '已应用');
    checks.push('committed success survives refresh failure'); await page.unrouteAll({ behavior: 'wait' });
    // Drop only the response after the actual database commit. No replay is permitted.
    reset(); await open(); const before = posts.length;
    await page.route('**/apply?*', async route => { await route.fetch(); await route.abort('connectionreset'); });
    await confirm(); await page.locator('.curation-state-tag').filter({ hasText: '已应用' }).waitFor();
    await page.getByRole('alertdialog').waitFor({ state: 'hidden' });
    assert.equal(posts.length, before + 1); checks.push('lost response reconciles persisted state with one POST');
    await page.unrouteAll({ behavior: 'wait' });
    // Exercise the real ApiClient timeout branch with a short browser-only timer.
    reset(); await open(); const timeoutBefore = posts.length;
    sql("CREATE FUNCTION knowledge.synthetic_slow() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(0.001);RETURN NEW;END $$;CREATE TRIGGER synthetic_slow BEFORE INSERT ON knowledge.field_notes FOR EACH ROW EXECUTE FUNCTION knowledge.synthetic_slow()");
    await page.evaluate(() => { const original = window.setTimeout.bind(window); window.__restoreTimer = () => { window.setTimeout = original }; window.setTimeout = (fn, ms, ...args) => original(fn, ms === 190000 ? 80 : ms, ...args); });
    await confirm(); await page.locator('.curation-state-tag').filter({ hasText: '已应用' }).waitFor();
    await page.getByRole('alertdialog').waitFor({ state: 'hidden' });
    assert.equal(posts.length, timeoutBefore + 1);
    await page.evaluate(() => window.__restoreTimer());
    sql('DROP TRIGGER synthetic_slow ON knowledge.field_notes;DROP FUNCTION knowledge.synthetic_slow()');
    checks.push('actual client timeout branch reconciles ongoing real transaction; one POST');
    // Route switch while an old response is held cannot overwrite the next candidate.
    reset(); sql(`INSERT INTO knowledge.candidates(id,project_id,state,created_at,updated_at,base_hash,base,view,batches,progress,applied_rounds) SELECT 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',project_id,'discarded',created_at,updated_at,base_hash,base,view,batches,progress,'[]' FROM knowledge.candidates LIMIT 1`);
    await open(); let releaseReply; const held = new Promise(resolve => { releaseReply = resolve; });
    await page.route('**/apply?*', async route => { const response = await route.fetch(); await held; await route.fulfill({ response }).catch(() => {}); });
    await confirm();
    await page.waitForTimeout(350);
    await page.evaluate(next => { history.pushState({}, '', next); window.dispatchEvent(new PopStateEvent('popstate')); }, `/projects/${m.project}/interfaces/curation/aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa`);
    releaseReply(); await page.locator('.curation-state-tag').filter({ hasText: '已废弃' }).waitFor();
    await page.waitForTimeout(800); assert.equal(await page.locator('.curation-state-tag').innerText(), '已废弃');
    checks.push('candidate switch rejects late application response');
    fs.writeFileSync(path.join(m.out, 'browser-scenarios.json'), JSON.stringify({ checks, posts: posts.length, errors, samples }, null, 2));
    assert.deepEqual(errors, []);
  } catch (error) { await screenshot('browser-failure.png').catch(() => {}); fs.writeFileSync(path.join(m.out, 'browser-failure.json'), JSON.stringify({ message: error.message, dialog: await page.getByRole('alertdialog').innerText().catch(() => ''), samples, posts, errors }, null, 2)); throw error; } finally { for (const child of lockChildren) child.kill('SIGTERM'); await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
