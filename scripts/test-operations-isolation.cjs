// Real private Chrome/API; only the selected GET response is fault-injected.
const fs = require("node:fs"),
  path = require("node:path"),
  assert = require("node:assert/strict");
const { chromium } = require(
  process.env.PLAYWRIGHT_MODULE ||
    "/Users/sheldon/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright",
);
const out = path.resolve(process.argv[2]);
assert(
  out.startsWith("/tmp/nexofolio-operations/browser.") ||
    out.startsWith("/private/tmp/nexofolio-operations/browser."),
);
const m = JSON.parse(fs.readFileSync(path.join(out, "browser.json")));
assert.equal(m.database, "nexofolio_operations_browser");
assert.equal(new URL(m.api).hostname, "127.0.0.1");
(async () => {
  const login = await fetch(m.api + "/v1/auth/login", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ account: "a80646", password: "fixture-pass" }),
  }).then((r) => r.json());
  const browser = await chromium.launch({
    executablePath:
      "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    headless: true,
  });
  const cases = [];
  try {
    for (const failed of ["tasks", "summary"])
      for (const initial of [true, false]) {
        const context = await browser.newContext({
          viewport: { width: 1512, height: 1100 },
          reducedMotion: "reduce",
        });
        await context.addInitScript(
          (v) =>
            localStorage.setItem(
              "nexofolio.session",
              JSON.stringify({ token: v.token, expiresAt: v.expires_at }),
            ),
          login,
        );
        const page = await context.newPage();
        page.setDefaultTimeout(15000);
        const responses = [],
          errors = [];
        let blocked = initial;
        page.on("pageerror", (e) => errors.push(e.message));
        page.on("response", async (r) => {
          if (!r.url().includes("/v1/operations/")) return;
          try {
            const body = await r.json();
            responses.push({
              received_at: Date.now(),
              path: new URL(r.url()).pathname,
              status: r.status(),
              sampled_at:
                body.snapshot?.resources?.rss_bytes?.sampled_at ||
                body.page?.observed_at,
            });
          } catch {}
        });
        const route =
          failed === "tasks"
            ? "**/v1/operations/tasks?*"
            : "**/v1/operations/summary";
        await page.route(route, (r) =>
          blocked
            ? r.fulfill({
                status: 503,
                contentType: "application/json",
                body: JSON.stringify({
                  error: {
                    code: "SERVICE_UNAVAILABLE",
                    message: "isolated channel failure",
                  },
                }),
              })
            : r.continue(),
        );
        await page.goto(m.front + "/management/operations");
        if (!initial) {
          await page.locator("[data-task-id]").first().waitFor();
          await page.waitForFunction(
            () =>
              document.querySelectorAll(".ops-overview .ops-source summary")
                .length === 4 &&
              !document
                .querySelector(".ops-overview")
                .innerText.includes("未采集"),
          );
        }
        const before = {
          overview: await page.locator(".ops-overview").innerText(),
          list: await page
            .locator('[role="tabpanel"][data-state="active"]')
            .innerText(),
        };
        blocked = true;
        const begun = Date.now();
        await page
          .getByRole("button", { name: "刷新状态", exact: true })
          .click();
        await page.locator(`[data-channel="${failed}"]`).waitFor();
        await page.waitForTimeout(24000);
        const overview = await page.locator(".ops-overview").innerText();
        const table = await page.locator(".ops-tasks").count();
        const times = await page
          .locator(".ops-source summary")
          .allTextContents();
        const listText = await page
          .locator('[role="tabpanel"][data-state="active"]')
          .innerText();
        const healthy = responses.filter(
          (r) =>
            r.status === 200 &&
            r.path.endsWith(failed === "tasks" ? "/summary" : "/tasks") &&
            r.received_at > begun,
        );
        assert(
          healthy.length >= 4,
          "healthy channel must keep polling on a single cadence",
        );
        assert(Date.parse(healthy.at(-1).sampled_at) > begun + 15000);
        assert.equal(
          await page
            .locator(
              `[data-channel="${failed === "tasks" ? "summary" : "tasks"}"]`,
            )
            .count(),
          0,
        );
        if (failed === "tasks") {
          assert(!overview.includes("已过期"), overview);
          assert(overview.includes("MiB"));
          assert(overview.includes("快照有效"));
          if (!initial) assert.notEqual(overview, before.overview);
          if (initial) assert.equal(table, 0);
          else assert(listText.includes("已过期"));
        } else {
          assert.equal(table, 1);
          assert(listText.includes("快照有效"));
          if (!initial) assert.notEqual(listText, before.list);
          assert(!listText.includes("已过期"));
          if (initial) assert(overview.includes("未采集"));
          else assert(overview.includes("已过期"));
        }
        await page.screenshot({
          path: path.join(
            out,
            `isolated-${failed}-${initial ? "initial" : "retained"}.png`,
          ),
          fullPage: true,
        });
        blocked = false;
        await page
          .getByRole("button", {
            name: failed === "tasks" ? "重新读取列表" : "重新读取摘要",
            exact: true,
          })
          .click();
        await page.waitForFunction(
          () => !document.querySelector('.ops-alert[role="alert"]'),
        );
        await page.locator("[data-task-id]").first().waitFor();
        await page.waitForFunction(
          () =>
            !document
              .querySelector(".ops-overview")
              .innerText.includes("已过期") &&
            document.querySelector(".ops-overview").innerText.includes("MiB"),
        );
        assert.equal(errors.length, 0);
        cases.push({
          failed,
          initial,
          healthy_responses: healthy,
          all_responses: responses,
          before,
          during: { overview, listText, times },
          recovered: true,
          errors,
        });
        await context.close();
      }
    fs.writeFileSync(
      path.join(out, "summary-task-isolation-fixed.json"),
      JSON.stringify({ cases }, null, 2),
    );
  } finally {
    await browser.close();
  }
})().catch((e) => {
  console.error(e);
  process.exitCode = 1;
});
