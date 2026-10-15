const fs = require("node:fs"),
  path = require("node:path"),
  assert = require("node:assert/strict"),
  crypto = require("node:crypto"),
  { execFileSync } = require("node:child_process");
const {
  chromium,
} = require("/Users/sheldon/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright");
const dir = process.argv[2];
assert(path.resolve(dir).startsWith("/tmp/nexofolio-operations/browser."));
const m = JSON.parse(fs.readFileSync(path.join(dir, "browser.json")));
const sql = (q) =>
  execFileSync(
    "/opt/homebrew/opt/postgresql@17/bin/psql",
    [
      "-h",
      "127.0.0.1",
      "-p",
      String(m.pg_port),
      "-d",
      m.database,
      "-At",
      "-c",
      q,
    ],
    { encoding: "utf8" },
  ).trim();
const states = {
  queued: "排队中",
  running: "运行中",
  waiting: "等待恢复",
  needs_attention: "需要处理",
  failed: "执行失败",
  stopped: "已停止",
};
const ids = {};
const inserts = [];
assert.equal(m.database, "nexofolio_operations_browser");
assert(/^[\da-f-]{36}$/.test(m.projects[0]));
for (const state of Object.keys(states)) {
  const id = (ids[state] = crypto.randomUUID());
  inserts.push(
    `INSERT INTO knowledge.curation_tasks(id,project_id,target,actor,state,run_id,generation,lease_until,next_retry_at,retry_reason,created_at,updated_at) VALUES('${id}','${m.projects[0]}','${crypto.randomUUID()}','{"kind":"unknown"}','${state}','${crypto.randomUUID()}',1,now()+interval '1 day',${state === "waiting" ? "now()+interval '1 day'" : "NULL"},${state === "waiting" ? "'model_unavailable'" : state === "needs_attention" ? "'model_configuration'" : "NULL"},now(),now())`,
  );
}
sql("BEGIN;" + inserts.join(";") + ";COMMIT;");
(async () => {
  const login = await fetch(m.api + "/v1/auth/login", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ account: "a80646", password: "fixture-pass" }),
  }).then((r) => r.json());
  const context = await chromium.launchPersistentContext(
    path.join(dir, "states-profile"),
    {
      executablePath:
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
      headless: true,
      viewport: { width: 1512, height: 1100 },
      reducedMotion: "reduce",
    },
  );
  await context.grantPermissions(["clipboard-read", "clipboard-write"], {
    origin: m.front,
  });
  await context.addInitScript(
    (v) =>
      localStorage.setItem(
        "nexofolio.session",
        JSON.stringify({ token: v.token, expiresAt: v.expires_at }),
      ),
    login,
  );
  const page = context.pages()[0];
  page.setDefaultTimeout(15000);
  const evidence = [];
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  try {
    await page.goto(m.front + "/management/operations");
    await page.locator("[data-task-id]").first().waitFor();
    for (const [state, label] of Object.entries(states)) {
      await page
        .getByRole("combobox", { name: "任务状态", exact: true })
        .click();
      await page.getByRole("option", { name: label, exact: true }).click();
      const row = page.locator(`[data-task-id="${ids[state]}"]`);
      await row.waitFor();
      assert.match(await row.innerText(), new RegExp(label));
      await row.getByRole("button", { name: "查看诊断" }).click();
      await page.getByRole("button", { name: "一键复制排查资料" }).click();
      await page
        .getByText("排查资料已复制，可直接交给 Agent。", { exact: true })
        .waitFor();
      const text = await page.evaluate(() => navigator.clipboard.readText());
      const value = JSON.parse(text.match(/```json\n([\s\S]+)\n```/)[1]);
      assert.equal(value.task.id, ids[state]);
      assert.equal(value.task.state, state);
      assert.equal(value.candidate, null);
      assert.equal(value.subject.stage, null);
      evidence.push({
        state,
        id: ids[state],
        source:
          "persisted synthetic fixture; runtime transitions separately exercised by API tests",
        package: value,
      });
      if (["waiting", "running", "needs_attention"].includes(state))
        await page.screenshot({
          path: path.join(dir, "state-" + state + ".png"),
          fullPage: true,
        });
      await page.getByRole("button", { name: "关闭弹窗", exact: true }).click();
    }
    await page.getByRole("combobox", { name: "任务状态", exact: true }).click();
    await page.getByRole("option", { name: "全部状态", exact: true }).click();
    const other = sql(
      `SELECT name FROM access.projects WHERE id='${m.projects[1]}'`,
    );
    await page.getByRole("combobox", { name: "项目", exact: true }).click();
    await page.getByRole("option", { name: other, exact: true }).click();
    await page.getByText("当前筛选下没有任务", { exact: true }).waitFor();
    evidence.push({
      case: "project_filter",
      project: m.projects[1],
      expected_rows: 0,
    });
    const frozen = await fetch(m.api + "/v1/operations/summary", {
      headers: { Authorization: "Bearer " + login.token },
    }).then((r) => r.json());
    await page.route("**/v1/operations/summary", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(frozen),
      }),
    );
    await page.waitForTimeout(22000);
    assert(
      (await page.locator(".ops-overview").innerText()).includes("已过期"),
    );
    await page.screenshot({
      path: path.join(dir, "stale-snapshot.png"),
      fullPage: true,
    });
    evidence.push({
      case: "real_snapshot_replay_ages_out",
      sampled_at: frozen.snapshot.model.sampled_at,
    });
    await page.unroute("**/v1/operations/summary");
    await page.getByRole("button", { name: "刷新状态", exact: true }).click();
    const inconsistent = structuredClone(frozen);
    inconsistent.snapshot.pools.value[0].size = 0;
    inconsistent.snapshot.pools.value[0].idle = 1;
    await page.route("**/v1/operations/summary", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(inconsistent),
      }),
    );
    await page.getByRole("button", { name: "刷新状态", exact: true }).click();
    await page.getByRole("tab", { name: "模型与资源" }).click();
    const use = page
      .locator(".ops-resource-grid table tbody tr")
      .first()
      .locator("td")
      .nth(2);
    await use.getByText("未采集 / 1", { exact: true }).waitFor();
    evidence.push({
      case: "inconsistent_non_atomic_pool_counter_is_unknown",
      source: "deliberately injected response edge case",
    });
    await page.unroute("**/v1/operations/summary");
    sql("UPDATE access.users SET enabled=false WHERE lower(account)='a80646'");
    // Either in-flight completion or the single polling clock may revoke access first.
    await page.waitForFunction(
      () => !location.pathname.startsWith("/management"),
    );
    assert.equal(await page.locator(".operations-panel").count(), 0);
    await page.screenshot({
      path: path.join(dir, "disabled-manager.png"),
      fullPage: true,
    });
    evidence.push({ case: "disabled_manager_revoked_live" });
    assert.equal(errors.length, 0);
    fs.writeFileSync(
      path.join(dir, "browser-states-results.json"),
      JSON.stringify({ evidence, errors }, null, 2),
    );
  } catch (e) {
    await page
      .screenshot({
        path: path.join(dir, "states-failure.png"),
        fullPage: true,
      })
      .catch(() => {});
    console.error(e);
    process.exitCode = 1;
  } finally {
    sql(
      `DELETE FROM knowledge.curation_tasks WHERE id IN (${Object.values(ids)
        .map((id) => `'${id}'`)
        .join(",")})`,
    );
    sql("UPDATE access.users SET enabled=true WHERE lower(account)='a80646'");
    await context.close();
  }
})().catch((e) => {
  console.error(e);
  process.exitCode = 1;
});
