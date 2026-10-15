const fs = require("node:fs");
const path = require("node:path");
const assert = require("node:assert/strict");
const { chromium } = require(
  process.env.PLAYWRIGHT_MODULE ||
    "/Users/sheldon/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright",
);
const dir = process.argv[2];
assert(path.resolve(dir).startsWith("/tmp/nexofolio-operations/browser."));
const m = JSON.parse(fs.readFileSync(path.join(dir, "browser.json")));
(async () => {
  const net = require("node:net");
  const port = await new Promise((resolve) => {
    const server = net.createServer();
    server.listen(0, "127.0.0.1", () => {
      const port = server.address().port;
      server.close(() => resolve(port));
    });
  });
  const chrome = require("node:child_process").spawn(
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    [
      `--user-data-dir=${path.join(dir, "chrome-profile")}`,
      `--remote-debugging-port=${port}`,
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-sync",
      "--window-size=1512,1200",
      "about:blank",
    ],
    { stdio: "ignore" },
  );
  let browser;
  for (let i = 0; i < 100; i++) {
    try {
      browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, {
        noDefaults: true,
        timeout: 1000,
      });
      break;
    } catch {
      await new Promise((r) => setTimeout(r, 100));
    }
  }
  assert(browser, "isolated native Chrome must start");
  const context = browser.contexts()[0];

  await context.grantPermissions(["clipboard-read", "clipboard-write"], {
    origin: m.front,
  });
  const page = context.pages()[0];
  await page.setViewportSize({ width: 1512, height: 1100 });
  page.setDefaultTimeout(15000);
  const errors = [],
    requests = [],
    cases = [];
  page.on("pageerror", (e) => errors.push(e.message));
  const authentication = [];
  page.on("response", (r) => {
    if (r.url().includes("/v1/auth/"))
      authentication.push({
        path: new URL(r.url()).pathname,
        status: r.status(),
      });
  });
  page.on("request", (r) => {
    if (r.url().includes("/v1/operations/"))
      requests.push({
        at: Date.now(),
        method: r.method(),
        path: new URL(r.url()).pathname,
        query: new URL(r.url()).search,
      });
  });
  async function shot(name) {
    await page.waitForTimeout(1200);
    await page.screenshot({
      path: path.join(dir, name + ".png"),
      fullPage: true,
    });
  }
  const auth = async (account) => {
    const value = await fetch(m.api + "/v1/auth/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ account, password: "fixture-pass" }),
    }).then((r) => r.json());
    assert(value.token);
    return value;
  };
  try {
    await page.goto(`${m.front}/projects/${m.projects[0]}/interfaces`);
    await page.waitForFunction(
      () =>
        document.querySelector("button.workspace-enter") ||
        document.querySelector(".management-entry"),
    );
    if (await page.locator("button.workspace-enter").count()) {
      await page
        .getByRole("button", { name: "登录", exact: true })
        .first()
        .click();
      await page.getByLabel("账号", { exact: true }).fill("A80646");
      await page.getByLabel("密码", { exact: true }).fill("fixture-pass");
      await page.getByRole("checkbox", { name: "记住账号和密码" }).uncheck();
      await page.getByRole("button", { name: "登录并进入" }).click();
    }
    await page.getByRole("link", { name: "管理中心", exact: true }).waitFor();
    await page.getByRole("link", { name: "管理中心", exact: true }).click();
    await page.getByRole("link", { name: "运行状态", exact: true }).click();
    await page.locator("[data-task-id]").first().waitFor();
    assert.equal(await page.locator("[data-task-id]").count(), 20);
    await shot("tasks");
    cases.push("menu_and_first_page");
    const first = await page
      .locator("[data-task-id]")
      .first()
      .getAttribute("data-task-id");
    await page.getByRole("button", { name: "下一页", exact: true }).click();
    await page.waitForFunction(() =>
      document
        .querySelector(".management-pagination")
        ?.textContent.includes("第 2 页"),
    );
    await page.waitForFunction(
      (id) =>
        document
          .querySelector("[data-task-id]")
          ?.getAttribute("data-task-id") !== id,
      first,
    );
    await page.waitForFunction(
      () => document.querySelectorAll("[data-task-id]").length === 8,
    );
    assert.equal(await page.locator("[data-task-id]").count(), 8);
    await page.getByRole("button", { name: "上一页", exact: true }).click();
    await page.waitForFunction(
      (id) =>
        document
          .querySelector("[data-task-id]")
          ?.getAttribute("data-task-id") === id,
      first,
    );
    cases.push("stable_cursor_paging");
    await page.getByRole("combobox", { name: "任务状态", exact: true }).click();
    await page.getByRole("option", { name: "已停止", exact: true }).click();
    await page.waitForFunction(
      () =>
        [...document.querySelectorAll(".ops-status")].every(
          (e) => e.textContent === "已停止",
        ) && document.querySelectorAll(".ops-status").length > 0,
    );
    assert.equal(await page.locator("[data-task-id]").count(), 13);
    await shot("filtered-stopped");
    cases.push("state_filter");
    await page.getByRole("combobox", { name: "任务状态", exact: true }).click();
    await page.getByRole("option", { name: "排队中", exact: true }).click();
    await page.getByText("当前筛选下没有任务", { exact: true }).waitFor();
    cases.push("empty_filter");
    await page.getByRole("combobox", { name: "任务状态", exact: true }).click();
    await page.getByRole("option", { name: "全部状态", exact: true }).click();
    await page.locator(`[data-task-id="${m.task}"]`).waitFor();
    await page
      .locator(`[data-task-id="${m.task}"]`)
      .getByRole("button", { name: "查看诊断" })
      .click();
    await page.getByRole("button", { name: "一键复制排查资料" }).waitFor();
    await page.getByRole("button", { name: "一键复制排查资料" }).click();
    const clipboard = await page.evaluate(() => navigator.clipboard.readText());
    const match = clipboard.match(/```json\n([\s\S]+)\n```/);
    assert(match);
    const diag = JSON.parse(match[1]);
    assert.equal(diag.schema_version, 2);
    assert.equal(diag.task.id, m.task);
    fs.writeFileSync(
      path.join(dir, "copied-diagnostic.json"),
      JSON.stringify(diag, null, 2),
    );
    await shot("diagnostic");
    await page.getByRole("button", { name: "关闭弹窗", exact: true }).click();
    cases.push("actual_clipboard_schema2_identity");
    await page.getByRole("tab", { name: "模型与资源" }).click();
    await page.getByRole("heading", { name: "数据库连接池" }).waitFor();
    await shot("resources");
    await page
      .locator(".operations-panel")
      .evaluate((el) => (el.scrollTop = el.scrollHeight));
    await shot("resources-lower");
    await page
      .locator(".operations-panel")
      .evaluate((el) => (el.scrollTop = 0));
    cases.push("resources_and_unknown");
    const n = requests.length;
    await page.getByRole("button", { name: "刷新状态", exact: true }).click();
    await page.waitForTimeout(500);
    assert(requests.length >= n + 2);
    cases.push("manual_refresh");
    const visibilitySession = await context.newCDPSession(page);
    await visibilitySession.send("Emulation.setFocusEmulationEnabled", {
      enabled: false,
    });
    const other = await context.newPage();
    await other.goto("about:blank");
    await other.bringToFront();
    await page.waitForFunction(() => document.hidden === true);
    await page.waitForTimeout(700);
    const hiddenStart = requests.length;
    await page.waitForTimeout(12000);
    assert.equal(requests.length, hiddenStart);
    await page.bringToFront();
    await page.waitForFunction(() => document.hidden === false);
    await page.waitForTimeout(800);
    assert(requests.length > hiddenStart);
    await other.close();
    cases.push("genuine_hidden_tab_zero_requests_and_visible_wake");
    await context.setOffline(true);
    await page
      .getByText("网络已断开，显示上次读取的快照。连接恢复后自动更新。")
      .waitFor();
    const offlineStart = requests.length;
    await page.waitForTimeout(6000);
    assert.equal(requests.length, offlineStart);
    await page.bringToFront();
    await context.setOffline(false);
    await page.waitForFunction(() => navigator.onLine && !document.hidden);
    for (let i = 0; i < 30 && requests.length === offlineStart; i++)
      await page.waitForTimeout(100);
    assert(requests.length > offlineStart);
    cases.push("offline_online_recovery");
    await page.route("**/v1/operations/summary", (route) =>
      route.fulfill({
        status: 503,
        contentType: "application/json",
        body: '{"error":{"message":"fixture unavailable"}}',
      }),
    );
    await page.getByRole("button", { name: "刷新状态", exact: true }).click();
    await page.getByRole("alert").waitFor();
    await shot("read-error");
    await page.unroute("**/v1/operations/summary");
    await page.getByRole("button", { name: "重新读取摘要", exact: true }).click();
    await page.waitForFunction(
      () => !document.querySelector('.ops-alert[role="alert"]'),
    );
    cases.push("read_failure_and_recovery");
    await page.getByRole("tab", { name: "任务与队列" }).click();
    let held;
    await page.route("**/v1/operations/tasks?*", async (route) => {
      if (route.request().url().includes("state=done") && !held) {
        const response = await route.fetch();
        held = { route, response };
        return;
      }
      await route.continue();
    });
    await page.getByRole("combobox", { name: "任务状态", exact: true }).click();
    await page.getByRole("option", { name: "已完成", exact: true }).click();
    for (let i = 0; i < 30 && !held; i++) await page.waitForTimeout(50);
    assert(held);
    await page.getByRole("combobox", { name: "任务状态", exact: true }).click();
    await page.getByRole("option", { name: "已停止", exact: true }).click();
    await page.waitForFunction(
      () => document.querySelector(".ops-status")?.textContent === "已停止",
    );
    await held.route.fulfill({ response: held.response }).catch(() => {});
    await page.waitForTimeout(300);
    assert(
      (await page.locator(".ops-status").allTextContents()).every(
        (x) => x === "已停止",
      ),
    );
    await page.unroute("**/v1/operations/tasks?*");
    cases.push("late_filter_response_isolated");
    // A separate authenticated ordinary user sees the unchanged workspace geometry and no entry.
    const ordinary = await auth("operator");
    const userPage = await context.newPage();
    await userPage.goto(m.front);
    await userPage.evaluate(
      (v) =>
        localStorage.setItem(
          "nexofolio.session",
          JSON.stringify({ token: v.token, expiresAt: v.expires_at }),
        ),
      ordinary,
    );
    await userPage.goto(m.front + "/projects");
    await userPage.waitForTimeout(500);
    assert.equal(
      await userPage
        .getByRole("link", { name: "运行状态", exact: true })
        .count(),
      0,
    );
    assert.equal(await userPage.locator(".management-entry").count(), 0);
    assert.equal(await userPage.locator(".management-sidebar").count(), 0);
    await userPage.screenshot({
      path: path.join(dir, "ordinary-workspace.png"),
      fullPage: true,
    });
    await userPage.goto(m.front + "/management/operations");
    await userPage.waitForURL("**/projects");
    cases.push("ordinary_entry_and_direct_route_hidden");
    assert.equal(errors.length, 0);
    assert(requests.every((r) => r.method === "GET"));
    fs.writeFileSync(
      path.join(dir, "browser-results.json"),
      JSON.stringify(
        { cases, errors, requests, hidden_request_delta: 0 },
        null,
        2,
      ),
    );
  } catch (e) {
    await shot("failure").catch(() => {});
    fs.writeFileSync(
      path.join(dir, "failure.txt"),
      await page
        .locator("body")
        .innerText()
        .catch(() => ""),
    );
    throw e;
  } finally {
    await browser.close();
    chrome.kill();
  }
})().catch((e) => {
  console.error(e);
  process.exitCode = 1;
});
