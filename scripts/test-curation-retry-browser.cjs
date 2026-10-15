// Actual isolated Chrome, Vite, API and PostgreSQL. Fixture manifest contains no credentials.
const fs = require("node:fs");
const path = require("node:path");
const assert = require("node:assert/strict");
const { chromium } = require(
  process.env.PLAYWRIGHT_MODULE ||
    "/Users/sheldon/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright",
);
const dir = process.argv[2];
assert(dir && path.resolve(dir).startsWith("/tmp/nexofolio-step02"));
const m = JSON.parse(fs.readFileSync(path.join(dir, "browser.json"), "utf8"));
const mode = (value) =>
  fs.writeFileSync(
    path.join(dir, "control.json"),
    JSON.stringify({ mode: value }),
  );
(async () => {
  mode("503");
  const context = await chromium.launchPersistentContext(
    path.join(dir, "chrome-profile"),
    {
      executablePath:
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
      headless: true,
      viewport: { width: 1440, height: 1050 },
      reducedMotion: "reduce",
    },
  );
  const page = context.pages()[0];
  page.setDefaultTimeout(25000);
  const errors = [],
    receipts = [],
    progress = [],
    pending = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("response", (r) => {
    if (r.url().includes("/curate"))
      pending.push(
        (async () => {
          let body;
          try {
            body = await r.json();
          } catch {}
          const safe = {
            path: new URL(r.url()).pathname,
            at: new Date().toISOString(),
            http: r.status(),
            task_id: body?.task_id,
            candidate_id: body?.candidate_id,
            task_state: body?.task_state,
            next_retry_at: body?.next_retry_at,
          };
          if (r.request().method() === "POST") receipts.push(safe);
          else if (body?.task_id) progress.push(safe);
        })(),
      );
  });
  const project = m.projects[0],
    endpoint = m.endpoints[0];
  const root = `${m.front}/projects/${project}/interfaces/endpoints/${endpoint}?env=${m.environments[0]}`;
  const dock = page.locator(".curation-dock");
  async function expand() {
    await dock.waitFor();
    if ((await dock.getAttribute("data-size")) === "mini")
      await page
        .getByRole("button", { name: "展开进度摘要", exact: true })
        .click();
    if ((await dock.getAttribute("data-size")) === "compact")
      await page
        .getByRole("button", { name: "展开完整进度", exact: true })
        .click();
  }
  async function pick(text, id) {
    await expand();
    await page
      .getByRole("button", { name: "切换活动任务", exact: true })
      .click();
    const response = page.waitForResponse(
      (r) =>
        r.request().method() === "GET" &&
        new URL(r.url()).searchParams.get("task_id") === id,
    );
    await page
      .locator(".curation-task-option")
      .filter({ hasText: text })
      .click();
    assert.equal((await (await response).json()).task_id, id);
  }
  async function start() {
    const [r] = await Promise.all([
      page.waitForResponse(
        (r) =>
          r.request().method() === "POST" &&
          r.url().endsWith(`/curate/describe/${endpoint}`),
      ),
      page.getByRole("button", { name: "整理接口", exact: true }).click(),
    ]);
    assert.equal(r.status(), 202);
    return r.json();
  }
  async function stop(id) {
    const [r] = await Promise.all([
      page.waitForResponse(
        (r) =>
          r.request().method() === "POST" &&
          r.url().endsWith(`/tasks/${id}/stop`),
      ),
      page.getByRole("button", { name: "停止本轮", exact: true }).click(),
    ]);
    assert.equal(r.status(), 204);
    await dock.getByText("任务已停止", { exact: true }).waitFor();
  }
  try {
    await page.goto(root, { waitUntil: "networkidle" });
    await page.waitForFunction(
      () =>
        document.querySelector("button.workspace-enter") ||
        document.querySelector(".document-describe"),
    );
    if (await page.locator("button.workspace-enter").count()) {
      await page
        .getByRole("button", { name: "登录", exact: true })
        .first()
        .click();
      await page.getByLabel("账号", { exact: true }).fill("operator");
      await page.getByLabel("密码", { exact: true }).fill("fixture-pass");
      await page.getByRole("checkbox", { name: "记住账号和密码" }).uncheck();
      await page.getByRole("button", { name: "登录并进入" }).click();
    }
    await page.getByRole("button", { name: "整理接口", exact: true }).waitFor();
    const A = await start();
    await page.waitForResponse(
      async (r) => {
        if (r.request().method() !== "GET" || !r.url().includes("/curate"))
          return false;
        try {
          const body = await r.json();
          return (
            body.task_id === A.task_id &&
            body.task_state === "waiting" &&
            body.fields_updated >= 3
          );
        } catch {
          return false;
        }
      },
      { timeout: 120000 },
    );
    await dock.getByText("等待恢复 · 将自动继续", { exact: true }).waitFor();
    await expand();
    assert.match(await dock.innerText(), /下次尝试/);
    assert.match(await dock.innerText(), /排队 .*执行 .*等待 /);
    assert.equal(await dock.locator(".is-running").count(), 0);
    await page.screenshot({ path: path.join(dir, "browser-waiting.png") });
    const review = await dock
      .locator("a")
      .filter({ hasText: "查看已生成内容" })
      .getAttribute("href");
    assert(review);
    await dock.locator("a").filter({ hasText: "查看已生成内容" }).click();
    await page.getByText("合成恢复接口", { exact: true }).first().click();
    await page
      .getByText("可核查的合成资料说明", { exact: false })
      .first()
      .waitFor();
    await page.screenshot({
      path: path.join(dir, "browser-saved-results.png"),
    });
    await page.goto(root, { waitUntil: "networkidle" });
    assert(await page.locator(".document-organize").isEnabled());
    await page.locator(".document-organize").click();
    const [accepted] = await Promise.all([
      page.waitForResponse(
        (r) => r.request().method() === "POST" && r.url().endsWith("/curate"),
      ),
      page
        .getByRole("alertdialog")
        .getByRole("button", { name: /提交并查看任务|开始新一轮整理/ })
        .click(),
    ]);
    assert.equal(accepted.status(), 202);
    const B = await accepted.json();
    assert.equal(B.task_state, "queued");
    await pick("项目整体整理", B.task_id);
    await dock.getByText("任务已接受，正在排队", { exact: true }).waitFor();
    await page.screenshot({ path: path.join(dir, "browser-queued.png") });
    await stop(B.task_id);
    await pick("/retry-0", A.task_id);
    await dock.getByText("等待恢复 · 将自动继续", { exact: true }).waitFor();
    mode("ok");
    await dock
      .getByText("本轮整理完成", { exact: true })
      .waitFor({ timeout: 110000 });
    await page.screenshot({ path: path.join(dir, "browser-recovered.png") });
    mode("invalid");
    const C = await start();
    await dock
      .getByText("需要处理", { exact: true })
      .waitFor({ timeout: 30000 });
    await expand();
    await page.screenshot({ path: path.join(dir, "browser-attention.png") });
    assert(
      await page
        .getByRole("button", { name: "续跑本轮", exact: true })
        .isEnabled(),
    );
    await stop(C.task_id);
    await page.screenshot({
      path: path.join(dir, "browser-attention-stopped.png"),
    });
    await Promise.all(pending);
    assert.deepEqual(errors, []);
    const same = progress.filter(
      (p) => p.task_id === A.task_id && p.candidate_id,
    );
    assert.equal(new Set(same.map((p) => p.candidate_id)).size, 1);
    assert(same.some((p) => p.task_state === "waiting"));
    assert(same.some((p) => p.task_state === "done"));
    fs.writeFileSync(
      path.join(dir, "browser-results.json"),
      JSON.stringify(
        {
          passed: true,
          errors,
          receipts,
          progress,
          tasks: { A: A.task_id, B: B.task_id, C: C.task_id },
        },
        null,
        2,
      ),
    );
    console.log("browser acceptance passed");
  } catch (error) {
    await page.screenshot({ path: path.join(dir, "browser-failure.png") });
    fs.writeFileSync(
      path.join(dir, "browser-failure.txt"),
      await page.locator("body").innerText(),
    );
    throw error;
  } finally {
    await context.close();
  }
})().catch((e) => {
  console.error(e);
  process.exitCode = 1;
});
