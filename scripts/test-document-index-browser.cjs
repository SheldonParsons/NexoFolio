// Real Chrome and API; delays wrap actual replies only for stale-response acceptance.
const fs = require("node:fs"),
  assert = require("node:assert/strict");
const {
  chromium,
} = require("/Users/sheldon/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright");
const m = JSON.parse(fs.readFileSync(0, "utf8"));
const save = (name, value) =>
  fs.writeFileSync(
    `${m.out}/${m.label}-${name}.json`,
    JSON.stringify(value, null, 2),
  );
const base = `/projects/${m.project}/interfaces`,
  url = `${m.front}${base}?env=${m.environment}`;
const indexPath = (path) =>
  /\/document$|\/catalog(?:\/|$)|\/endpoints$|\/document-lifecycle$/.test(path);
(async () => {
  const browser = await chromium.launch({
    executablePath:
      "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    headless: true,
  });
  const context = await browser.newContext({
    viewport: { width: 1440, height: 1000 },
  });
  await context.addInitScript((login) => {
    localStorage.setItem(
      "nexofolio.session",
      JSON.stringify({ token: login.token, expiresAt: login.expires_at }),
    );
    window.documentTimings = {};
    const observer = new MutationObserver(() => {
      if (
        document.querySelector("[data-endpoint-id]") &&
        !window.documentTimings.indexDom
      )
        window.documentTimings.indexDom = performance.now();
      if (document.querySelector(".brand-transition"))
        window.documentTimings.animationSeen = true;
    });
    observer.observe(document, { subtree: true, childList: true });
  }, m.login);
  const page = await context.newPage(),
    errors = [],
    failed = [],
    network = [],
    rounds = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("requestfailed", (r) =>
    failed.push({
      path: new URL(r.url()).pathname,
      error: r.failure()?.errorText,
    }),
  );
  page.on("requestfinished", (r) => {
    if (r.url().includes("/api/"))
      network.push({
        path: new URL(r.url()).pathname,
        query: new URL(r.url()).search,
        timing: r.timing(),
      });
  });
  const search = page.getByRole("searchbox", {
    name: "搜索接口名称、用途或路径",
  });
  const rows = page.locator(".interface-list-scroll [data-endpoint-id]");
  const ready = () =>
    page.waitForFunction(
      () =>
        document.querySelector(".interface-list-scroll [data-endpoint-id]") &&
        !document.querySelector(".app-viewport[inert]"),
    );
  const navigate = async (path) =>
    page.evaluate((path) => {
      history.pushState({}, "", path);
      dispatchEvent(new PopStateEvent("popstate"));
    }, path);
  try {
    for (let round = 0; round < 4; round++) {
      network.length = 0;
      await page.goto(url);
      await ready();
      assert.equal(await rows.count(), 502);
      const timings = await page.evaluate(() => ({
        ...window.documentTimings,
        htmlDom:
          performance.getEntriesByType("navigation")[0]
            .domContentLoadedEventEnd,
      }));
      await search.fill("/synthetic/orders");
      await page.waitForFunction(
        () =>
          document.querySelectorAll(".interface-list-scroll [data-endpoint-id]")
            .length === 1,
      );
      timings.interactive = await page.evaluate(() => performance.now());
      const reads = network.filter((r) => indexPath(r.path));
      assert.equal(
        reads.filter((r) => r.path.endsWith("/catalog/unclassified")).length,
        m.label === "before" ? 6 : 0,
      );
      if (m.label === "after") assert.equal(reads.length, 1);
      assert(!network.some((r) => /\/endpoints\/|\/examples\//.test(r.path)));
      if (round)
        rounds.push({
          round,
          ...timings,
          indexRequests: reads.length,
          network: [...network],
        });
    }
    save("timings", rounds);
    await search.fill("");
    await page.screenshot({
      path: `${m.out}/${m.label}-document.png`,
      fullPage: true,
    });
    if (m.label === "after") {
      const retired = page.getByRole("checkbox", { name: "显示已废弃" });
      await retired.click();
      assert.equal(await rows.count(), 503);
      await retired.click();
      await page
        .getByRole("button", { name: "折叠父目录", exact: true })
        .click();
      await search.fill("/synthetic/orders");
      await search.fill("");
      assert.equal(
        await page
          .getByRole("button", { name: "展开父目录", exact: true })
          .getAttribute("aria-expanded"),
        "false",
      );
      await page
        .getByRole("button", { name: "展开父目录", exact: true })
        .click();
      const beforeDetail = network.length;
      await page.locator(`[data-endpoint-id="${m.target}"]`).click();
      await page.getByRole("tab", { name: /真实样本/ }).waitFor();
      assert(
        network
          .slice(beforeDetail)
          .some((r) => r.path.endsWith(`/endpoints/${m.target}`)),
      );
      assert(!network.some((r) => /\/examples\//.test(r.path)));
      await page.getByRole("tab", { name: /真实样本/ }).click();
      await page.locator(".sample-choice button").first().click();
      await page.waitForResponse(
        (r) => r.url().includes("/examples/") && r.status() === 200,
      );
      await page.screenshot({
        path: `${m.out}/after-sample.png`,
        fullPage: true,
      });
      // Delayed old-project response must be cancelled and cannot replace the new project's index.
      let release, started;
      const pending = new Promise((r) => {
        started = r;
      });
      const gate = new Promise((r) => {
        release = r;
      });
      await page.route(
        `**/v1/projects/${m.forbidden_project}/document`,
        async (route) => {
          const response = await route.fetch();
          started();
          await gate;
          await route.fulfill({ response }).catch(() => {});
        },
      );
      await navigate(`/projects/${m.forbidden_project}/interfaces`);
      await pending;
      await navigate(`${base}?env=${m.environment}`);
      await ready();
      release();
      await page.waitForTimeout(100);
      assert.equal(await rows.count(), 502);
      assert(
        failed.some(
          (r) =>
            r.path.endsWith(`/${m.forbidden_project}/document`) &&
            r.error.includes("ABORTED"),
        ),
      );
      // Rapid environment selection preserves the project-wide index and filters locally.
      const indexCount = network.filter((r) =>
        r.path.endsWith("/document"),
      ).length;
      for (const env of [
        m.other_environment,
        m.environment,
        m.other_environment,
      ]) {
        await navigate(`${base}?env=${env}`);
        await page.waitForFunction((env) => location.search.includes(env), env);
      }
      await page.waitForFunction(
        () =>
          document.querySelectorAll(".interface-list-scroll [data-endpoint-id]")
            .length === 1,
      );
      assert.equal(
        network.filter((r) => r.path.endsWith("/document")).length,
        indexCount,
      );
      assert.equal(
        await page.locator(`[data-endpoint-id="${m.target}"]`).count(),
        1,
      );
      assert.equal(
        await page
          .getByRole("button", { name: "开始新一轮整理", exact: true })
          .count(),
        1,
      );
      assert.equal(
        await page.getByRole("link", { name: "管理中心", exact: true }).count(),
        0,
      );
      save("functional", {
        cases: [
          "retired visibility",
          "directory expansion and search",
          "details and samples on demand",
          "late project response cancelled",
          "rapid environment changes reuse index",
          "ordinary member layout",
        ],
        failed,
        network,
      });
    }
    assert.deepEqual(errors, []);
    save("errors", errors);
  } catch (error) {
    await page.screenshot({
      path: `${m.out}/${m.label}-failure.png`,
      fullPage: true,
    });
    save("failure", {
      message: error.message,
      errors,
      failed,
      network,
      text: await page.locator("body").innerText(),
    });
    throw error;
  } finally {
    await browser.close();
  }
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
