// Real isolated Chrome, actual API and frozen v3 value/source reads after synthetic review.
const fs = require("node:fs"),
  path = require("node:path"),
  assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const {
  chromium,
} = require("/Users/sheldon/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright");
const dir = path.resolve(process.argv[2]);
assert(dir.startsWith("/tmp/nexofolio-operations/browser."));
const m = JSON.parse(fs.readFileSync(path.join(dir, "browser.json")));
assert.equal(m.database, "nexofolio_operations_browser");
(async () => {
  const login = await fetch(m.api + "/v1/auth/login", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ account: "A80646", password: "fixture-pass" }),
  }).then((r) => r.json());
  const headers = { authorization: "Bearer " + login.token };
  const task = await fetch(
    `${m.api}/v1/projects/${m.projects[0]}/curate/tasks/${m.task}`,
    { headers },
  ).then((r) => r.json());
  const candidate = task.candidate_id;
  const version = execFileSync(
    "/opt/homebrew/opt/postgresql@17/bin/psql",
    [
      "-XqAt",
      "-h",
      "127.0.0.1",
      "-p",
      String(m.pg_port),
      "-d",
      m.database,
      "-c",
      `SELECT execution_input->>'version' FROM knowledge.candidates WHERE id='${candidate}'`,
    ],
    { encoding: "utf8" },
  ).trim();
  assert.equal(version, "3");
  const applied = await fetch(
    `${m.api}/v1/projects/${m.projects[0]}/curate/candidates/${candidate}/apply`,
    { method: "POST", headers },
  );
  assert.equal(applied.status, 200);
  const browser = await chromium.launch({
    executablePath:
      "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    headless: true,
  });
  const context = await browser.newContext({
    viewport: { width: 1512, height: 1100 },
    permissions: ["clipboard-read", "clipboard-write"],
  });
  await context.addInitScript(
    (v) =>
      localStorage.setItem(
        "nexofolio.session",
        JSON.stringify({ token: v.token, expiresAt: v.expires_at }),
      ),
    login,
  );
  const page = await context.newPage(),
    errors = [],
    reads = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("response", (r) => {
    if (r.url().includes("/field-values") || r.url().includes("/examples/"))
      reads.push({
        path: new URL(r.url()).pathname,
        query: new URL(r.url()).search,
        status: r.status(),
      });
  });
  try {
    await page.goto(
      `${m.front}/projects/${m.projects[0]}/interfaces/endpoints/${m.endpoints[0][0]}?env=${m.environments[0]}`,
    );
    await page
      .getByRole("button", { name: "查看观测值", exact: true })
      .first()
      .click();
    await page.locator(".observed-values .observed-item").first().waitFor();
    await page
      .getByRole("button", { name: "复制完整值", exact: true })
      .first()
      .click();
    await page
      .getByText("已复制完整值（JSON，保留类型）", { exact: true })
      .waitFor();
    const copied = await page.evaluate(() => navigator.clipboard.readText());
    assert.notEqual(copied, "");
    JSON.parse(copied);
    await page.screenshot({
      path: path.join(dir, "frozen-values.png"),
      fullPage: true,
    });
    const response = page.waitForResponse(
      (r) => r.url().includes("/examples/") && r.status() === 200,
    );
    await page
      .getByRole("button", { name: "查看真实样本", exact: true })
      .first()
      .click();
    await response;
    await page.screenshot({
      path: path.join(dir, "frozen-source.png"),
      fullPage: true,
    });
    assert.equal(errors.length, 0);
    assert(reads.length >= 3 && reads.every((r) => r.status === 200));
    fs.writeFileSync(
      path.join(dir, "frozen-browser-results.json"),
      JSON.stringify(
        {
          version,
          cases: [
            "v3 synthetic candidate review/apply",
            "persisted observed values rendered",
            "full typed value copied through clipboard",
            "source sample read from actual API",
          ],
          errors,
          reads,
          copy_json_bytes: Buffer.byteLength(copied),
        },
        null,
        2,
      ),
    );
  } finally {
    await browser.close();
  }
})().catch((e) => {
  console.error(e);
  process.exitCode = 1;
});
