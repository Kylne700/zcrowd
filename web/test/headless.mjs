// Headless test of web/index.html in real browsers, served by `python3 -m http.server` and opened from file://.
// Privacy: zero requests after load while typing, the Content-Security-Policy stops an injected fetch, image, beacon,
// WebSocket and form, storage stays empty, the URL never changes. Use: attributes, wording, no horizontal scroll at 375x812,
// keyboard-only use, text contrast >= 4.5:1 (computed from computed styles), the verdict in an aria-live region.
// Usage: cd web/test && npm ci && npx playwright-core install chromium firefox webkit   (once)
//        BROWSERS=chromium,firefox,webkit node web/test/headless.mjs          (BROWSERS defaults to chromium)
// PLAYWRIGHT_CORE=<a playwright-core directory> uses that package instead of web/test/node_modules; Chromium is then the
// system one unless CI is set. CHROMIUM=<binary> overrides. Every browser starts with a temporary profile; nothing attaches
// to a running browser.
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";

const WEB = fileURLToPath(new URL("../", import.meta.url));
const COV = JSON.parse(fs.readFileSync(path.join(WEB, "../fixtures/flows-sample.meta.json"), "utf8"));
let pw;
try { pw = createRequire(import.meta.url)(process.env.PLAYWRIGHT_CORE ?? "playwright-core"); } catch {
  console.error("playwright-core not found: run `npm ci` in web/test, or set PLAYWRIGHT_CORE to a playwright-core directory");
  process.exit(2);
}
const names = (process.env.BROWSERS ?? "chromium").split(",").map((s) => s.trim()).filter(Boolean);
for (const n of names) assert.ok(["chromium", "firefox", "webkit"].includes(n), `unknown browser ${n}`);
const systemChromium = process.env.PLAYWRIGHT_CORE && !process.env.CI ? ["/usr/bin/chromium", "/usr/bin/chromium-browser", "/usr/bin/google-chrome"].find((p) => fs.existsSync(p)) : undefined;
const launch = (n) => pw[n].launch({ headless: true, executablePath: n === "chromium" ? process.env.CHROMIUM ?? systemChromium : undefined });

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

const HTTP = `http://127.0.0.1:${port}/index.html`, FILE = pathToFileURL(path.join(WEB, "index.html")).href;
const iso = (t) => new Date(t * 1000).toISOString();
const LAST_UTC = iso(COV.last_t).slice(0, 10) + " " + iso(COV.last_t).slice(11, 16) + " UTC"; // as the page prints it
let httpLoads = 0;

// Helpers defined inside every page before its own script runs (window.__t); the page never calls them.
function pageHelpers() {
  const parse = (c) => {
    const m = /^(rgba?|color\(srgb)[ (]+([^)]+)\)$/.exec(c);
    if (!m) throw new Error("cannot parse colour " + c);
    const v = m[2].split(/[\s,/]+/).filter(Boolean).map((s) => (s.endsWith("%") ? parseFloat(s) / 100 : Number(s))), k = m[1] === "color(srgb" ? 255 : 1;
    return [v[0] * k, v[1] * k, v[2] * k, v.length > 3 ? v[3] : 1];
  };
  const over = (top, below) => [0, 1, 2].map((i) => top[i] * top[3] + below[i] * (1 - top[3])).concat(1); // `below` is opaque
  const lum = (rgb) => rgb.slice(0, 3).map((v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }).reduce((s, v, i) => s + v * [0.2126, 0.7152, 0.0722][i], 0);
  const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
  // what is behind an element: every ancestor's background laid over the white canvas
  const backdrop = (el) => { const chain = []; for (let e = el; e; e = e.parentElement) chain.push(parse(getComputedStyle(e).backgroundColor)); return chain.reduceRight((below, layer) => over(layer, below), [255, 255, 255, 1]); };
  const opacity = (el) => { let o = 1; for (let e = el; e; e = e.parentElement) o *= Number(getComputedStyle(e).opacity); return o; };
  const visible = (el) => el.checkVisibility({ visibilityProperty: true }) && el.getClientRects().length > 0; // false inside a closed <details>
  const label = (e) => e.id || e.dataset.ex || e.tagName.toLowerCase();
  const tabbables = () => [...document.querySelectorAll("a[href], button, input, select, textarea, summary, [tabindex]")].filter((e) => !e.disabled && (e.tagName === "SUMMARY" || e.tabIndex >= 0) && visible(e));
  const contrastScan = () => {
    const out = { checked: 0, min: Infinity, bad: [] };
    const note = (el, colour, what, extra = 1) => {
      const bg = backdrop(el), fg = parse(colour);
      fg[3] *= opacity(el) * extra;
      const r = ratio(over(fg, bg), bg);
      out.checked++; out.min = Math.min(out.min, r);
      if (r < 4.5) out.bad.push(`${what}: ${r.toFixed(2)} (${colour} on rgb(${bg.slice(0, 3).map(Math.round)}))`);
    };
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let n; (n = walker.nextNode()); ) {
      const el = n.parentElement;
      if (n.textContent.trim() && visible(el) && !["SCRIPT", "STYLE", "OPTION"].includes(el.tagName)) note(el, getComputedStyle(el).color, JSON.stringify(n.textContent.trim().slice(0, 30)));
    }
    for (const el of document.querySelectorAll("input, select, textarea")) {
      if (!visible(el)) continue;
      note(el, getComputedStyle(el).color, "#" + el.id + " text");
      if (el.placeholder) note(el, getComputedStyle(el, "::placeholder").color, "#" + el.id + " placeholder", Number(getComputedStyle(el, "::placeholder").opacity));
    }
    return out;
  };
  const ring = () => { // the focus indicator of the focused element
    const el = document.activeElement, cs = getComputedStyle(el), around = backdrop(el.parentElement);
    return { label: label(el), focusVisible: el.matches(":focus-visible"), style: cs.outlineStyle, width: parseFloat(cs.outlineWidth), ratio: ratio(over(parse(cs.outlineColor), around), around) };
  };
  window.__t = { contrastScan, ring, tabbables, label, visible };
}

async function openPage(browser, url, { viewport = { width: 1280, height: 900 }, time } = {}) {
  if (url === HTTP) httpLoads++;
  const ctx = await browser.newContext({ viewport });
  if (time !== undefined) await ctx.clock.setFixedTime(time);
  const log = { routed: [], events: [], failed: [], problems: [] };
  await ctx.route("**", (r) => { log.routed.push(r.request().url()); r.continue(); });
  const page = await ctx.newPage();
  page.on("request", (r) => log.events.push(r.url()));
  page.on("requestfailed", (r) => log.failed.push(r.url() + " " + r.failure()?.errorText));
  page.on("pageerror", (e) => log.problems.push(String(e)));
  page.on("console", (m) => m.type() === "error" && log.problems.push(m.text()));
  await page.addInitScript(pageHelpers);
  await page.addInitScript(() => {
    window.__csp = [];
    document.addEventListener("securitypolicyviolation", (e) => window.__csp.push(e.violatedDirective.split(" ")[0] + " " + e.blockedURI));
  });
  await page.goto(url, { waitUntil: "load" });
  return { ctx, page, log, loaded: { routed: log.routed.length, events: log.events.length } };
}

// Tab and Shift+Tab walk the controls in page order; each shows a visible focus ring; Enter on any control neither submits,
// navigates, reloads nor makes a request.
async function keyboard(page, url, log, loaded) {
  const labels = () => page.evaluate(() => window.__t.tabbables().map(window.__t.label));
  const walk = async (key, n) => { const seen = []; for (let i = 0; i < n; i++) { await page.keyboard.press(key); seen.push(await page.evaluate(() => window.__t.ring())); } return seen; };
  const rings = (seen) => { for (const r of seen) assert.ok(r.focusVisible && r.style !== "none" && r.width >= 2 && r.ratio >= 3, `no visible focus on ${JSON.stringify(r)}`); };
  assert.equal(await page.evaluate(() => document.querySelectorAll('[tabindex]:not([tabindex="0"]):not([tabindex="-1"])').length), 0, "a positive tabindex changes the tab order");
  await page.evaluate(() => { window.__marker = 1; });
  const closed = await labels();
  assert.deepEqual(closed, ["mode", "pool", "amount", "GREEN", "AMBER", "RED", "NO MATCH", "summary"]);
  await page.locator("#mode").focus();
  const fwd = await walk("Tab", closed.length - 1);
  assert.deepEqual(fwd.map((r) => r.label), closed.slice(1), "Tab order is not page order");
  rings(fwd);
  await page.keyboard.press("Enter"); // on the summary: opens Advanced
  assert.equal(await page.evaluate(() => document.querySelector("details").open), true, "Enter on the summary did not open Advanced");
  const open = await labels();
  assert.deepEqual(open, [...closed, "tol", "fee", "look"]);
  const more = await walk("Tab", 3);
  assert.deepEqual(more.map((r) => r.label), ["tol", "fee", "look"]);
  rings(more);
  const back = await walk("Shift+Tab", open.length - 1);
  assert.deepEqual(back.map((r) => r.label), open.slice(0, -1).reverse(), "Shift+Tab order is not reverse page order");
  rings(back); // includes the first control, reached by keyboard
  for (const [i, l] of open.entries()) {
    if (l === "summary") continue;
    await page.evaluate((k) => window.__t.tabbables()[k].focus(), i);
    await page.keyboard.press("Enter");
    if (l === "mode" || l === "pool") await page.keyboard.press("Escape"); // a native menu may have opened
    assert.equal(page.url(), url, `Enter on #${l} changed the URL`);
    assert.equal(await page.evaluate(() => window.__marker), 1, `Enter on #${l} reloaded the page`);
    assert.deepEqual([log.routed.length, log.events.length], [loaded.routed, loaded.events], `Enter on #${l} made a request: ${log.events.slice(loaded.events)}`);
  }
  return { tabStops: open.length };
}

async function privacy(browser, origin, url) {
  const { ctx, page, log, loaded } = await openPage(browser, url);
  const attr = (sel, name) => page.locator(sel).getAttribute(name);
  const resources = await page.evaluate(() => performance.getEntriesByType("resource").map((e) => e.name));
  const contrast = [];
  const scan = async (what) => { const c = await page.evaluate(() => window.__t.contrastScan()); assert.deepEqual(c.bad, [], `contrast < 4.5 (${what})`); contrast.push(c); };

  // static: policy, wording, attributes
  const csp = await attr('meta[http-equiv="Content-Security-Policy"]', "content");
  for (const d of ["default-src 'none'", "connect-src 'none'", "form-action 'none'", "base-uri 'none'", "img-src data:"]) assert.ok(csp.includes(d), `CSP lacks ${d}`);
  assert.equal(await attr('meta[name="referrer"]', "content"), "no-referrer");
  assert.match(await attr('meta[name="viewport"]', "content"), /width=device-width/);
  assert.match(await attr('link[rel="icon"]', "href"), /^data:/);
  assert.match(await page.locator("#cannot-send").innerText(), /This page cannot send anything: its Content-Security-Policy blocks all network requests\. Check DevTools > Network\./);
  assert.equal(await page.locator("form").count(), 0, "no <form> element");
  assert.equal(await page.locator("a").count(), 0, "no links");
  assert.equal(await attr("html", "translate"), "no");
  assert.equal(await attr("html", "lang"), "en");
  for (const id of ["amount", "tol", "fee", "look"]) {
    for (const [name, want] of [["autocomplete", "off"], ["autocapitalize", "off"], ["autocorrect", "off"], ["spellcheck", "false"], ["translate", "no"], ["inputmode", id === "amount" ? "decimal" : "numeric"]]) assert.equal(await attr("#" + id, name), want, `#${id} ${name}`);
  }
  const limits = await page.locator("#limits").innerText();
  for (const re of [/GitHub Pages/, /GitHub sees your IP address like any website/, /never your amount/, /Browser extensions can read any page/, /command line tool/, /downloaded and opened offline in a private window with extensions turned off/]) assert.match(limits, re);
  assert.match((await page.locator("#snapshot").innerText()).trim(), /^Data snapshot of \d{4}-\d\d-\d\d, \d+ days? old; the CLI syncs live data\.$/);
  assert.equal(await page.evaluate(() => ["verdict", "msg"].map((id) => document.getElementById(id).closest('[aria-live="polite"]') !== null).join()), "true,true", "verdict and messages must sit in an aria-live=polite region");

  // keyboard first: it leaves Advanced open, so the scans below see every control
  const kb = await keyboard(page, url, log, loaded);

  // type like a judge: both amounts, then Enter
  const amount = page.locator("#amount");
  const verdicts = {};
  for (const typed of ["1.0", "1.23456789"]) {
    await amount.fill("");
    await amount.pressSequentially(typed, { delay: 15 });
    await amount.press("Enter");
    verdicts[typed] = { verdict: (await page.locator("#verdict").innerText()).trim(), note: (await page.locator("#note").innerText()).trim() };
  }
  // "Not sure" (the default) scores all three pools and shows the worst: 1.0 ZEC has no crowd in Sapling or Orchard
  // but 162 entries into Ironwood. A pool without an entry is NO MATCH, which GREEN outranks, so the missing-shields line has
  // to follow every pool shown (research/46 F1): Sapling is NO MATCH for 1.0 and for 1.23456789 (Orchard never counts)
  const missing = `shields after ${LAST_UTC} (last synced block) are not counted`;
  assert.equal(verdicts["1.0"].verdict, "GREEN");
  assert.equal(verdicts["1.0"].note, `Note: ${missing}.`);
  assert.equal(verdicts["1.23456789"].verdict, "AMBER");
  assert.match(verdicts["1.23456789"].note, /easy to recognise later/);
  assert.ok(verdicts["1.23456789"].note.includes(missing), verdicts["1.23456789"].note);
  await scan("AMBER");
  // the first suggestion promises what typing that amount shows (research/46 F2): crowd and verdict are the worst of the three pools
  const first = (await page.locator("#sugg li").first().innerText()).match(/^(\d+\.\d{8}) ZEC -> crowd (\d+) \(weighted (\d+\.\d), (GREEN|AMBER|RED)\)$/);
  assert.ok(first, "no suggestion line for 1.23456789");
  await amount.fill(first[1]);
  assert.match(await page.locator("#crowd").innerText(), new RegExp(`^${first[2]} .* weighted ${first[3].replace(".", "\\.")}`));
  assert.equal((await page.locator("#verdict").innerText()).trim(), first[4]);
  // a typed value that has a big crowd in two pools: one line per pool, then each pool on its own
  await amount.fill("1.2497");
  await amount.press("Enter");
  assert.equal((await page.locator("#verdict").innerText()).trim(), "GREEN");
  assert.match(await page.locator("#crowd").innerText(), /^157 earlier shields and moves into sapling \(the worst of the three pools\) of about 1\.24970000 ZEC .* weighted 114\.5/);
  assert.deepEqual(await page.locator("#pools li").allInnerTexts(), ["pool sapling: k=157 weighted=114.5 -> GREEN", "pool orchard: k=0 weighted=0.0 -> NO MATCH", "pool ironwood: k=592 weighted=437.0 -> GREEN"]);
  assert.match(await page.locator("#pool-note").innerText(), /^592 entries of about this amount went into the other pools: linkable only through a visible pool move; not counted\./);
  // only Orchard has no entry here, and no later block can add one (ZIP 258): no missing-shields line
  assert.doesNotMatch(await page.locator("#note").innerText(), /shields after/);
  const poolPick = {};
  for (const pool of ["ironwood", "orchard", "sapling"]) {
    await page.locator("#pool").selectOption(pool);
    poolPick[pool] = { verdict: (await page.locator("#verdict").innerText()).trim(), crowd: (await page.locator("#crowd").innerText()).trim(), lines: await page.locator("#pools li").count() };
    assert.equal(poolPick[pool].lines, 0, "one pool chosen: no per-pool list");
  }
  assert.equal(poolPick.ironwood.verdict, "GREEN");
  assert.match(poolPick.ironwood.crowd, /^592 earlier shields and moves into ironwood of about 1\.24970000 ZEC .* weighted 437\.0/);
  assert.equal(poolPick.orchard.verdict, "NO MATCH");
  assert.equal(poolPick.sapling.verdict, "GREEN");
  assert.match(poolPick.sapling.crowd, /^157 earlier shields and moves into sapling of about /);
  await page.locator("#pool").selectOption("any");
  // NO MATCH and RED say which shields are missing (the CLI prints the same words); GREEN and AMBER do not
  const examples = {};
  for (const want of ["GREEN", "AMBER", "RED", "NO MATCH"]) {
    await page.locator(`button[data-ex="${want}"]`).click();
    examples[want] = [await amount.inputValue(), (await page.locator("#verdict").innerText()).trim()];
    assert.equal(examples[want][1], want, `example button ${want}`);
    const note = await page.locator("#note").innerText();
    if (want === "RED" || want === "NO MATCH") assert.ok(note.includes(missing), `${want}: no "${missing}" in ${JSON.stringify(note)}`);
    else assert.doesNotMatch(note, /shields after/, want);
    await scan(want);
  }
  await amount.fill("abc");
  assert.match(await page.locator("#msg").innerText(), /Use digits and at most 8 decimals/);
  await scan("error message");
  await amount.fill("");
  assert.match(await page.locator("#msg").innerText(), /Type an amount/);
  await scan("empty");
  // shield mode: the decoys are the other entries into the pool the funds arrive in. A shield cannot go into Orchard (ZIP 258),
  // so "Not sure" is the worse of Sapling and Ironwood: 1.24985 ZEC has a crowd of 88 in Sapling but 8 in Ironwood
  await page.locator("#mode").selectOption("shield");
  await amount.fill("1.24985");
  assert.equal((await page.locator("#verdict").innerText()).trim(), "AMBER");
  assert.deepEqual(await page.locator("#pools li").allInnerTexts(), ["pool sapling: k=88 weighted=64.5 -> GREEN", "pool ironwood: k=8 weighted=6.0 -> AMBER"]);
  assert.match(await page.locator("#crowd").innerText(), /^8 other shields and moves into ironwood \(the worst of the two pools\) of about 1\.24985000 ZEC /);
  assert.match(await page.locator("#pool option[value=any]").innerText(), /^Not sure \(check Sapling and Ironwood, show the worse\)$/);
  await scan("shield AMBER");
  await page.locator("#pool").selectOption("orchard");
  assert.match(await page.locator("#msg").innerText(), /^A shield cannot go into Orchard: from NU6\.3 on no value may enter that pool \(ZIP 258\)\./);
  assert.ok(await page.locator("#out").isHidden(), "no verdict for a shield into Orchard");
  await scan("shield into Orchard refused");
  await page.locator("#pool").selectOption("sapling");
  assert.equal((await page.locator("#verdict").innerText()).trim(), "GREEN");
  assert.equal((await page.locator("#msg").innerText()).trim(), "");
  assert.match(await page.locator("#crowd").innerText(), /^88 other shields and moves into sapling of about 1\.24985000 ZEC /);
  await page.locator("#pool").selectOption("any");
  assert.equal(page.url(), url, "URL changed");
  assert.equal(log.routed.length, loaded.routed, `requests after load while typing: ${log.routed.slice(loaded.routed)}`);
  assert.equal(log.events.length, loaded.events, `request events after load: ${log.events.slice(loaded.events)}`);
  const typingEvents = log.events.length - loaded.events;

  if (process.env.SCREENSHOT && origin === "http" && browser.browserType().name() === "chromium") { await page.locator("#mode").selectOption("unshield"); await amount.fill("1.23456789"); await page.screenshot({ path: process.env.SCREENSHOT, fullPage: true }); }
  // a store the browser refuses to open on this origin cannot hold anything either
  const storage = await page.evaluate(async () => {
    const one = (f) => f().then((n) => n, () => "unavailable");
    return { local: await one(async () => localStorage.length), session: await one(async () => sessionStorage.length), cookie: document.cookie, idb: await one(async () => (await indexedDB.databases()).length), caches: await one(async () => (await caches.keys()).length) };
  });
  for (const [k, v] of Object.entries(storage)) assert.ok(v === 0 || v === "" || v === "unavailable", `storage ${k}: ${v}`);

  // injected exfiltration attempts: the Content-Security-Policy must stop each one before any request is made
  // the targets are the live local server, so anything that got through would show in its access log
  const base = `http://127.0.0.1:${port}`;
  const attempts = await page.evaluate(async (b) => ({
    fetch: await fetch(b + "/x-fetch?q=1").then(() => "sent", (e) => "blocked " + e.name),
    img: await new Promise((r) => { const i = new Image(); i.onload = () => r("loaded"); i.onerror = () => r("blocked"); i.src = b + "/x-img.png?q=2"; }),
    beacon: "returned " + navigator.sendBeacon(b + "/x-beacon?q=3", "q"),
    ws: await new Promise((r) => { try { const w = new WebSocket(b.replace("http", "ws") + "/x-ws"); w.onerror = () => r("failed"); w.onopen = () => r("open"); } catch (e) { r("blocked " + e.name); } }),
  }), base);
  assert.equal(attempts.fetch, "blocked TypeError");
  assert.equal(attempts.img, "blocked");
  assert.notEqual(attempts.ws, "open");
  // Firefox throws from submit() when the policy blocks the action; the others return
  attempts.form = await page.evaluate((b) => { const f = document.createElement("form"); f.action = b + "/x-form?q=4"; document.body.appendChild(f); try { f.submit(); return "returned"; } catch { return "threw"; } }, base);
  await page.waitForTimeout(400);
  // each vector must show up as a policy violation (sendBeacon returns true even when blocked, so its return value proves nothing)
  const violations = await page.evaluate(() => window.__csp);
  for (const [dir, frag] of [["connect-src", "/x-fetch"], ["img-src", "/x-img.png"], ["connect-src", "/x-beacon"], ["connect-src", "/x-ws"], ["form-action", "/x-form"]]) {
    assert.ok(violations.some((v) => v.startsWith(dir) && v.includes(frag)), `no ${dir} violation for ${frag}: ${violations}`);
  }
  assert.equal(page.url(), url, "form submit navigated");
  assert.equal(log.routed.length, loaded.routed, `a request left the page: ${log.routed.slice(loaded.routed)}`);
  assert.deepEqual(log.problems.filter((p) => !/Content[- ]Security[- ]Policy|Refused to|violates/i.test(p)), [], "page errors");

  await ctx.close();
  const min = Math.min(...contrast.map((c) => c.min));
  return { origin, loadRequests: loaded.events, resources: resources.length, requestsWhileTyping: typingEvents, requestsAfterAttempts: log.routed.length - loaded.routed, failedByPolicy: log.failed.length, violations: violations.length, tabStops: kb.tabStops, contrastTexts: contrast.reduce((n, c) => n + c.checked, 0), contrastMin: Number(min.toFixed(2)), attempts, storage, examples, verdicts, poolPick };
}

// 375x812, the longest layouts (Advanced open): nothing may reach past the right edge, and the page must not scroll sideways
async function narrow(browser, url) {
  const { ctx, page } = await openPage(browser, url, { viewport: { width: 375, height: 812 } });
  await page.locator("summary").click();
  const amount = page.locator("#amount");
  const sideways = () => page.evaluate(() => {
    scrollTo(0, 0);
    const w = document.documentElement.clientWidth;
    const wide = [...document.querySelectorAll("body *")].filter((e) => e.tagName !== "OPTION" && window.__t.visible(e) && e.getBoundingClientRect().right > w + 0.5).map((e) => e.id || e.tagName.toLowerCase());
    scrollTo(1000, 0); // a page that cannot scroll sideways stays at 0
    return { w, scrollWidth: document.documentElement.scrollWidth, bodyScrollWidth: document.body.scrollWidth, scrollX, wide };
  });
  const widest = [];
  for (const [state, act] of [
    ["empty", async () => {}],
    ...["GREEN", "AMBER", "RED", "NO MATCH"].map((want) => [want, () => page.locator(`button[data-ex="${want}"]`).click()]),
    ["odd amount", () => amount.fill("1.23456789")],
    ["error", () => amount.fill("abc")],
    ["shield", async () => { await page.locator("#mode").selectOption("shield"); await amount.fill("1.24985"); }],
  ]) {
    await act();
    const s = await sideways();
    assert.ok(s.scrollX === 0 && s.scrollWidth <= s.w && s.bodyScrollWidth <= s.w && s.wide.length === 0, `horizontal overflow at 375px (${state}): ${JSON.stringify(s)}`);
    widest.push(Math.max(s.scrollWidth, s.bodyScrollWidth));
  }
  await ctx.close();
  return { widest: Math.max(...widest), clientWidth: 375 };
}

// The page says how old its frozen sample is, from the wall clock: plain wording, not an error
async function snapshotLine(browser, url) {
  const day = iso(COV.last_t).slice(0, 10), seen = [];
  for (const [offset, days] of [[0, 0], [-86400, 0], [86400 + 3600, 1], [3 * 86400 + 5, 3], [400 * 86400, 400]]) {
    const { ctx, page } = await openPage(browser, url, { time: new Date((COV.last_t + offset) * 1000) });
    const text = (await page.locator("#snapshot").innerText()).trim();
    assert.equal(text, `Data snapshot of ${day}, ${days} ${days === 1 ? "day" : "days"} old; the CLI syncs live data.`);
    assert.doesNotMatch(text, /warn|error|stale|outdated|expired/i);
    seen.push(days);
    await ctx.close();
  }
  return seen;
}

const report = [], versions = [];
try {
  for (const name of names) {
    const browser = await launch(name);
    versions.push(`${name} ${browser.version()}`);
    try {
      const runs = [];
      for (const [origin, url] of [["http", HTTP], ["file", FILE]]) runs.push(await privacy(browser, origin, url));
      const narrowRun = await narrow(browser, FILE), ageDays = await snapshotLine(browser, FILE);
      report.push({ browser: versions.at(-1), runs, narrow: narrowRun, snapshotDays: ageDays });
      console.log(`${versions.at(-1)}: ${runs.map((r) => `${r.origin}: ${r.requestsWhileTyping} requests while typing, ${r.violations} policy violations, ${r.tabStops} tab stops, ${r.contrastTexts} texts at >= ${r.contrastMin}:1`).join("; ")}; 375px: widest ${narrowRun.widest}`);
    } finally {
      await browser.close();
    }
  }
} finally {
  server.kill();
}

// what the static server itself saw: each browser fetched the page over http once and nothing else
await new Promise((r) => setTimeout(r, 300)); // let the server flush its log
const hits = served.split("\n").filter((l) => /"(GET|POST|HEAD|PUT)/.test(l)).map((l) => l.match(/"([A-Z]+ \S+)/)[1]).filter((h) => h !== "GET /__ready");
assert.deepEqual(hits, Array(httpLoads).fill("GET /index.html"), `server saw: ${hits}`);
if (process.env.VERBOSE) console.log(JSON.stringify({ server_saw: hits, report }, null, 1));
console.log(`HEADLESS OK (${versions.join(", ")}; the server saw ${hits.length} page load${hits.length === 1 ? "" : "s"} and nothing else)`);
