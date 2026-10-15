const fs = require("node:fs");
const path = require("node:path");
const assert = require("node:assert/strict");
const { chromium } = require(
  process.env.PLAYWRIGHT_MODULE ||
    "/Users/sheldon/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright",
);
const dir = process.argv[2];
assert(dir && path.resolve(dir).startsWith("/tmp/nexofolio-step02/"));
const m = JSON.parse(fs.readFileSync(path.join(dir, "browser.json")));
const project = m.projects[0],
  endpoint = m.endpoints[0];
const root = `${m.front}/projects/${project}/interfaces/endpoints/${endpoint}?env=${m.environments[0]}`;
const mode = (value) =>
  fs.writeFileSync(
    path.join(dir, "control.json"),
    JSON.stringify({ mode: value }),
  );
(async () => {
  let ready = false;
  const started = Date.now();
  while (Date.now() - started < 10000) {
    try {
      const response = await fetch(m.front);
      await response.text();
      if (response.ok) {
        ready = true;
        break;
      }
    } catch {}
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  assert(ready, "isolated frontend must be ready before browser acceptance");
  const login = await fetch(m.api + "/v1/auth/login", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ account: "a80646", password: "fixture-pass" }),
  });
  assert(login.ok);
  const token = (await login.json()).token;
  const cards = await fetch(m.api + "/v1/projects", {
    headers: { Authorization: "Bearer " + token },
  }).then((r) => r.json());
  const projectName = cards.items.find(
    (item) => item.project_id === project,
  )?.name;
  assert(
    projectName,
    "select the actual scoped project, independent of database result order",
  );

  const context = await chromium.launchPersistentContext(
    path.join(dir, "chrome-profile"),
    {
      executablePath:
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
      headless: true,
      viewport: { width: 1440, height: 1100 },
      reducedMotion: "reduce",
    },
  );
  await context.grantPermissions(["clipboard-read", "clipboard-write"], {
    origin: m.front,
  });
  const page = context.pages()[0];
  page.setDefaultTimeout(20000);
  const errors = [],
    receipts = [],
    cases = [],
    pending = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("response", (r) => {
    if (r.request().method() === "POST" && r.url().includes("/curate"))
      pending.push(
        (async () => {
          let data;
          try {
            data = await r.json();
          } catch {}
          receipts.push({
            path: new URL(r.url()).pathname,
            status: r.status(),
            task_id: data?.task_id,
            task_state: data?.task_state,
            candidate_id: data?.candidate_id,
          });
        })(),
      );
  });
  const dock = page.locator(".curation-dock"),
    dialog = page.getByRole("alertdialog");
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
  async function submitEndpoint() {
    const [response] = await Promise.all([
      page.waitForResponse(
        (r) =>
          r.request().method() === "POST" &&
          r.url().endsWith(`/curate/describe/${endpoint}`),
      ),
      page.getByRole("button", { name: "整理接口", exact: true }).click(),
    ]);
    assert.equal(response.status(), 202);
    return response.json();
  }
  async function open() {
    await page.locator(".document-organize").click();
    await dialog.waitFor();
    await dialog
      .getByRole("button", {
        name: /开始新一轮整理|提交并查看任务|查看待处理任务/,
        exact: true,
      })
      .waitFor();
    await page.waitForFunction(
      () =>
        !document
          .querySelector('[role="alertdialog"]')
          ?.textContent.includes("正在核对当前范围"),
    );
  }
  async function accept(label) {
    const [response] = await Promise.all([
      page.waitForResponse(
        (r) => r.request().method() === "POST" && r.url().endsWith("/curate"),
      ),
      dialog.getByRole("button", { name: label, exact: true }).click(),
    ]);
    assert.equal(response.status(), 202);
    return response.json();
  }
  async function select(id) {
    await expand();
    await page
      .getByRole("button", { name: "切换活动任务", exact: true })
      .click();
    const [response] = await Promise.all([
      page.waitForResponse(
        (r) =>
          r.request().method() === "GET" &&
          new URL(r.url()).searchParams.get("task_id") === id,
      ),
      page
        .locator(".curation-task-option")
        .filter({ hasText: id === singleId ? "/retry-0" : "项目整体整理" })
        .click(),
    ]);
    assert.equal((await response.json()).task_id, id);
  }
  async function stop(id) {
    const [response] = await Promise.all([
      page.waitForResponse(
        (r) =>
          r.request().method() === "POST" &&
          r.url().endsWith(`/tasks/${id}/stop`),
      ),
      page.getByRole("button", { name: "停止本轮", exact: true }).click(),
    ]);
    assert.equal(response.status(), 204);
    await dock.getByText("任务已停止", { exact: true }).waitFor();
  }
  async function current(id, state, timeout = 30000) {
    return (
      await page.waitForResponse(
        async (r) => {
          if (r.request().method() !== "GET" || !r.url().includes("/curate"))
            return false;
          try {
            const d = await r.json();
            return d.task_id === id && d.task_state === state;
          } catch {
            return false;
          }
        },
        { timeout },
      )
    ).json();
  }
  async function diagnostic(id, status, taskOnly = false) {
    await page.goto(m.front + "/management/audit?tab=curation", {
      waitUntil: "networkidle",
    });
    await page.getByRole("combobox", { name: "项目", exact: true }).click();
    await page.getByRole("option", { name: projectName, exact: true }).click();
    const row = page
      .locator(".diagnostic-round")
      .filter({ hasText: id.slice(0, 8) });
    await row.waitFor();
    await row.click();
    await page
      .getByRole("button", { name: "一键复制排查资料", exact: true })
      .click();
    await page
      .getByText("排查资料已复制，可直接交给 Agent。", { exact: true })
      .waitFor();
    const text = await page.evaluate(() => navigator.clipboard.readText());
    const raw = text.match(/```json\n([\s\S]+)\n```/);
    assert(raw);
    const data = JSON.parse(raw[1]);
    assert.equal(data.subject.status, status);
    assert(data.task?.id);
    if (taskOnly) assert.equal(data.candidate, null);
    else assert.equal(data.candidate.id, id);
    assert(!JSON.stringify(data).includes("fixture-model-key"));
    assert(!Object.hasOwn(data, "execution_input"));
    fs.writeFileSync(
      path.join(
        dir,
        `copied-${status}-${taskOnly ? "task" : "candidate"}.json`,
      ),
      JSON.stringify(data, null, 2),
    );
    await page.screenshot({
      path: path.join(
        dir,
        `diagnostic-${status}-${taskOnly ? "task" : "candidate"}.png`,
      ),
    });
    await page.goto(root, { waitUntil: "networkidle" });
    return data;
  }
  let singleId;
  try {
    mode("hold");
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
      await page.getByLabel("账号", { exact: true }).fill("a80646");
      await page.getByLabel("密码", { exact: true }).fill("fixture-pass");
      await page.getByRole("checkbox", { name: "记住账号和密码" }).uncheck();
      await page.getByRole("button", { name: "登录并进入" }).click();
    }
    await page.getByRole("button", { name: "整理接口", exact: true }).waitFor();
    const single = await submitEndpoint();
    singleId = single.task_id;
    await current(singleId, "running");
    await open();
    assert.match(await dialog.innerText(), /其他范围有活动任务/);
    const queued = await accept("开始新一轮整理");
    assert.equal(queued.task_state, "queued");
    assert.equal(queued.candidate_id, null);
    cases.push({ state: "other_scope", task: queued.task_id });
    await open();
    assert.match(await dialog.innerText(), /当前范围正在排队/);
    assert.match(await dialog.innerText(), /尚未生成候选/);
    const reused = await accept("提交并查看任务");
    assert.equal(reused.task_id, queued.task_id);
    assert.equal(reused.task_state, "queued");
    cases.push({ state: "queued_without_candidate", task: queued.task_id });
    await select(singleId);
    await stop(singleId);
    await select(queued.task_id);
    await current(queued.task_id, "running");
    await open();
    assert.match(await dialog.innerText(), /当前范围正在执行/);
    const running = await accept("提交并查看任务");
    assert.equal(running.task_id, queued.task_id);
    cases.push({ state: "running", task: queued.task_id });
    await expand();
    await stop(queued.task_id);
    mode("organize503");
    await open();
    const waiting = await accept("开始新一轮整理");
    const saved = await current(waiting.task_id, "waiting", 45000);
    await open();
    assert.match(await dialog.innerText(), /当前范围正在等待上游恢复/);
    assert(!/开始新一轮则独立整理/.test(await dialog.innerText()));
    await page.screenshot({ path: path.join(dir, "confirmation-waiting.png") });
    const again = await accept("提交并查看任务");
    assert.equal(again.task_id, waiting.task_id);
    assert.equal(again.task_state, "waiting");
    cases.push({
      state: "waiting",
      task: waiting.task_id,
      candidate: saved.candidate_id,
    });
    const copied = await diagnostic(saved.candidate_id, "waiting");
    assert.equal(copied.task.id, waiting.task_id);
    assert(copied.task.next_retry_at);
    assert(
      copied.related_events.items.some(
        (e) =>
          e.action === "curation.suspended" &&
          e.execution?.task_id === waiting.task_id,
      ),
    );
    mode("ok");
    await current(waiting.task_id, "done", 120000);
    await open();
    assert.match(await dialog.innerText(), /开始新一轮整理/);
    const links = dialog.locator(".curation-confirm-resume-actions button");
    assert((await links.count()) >= 2);
    const boxes = await links.evaluateAll((nodes) =>
      nodes.map((n) => {
        const r = n.getBoundingClientRect();
        return { left: r.left, right: r.right, top: r.top };
      }),
    );
    for (let i = 1; i < boxes.length; i++)
      assert(
        boxes[i].top > boxes[i - 1].top ||
          boxes[i].left - boxes[i - 1].right >= 10,
      );
    await page.screenshot({
      path: path.join(dir, "confirmation-new-and-resume-layout.png"),
    });
    await dialog.getByRole("button", { name: "取消", exact: true }).click();
    mode("401");
    await open();
    const attention = await accept("开始新一轮整理");
    const attentionProgress = await current(
      attention.task_id,
      "needs_attention",
    );
    await open();
    assert.match(await dialog.innerText(), /当前范围的任务需要处理/);
    const before = receipts.length;
    await dialog
      .getByRole("button", { name: "查看待处理任务", exact: true })
      .click();
    await dock.getByText("需要处理", { exact: true }).waitFor();
    await Promise.all(pending);
    assert.equal(receipts.length, before);
    cases.push({
      state: "needs_attention",
      task: attention.task_id,
      primary_mutations: 0,
    });
    await diagnostic(attentionProgress.candidate_id, "needs_attention");
    await expand();
    await stop(attention.task_id);
    const seed = { id: "no-candidate-" + Date.now(), action: "seed_attention" };
    fs.writeFileSync(path.join(dir, "command.json"), JSON.stringify(seed));
    let seeded;
    for (let i = 0; i < 100; i++) {
      try {
        const v = JSON.parse(
          fs.readFileSync(path.join(dir, "command-result.json")),
        );
        if (v.id === seed.id) {
          seeded = v.task;
          break;
        }
      } catch {}
      await new Promise((r) => setTimeout(r, 100));
    }
    assert(seeded && !seeded.candidate_id);
    await open();
    assert.match(await dialog.innerText(), /尚未生成候选/);
    const beforeView = receipts.length;
    await dialog
      .getByRole("button", { name: "查看待处理任务", exact: true })
      .click();
    await dock.getByText("需要处理", { exact: true }).waitFor();
    await Promise.all(pending);
    assert.equal(receipts.length, beforeView);
    await diagnostic(seeded.id, "needs_attention", true);
    await open();
    await dialog
      .getByRole("button", { name: "查看待处理任务", exact: true })
      .click();
    await dock.getByText("需要处理", { exact: true }).waitFor();
    await expand();
    await stop(seeded.id);
    cases.push({
      state: "needs_attention_without_candidate",
      task: seeded.id,
      primary_mutations: 0,
    });
    // An explicit repair submission of a task without a candidate keeps its task identity.
    const seedAgain = { id: "repair-" + Date.now(), action: "seed_attention" };
    fs.writeFileSync(path.join(dir, "command.json"), JSON.stringify(seedAgain));
    let repair;
    for (let i = 0; i < 100; i++) {
      try {
        const v = JSON.parse(
          fs.readFileSync(path.join(dir, "command-result.json")),
        );
        if (v.id === seedAgain.id) {
          repair = v.task;
          break;
        }
      } catch {}
      await new Promise((r) => setTimeout(r, 100));
    }
    assert(repair);
    mode("ok");
    await open();
    const repaired = await accept("修复后重新提交");
    assert.equal(repaired.task_id, repair.id);
    await current(repair.id, "done", 45000);
    cases.push({ state: "explicit_repair_without_candidate", task: repair.id });
    // Drop a real accepted response: admission must still be discovered without replaying its POST.
    mode("hold");
    let posts = 0,
      uncertainTask;
    const matcher = `**/api/v1/projects/${project}/curate`;
    await page.route(matcher, async (route) => {
      if (route.request().method() !== "POST") return route.continue();
      posts++;
      const response = await route.fetch();
      assert.equal(response.status(), 202);
      uncertainTask = await response.json();
      await route.abort("failed");
    });
    await open();
    await dialog
      .getByRole("button", { name: "开始新一轮整理", exact: true })
      .evaluate((button) => {
        button.click();
        button.click();
      });
    await page.waitForResponse(async (r) => {
      if (r.request().method() !== "GET" || !r.url().includes("/curate"))
        return false;
      try {
        return (
          (await r.json()).task_id === uncertainTask?.task_id && !!uncertainTask
        );
      } catch {
        return false;
      }
    });
    await page.unroute(matcher);
    assert.equal(posts, 1);
    await page.waitForFunction(
      () =>
        document
          .querySelector(".curation-dock")
          ?.textContent.includes("知识正在成长") ||
        document
          .querySelector(".curation-dock")
          ?.textContent.includes("正在整理文档"),
    );
    await expand();
    const stopResponse = page.waitForResponse(
      (r) =>
        r.request().method() === "POST" &&
        r.url().includes("/tasks/") &&
        r.url().endsWith("/stop"),
    );
    await page.getByRole("button", { name: "停止本轮", exact: true }).click();
    assert.equal((await stopResponse).status(), 204);
    cases.push({
      state: "uncertain_response_and_double_click",
      actual_posts: posts,
    });
    await Promise.all(pending);
    assert.deepEqual(errors, []);
    fs.writeFileSync(
      path.join(dir, "browser-results.json"),
      JSON.stringify({ passed: true, cases, receipts, errors }, null, 2),
    );
    console.log("All actual browser/API/PG fixes acceptance cases passed");
  } catch (e) {
    await page.screenshot({ path: path.join(dir, "browser-failure.png") });
    fs.writeFileSync(
      path.join(dir, "browser-failure.txt"),
      await page.locator("body").innerText(),
    );
    throw e;
  } finally {
    await context.close();
  }
})().catch((e) => {
  console.error(e);
  process.exitCode = 1;
});
