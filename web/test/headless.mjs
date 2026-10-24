// Headless privacy test of web/index.html, served by `python3 -m http.server` and opened from file://.
// Usage: PLAYWRIGHT_CORE=<path to a playwright-core package> [CHROMIUM=<browser binary>] node web/test/headless.mjs
// Starts its own browser with a temporary profile; it never attaches to any other browser.
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";

const WEB = fileURLToPath(new URL("../", import.meta.url));
if (!process.env.PLAYWRIGHT_CORE) { console.error("set PLAYWRIGHT_CORE to a playwright-core package directory"); process.exit(2); }
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_CORE);
const exe = process.env.CHROMIUM ?? ["/usr/bin/chromium", "/usr/bin/chromium-browser", "/usr/bin/google-chrome"].find((p) => fs.existsSync(p));
if (!exe) { console.error("no Chromium found: set CHROMIUM"); process.exit(2); }

const freePort = () => new Promise((res) => { const s = net.createServer().listen(0, "127.0.0.1", () => { const p = s.address().port; s.close(() => res(p)); }); });
const port = await freePort();
const server = spawn("python3", ["-m", "http.server", String(port), "--bind", "127.0.0.1", "--directory", WEB], { stdio: ["ignore", "ignore", "pipe"] });
let served = "";
server.stderr.on("data", (d) => (served += d));
for (let i = 0; ; i++) { // wait until it answers
  if (await fetch(`http://127.0.0.1:${port}/__ready`).then(() => true, () => false)) break;
  assert.ok(i < 50, "python http.server did not start");
  await new Promise((r) => setTimeout(r, 100));
}

const browser = await chromium.launch({ executablePath: exe, headless: true });
const report = [];
try {
  for (const [name, url] of [["http", `http://127.0.0.1:${port}/index.html`], ["file", pathToFileURL(path.join(WEB, "index.html")).href]]) {
    const ctx = await browser.newContext();
    const routed = [], events = [], failed = [], problems = [];
    await ctx.route("**", (r) => { routed.push(r.request().url()); r.continue(); });
    const page = await ctx.newPage();
    page.on("request", (r) => events.push(r.url()));
    page.on("requestfailed", (r) => failed.push(r.url() + " " + r.failure()?.errorText));
    page.on("pageerror", (e) => problems.push(String(e)));
    page.on("console", (m) => m.type() === "error" && problems.push(m.text()));
    await page.addInitScript(() => {
      window.__csp = [];
      document.addEventListener("securitypolicyviolation", (e) => window.__csp.push(e.violatedDirective.split(" ")[0] + " " + e.blockedURI));
    });
    await page.goto(url, { waitUntil: "load" });
    const loaded = { routed: routed.length, events: events.length };
    const resources = await page.evaluate(() => performance.getEntriesByType("resource").map((e) => e.name));

    const csp = await page.locator('meta[http-equiv="Content-Security-Policy"]').getAttribute("content");
    for (const d of ["default-src 'none'", "connect-src 'none'", "form-action 'none'", "base-uri 'none'", "img-src data:"]) assert.ok(csp.includes(d), `CSP lacks ${d}`);
    assert.equal(await page.locator('meta[name="referrer"]').getAttribute("content"), "no-referrer");
    assert.match(await page.locator("#cannot-send").innerText(), /This page cannot send anything: its Content-Security-Policy blocks all network requests\. Check DevTools > Network\./);
    assert.equal(await page.locator("form").count(), 0, "no <form> element");
    for (const id of ["amount", "tol", "fee", "look"]) assert.equal(await page.locator("#" + id).getAttribute("autocomplete"), "off");

    // type like a judge: both amounts, then Enter
    const amount = page.locator("#amount");
    const verdicts = {};
    for (const typed of ["1.0", "1.23456789"]) {
      await amount.fill("");
      await amount.pressSequentially(typed, { delay: 15 });
      await amount.press("Enter");
      verdicts[typed] = { verdict: (await page.locator("#verdict").innerText()).trim(), note: (await page.locator("#note").innerText()).trim() };
    }
    assert.equal(verdicts["1.0"].verdict, "NO MATCH");
    assert.equal(verdicts["1.0"].note, "");
    assert.equal(verdicts["1.23456789"].verdict, "NO MATCH");
    assert.match(verdicts["1.23456789"].note, /easy to recognise later/);
    // a typed value that has a big crowd, and the four example buttons
    await amount.fill("1.2497");
    await amount.press("Enter");
    assert.equal((await page.locator("#verdict").innerText()).trim(), "GREEN");
    assert.match(await page.locator("#crowd").innerText(), /^157 earlier shields .* weighted 114\.5/);
    const examples = {};
    for (const want of ["GREEN", "AMBER", "RED", "NO MATCH"]) {
      await page.locator(`button[data-ex="${want}"]`).click();
      examples[want] = [await amount.inputValue(), (await page.locator("#verdict").innerText()).trim()];
      assert.equal(examples[want][1], want, `example button ${want}`);
    }
    await page.locator("#mode").selectOption("shield");
    await amount.fill("1.24985");
    assert.equal((await page.locator("#verdict").innerText()).trim(), "GREEN");
    assert.equal(page.url(), url, "URL changed");
    assert.equal(routed.length, loaded.routed, `requests after load while typing: ${routed.slice(loaded.routed)}`);
    assert.equal(events.length, loaded.events, `request events after load: ${events.slice(loaded.events)}`);
    const typingEvents = events.length - loaded.events;

    if (process.env.SCREENSHOT && name === "http") { await page.locator("#mode").selectOption("unshield"); await amount.fill("1.23456789"); await page.screenshot({ path: process.env.SCREENSHOT, fullPage: true }); }
    const storage = await page.evaluate(async () => ({
      local: localStorage.length, session: sessionStorage.length, cookie: document.cookie,
      idb: (await indexedDB.databases()).length, caches: (await caches.keys()).length,
    }));
    assert.deepEqual(storage, { local: 0, session: 0, cookie: "", idb: 0, caches: 0 });

    // injected exfiltration attempts: the Content-Security-Policy must stop each one before any request is made
    // the targets are the live local server, so anything that got through would show in its access log
    const attempts = await page.evaluate(async (base) => ({
      fetch: await fetch(base + "/x?q=1").then(() => "sent", (e) => "blocked " + e.name),
      img: await new Promise((r) => { const i = new Image(); i.onload = () => r("loaded"); i.onerror = () => r("blocked"); i.src = base + "/x.png?q=2"; }),
      beacon: "returned " + navigator.sendBeacon(base + "/x?q=3", "q"),
      ws: await new Promise((r) => { try { const w = new WebSocket(base.replace("http", "ws") + "/x"); w.onerror = () => r("failed"); w.onopen = () => r("open"); } catch (e) { r("blocked " + e.name); } }),
    }), `http://127.0.0.1:${port}`);
    assert.equal(attempts.fetch, "blocked TypeError");
    assert.equal(attempts.img, "blocked");
    await page.evaluate((base) => { const f = document.createElement("form"); f.action = base + "/x?q=4"; document.body.appendChild(f); f.submit(); }, `http://127.0.0.1:${port}`);
    await page.waitForTimeout(400);
    // each vector must show up as a policy violation (sendBeacon returns true even when blocked, so its return value proves nothing)
    const violations = await page.evaluate(() => window.__csp);
    for (const [dir, frag] of [["connect-src", "/x?q=1"], ["img-src", "/x.png?q=2"], ["connect-src", "/x?q=3"], ["connect-src", `ws://127.0.0.1:${port}/x`], ["form-action", "/x"]]) {
      assert.ok(violations.some((v) => v.startsWith(dir + " ") && v.includes(frag)), `no ${dir} violation for ${frag}: ${violations}`);
    }
    assert.equal(page.url(), url, "form submit navigated");
    assert.equal(routed.length, loaded.routed, `a request left the page: ${routed.slice(loaded.routed)}`);
    assert.deepEqual(problems.filter((p) => !/Content Security Policy|Refused to/i.test(p)), [], "page errors");

    report.push({ origin: name, loadRequests: loaded.events, resources, requestsWhileTyping: typingEvents, afterInjectedAttempts: { requestEvents: events.slice(loaded.events), failed, reachedNetwork: routed.length - loaded.routed }, verdicts, examples, violations: violations.length, attempts, storage });
    await ctx.close();
  }
} finally {
  await browser.close();
  server.kill();
}

// what the static server itself saw: the browser fetched the page and nothing else
await new Promise((r) => setTimeout(r, 300)); // let the server flush its log
const hits = served.split("\n").filter((l) => /"(GET|POST|HEAD|PUT)/.test(l)).map((l) => l.match(/"([A-Z]+ \S+)/)[1]).filter((h) => h !== "GET /__ready");
assert.deepEqual(hits, ["GET /index.html"], `server saw: ${hits}`);
console.log(JSON.stringify({ server_saw: hits, runs: report }, null, 1));
console.log("HEADLESS OK");
