// Parity: the script shipped in web/index.html must give the same crowd, verdict and suggestions as the Rust CLI.
// Usage: cargo build -p zcrowd && node web/test/parity.mjs   (ZCROWD_BIN overrides the binary path)
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import vm from "node:vm";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { pageData, render } from "../build.mjs";

const ROOT = fileURLToPath(new URL("../../", import.meta.url));
const BIN = process.env.ZCROWD_BIN ?? path.join(ROOT, "target/debug/zcrowd");
const SAMPLE = path.join(ROOT, "fixtures/flows-sample.ndjson");
const DAY = 86400;
const read = (p) => fs.readFileSync(p, "utf8");
const template = read(path.join(ROOT, "web/index.template.html"));
const shipped = read(path.join(ROOT, "web/index.html"));
const ndjson = (p) => read(p).split("\n").filter(Boolean).map((l) => JSON.parse(l));
const sampleEvents = ndjson(SAMPLE);
const sampleCov = JSON.parse(read(path.join(ROOT, "fixtures/flows-sample.meta.json")));

// The file under test must be the one the build produces from the template and the committed sample.
assert.equal(shipped, render(template, pageData(sampleEvents, sampleCov)), "web/index.html is stale: run node web/build.mjs");

function load(html) {
  const scripts = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)];
  assert.equal(scripts.length, 1, "exactly one inline script");
  const ctx = vm.createContext({}); // no document: only the scoring part of the script runs
  vm.runInContext(scripts[0][1], ctx);
  const ev = (code) => vm.runInContext(code, ctx);
  const plain = (x) => JSON.parse(JSON.stringify(x)); // drop the other realm's prototypes
  const [parse, check] = [ev("parseZec"), ev("check")];
  return { parse, check: (...a) => plain(check(...a)), now: ev("NOW") };
}

function cli(data, mode, amount, c) {
  const flags = [];
  if (c.tol !== undefined) flags.push(`--tolerance-zat=${c.tol}`);
  if (c.fee !== undefined) flags.push(`--fee-allowance-zat=${c.fee}`);
  if (c.look !== undefined) flags.push(`--lookback-days=${c.look}`);
  const r = spawnSync(BIN, ["check", `--${mode}=${amount}`, "--json", "--data", data, ...flags], { cwd: ROOT, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
  return r.status === 0 ? JSON.parse(r.stdout) : null;
}

let compared = 0;
function same(page, data, label, mode, amount, c = {}) {
  const want = cli(data, mode, amount, c);
  assert.ok(want, `${label}: the CLI rejected a valid amount ${amount}`);
  const got = page.check(mode, page.parse(amount), { lookback: c.look ?? 30, tol: c.tol ?? 0, fee: c.fee ?? 50000, now: page.now });
  const view = (r, matches) => ({ k: r.k, weighted: r.weighted, verdict: r.verdict, odd_amount: r.odd_amount, matches, suggestions: r.suggestions });
  assert.deepStrictEqual(view(got, got.matches.length), view(want, want.matches.length), `${label}: ${mode} ${amount} ${JSON.stringify(c)}`);
  compared++;
  return got;
}

const fmt = (z) => `${Math.floor(z / 1e8)}.${String(z % 1e8).padStart(8, "0")}`;

// ---- 1. the real sample ----
const page = load(shipped);
let amounts = 0;
for (const a of ["1.0", "1.23456789", "1.2497", "1.24985", "1.25", "2.5", "0.001", "125", "5", "0.0001", "100", "3.74975", "4.9997", "1.", ".5", " 007.50 ", "21000000", "0.00000001", "1.00015", "0.01015", "49.99999999"]) {
  same(page, SAMPLE, "sample", "unshield", a);
  amounts++;
}
for (const a of ["1.24985", "1.0", "2.4998", "125", "4.9997"]) { same(page, SAMPLE, "sample", "shield", a); amounts++; }
for (const c of [{ tol: 10000 }, { tol: 100000000 }, { fee: 0 }, { fee: 10000 }, { look: 7 }, { look: 14 }, { look: 45 }]) {
  same(page, SAMPLE, "sample", "unshield", "1.2497", c);
  same(page, SAMPLE, "sample", "shield", "1.24985", c);
}

// real shields with an amount no other shield has, so the window edges are exact
const shields = sampleEvents.filter((e) => e.kind === "shield" && e.confidence === "high");
const seen = new Map();
for (const e of shields) seen.set(e.zat, (seen.get(e.zat) ?? 0) + 1);
const lone = (pred) => shields.find((e) => seen.get(e.zat) === 1 && e.zat > 1e7 && pred(e) && !shields.some((o) => o !== e && Math.abs(o.zat - e.zat) <= 100000));
const recent = lone((e) => sampleCov.last_t - e.t < 10 * DAY);
const old = lone((e) => sampleCov.last_t - e.t > 20 * DAY && sampleCov.last_t - e.t < 29 * DAY);
assert.ok(recent && old, "the sample has no isolated recent/old shield to test edges with");
const S = recent.zat;
const edge = (mode, z, want, c = {}) => assert.equal(same(page, SAMPLE, "edge", mode, fmt(z), c).k, want, `${mode} ${z} ${JSON.stringify(c)}`);
edge("unshield", S - 50000, 1); // X + fee allowance == pool_in: a send-max round trip is found
edge("unshield", S - 50001, 0); // one zat further: not found
edge("unshield", S, 1);
edge("unshield", S + 1, 0);
edge("shield", S, 1);
edge("shield", S - 1, 0);
edge("shield", S + 1, 0);
edge("unshield", S + 1, 1, { tol: 1 });
edge("unshield", S - 50001, 1, { tol: 1 });
edge("unshield", S - 50000, 0, { fee: 0 });
edge("unshield", old.zat - 15000, 1); // older than 14 days: weighs 0.5
edge("unshield", old.zat - 15000, 0, { look: 7 });
amounts += 12;

// ---- 2. inputs the parser must reject (Rust and JS alike) ----
const invalid = ["1e3", "+1", "1,5", "1 2", "0", "0.0", "21000000.00000001", "21000001", "1.123456789", "abc", "", ".", " ", "1.2.3", "-1", "0x10", "١٢٣", "１２", "1_000", "1 2"];
for (const s of invalid) {
  let js = true;
  try { page.parse(s); } catch { js = false; }
  assert.equal(js, false, `JS accepted ${JSON.stringify(s)}`);
  assert.equal(cli(SAMPLE, "unshield", s, {}), null, `CLI accepted ${JSON.stringify(s)}`);
}
for (const [s, z] of [["1.", 100000000], [".5", 50000000], ["   2.50　", 250000000], ["007.5", 750000000], ["0.00000001", 1], ["21000000", 2.1e15]]) assert.equal(page.parse(s), z, s);

// ---- 3. synthetic data: window edges, weights, thresholds, tie-breaks, ignored events ----
const T = 1_790_000_000;
let n = 0;
const ev = (kind, zat, t, extra = {}) => ({ h: 3_000_000 + n, t, kind, zat, pool: "sapling", fee_zat: null, n_tin: 1, n_tout: 0, txid: (n++).toString(16).padStart(64, "0"), confidence: "high", ...extra });
const zec = (x) => Math.round(x * 1e8);
const synth = [];
for (const t of [T - 30 * DAY, T - 30 * DAY - 1, T - 14 * DAY, T - 14 * DAY - 1, T, T + 1]) synth.push(ev("shield", zec(10), t)); // lookback and weight edges
for (let i = 0; i < 10; i++) synth.push(ev("shield", zec(20), T - i * 3600)); // weighted 10 -> GREEN
for (let i = 0; i < 9; i++) synth.push(ev("shield", zec(30), T - i * 3600)); // weighted 9 -> AMBER
for (let i = 0; i < 20; i++) synth.push(ev("shield", zec(40), T - 20 * DAY - i)); // 20 x 0.5 = 10 -> GREEN
for (let i = 0; i < 19; i++) synth.push(ev("shield", zec(40.5), T - 20 * DAY - i)); // 9.5 -> AMBER
for (let i = 0; i < 5; i++) { synth.push(ev("unshield", zec(50), T - i * 3600)); synth.push(ev("shield", zec(50), T - i * 3600, { confidence: "low" })); } // ignored
synth.push(ev("shield", zec(60), T - 3600));
for (const a of [60.5, 59.5, 60.4]) for (let i = 0; i < 3; i++) synth.push(ev("shield", zec(a), T - 7200 - i)); // equal crowds: closer first, then smaller
synth.push(ev("shield", zec(70), T - 100, { coinbase: true })); // coinbase shields count
const cov = { first_h: 3_000_000, first_t: T - 31 * DAY, last_h: 3_000_000 + n, last_t: T };
const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zcrowd-parity-"));
try {
  const file = path.join(dir, "synth.ndjson");
  fs.writeFileSync(file, synth.map((e) => JSON.stringify(e)).join("\n") + "\n");
  fs.writeFileSync(path.join(dir, "synth.meta.json"), JSON.stringify(cov));
  const sp = load(render(template, pageData(synth, cov)));
  const k = { 10: 4, 20: 10, 30: 9, 40: 20, 50: 0, 60: 1, 70: 1, 80: 0 };
  for (const a of [10, 20, 30, 40, 40.5, 50, 60, 70, 80]) {
    for (const mode of ["unshield", "shield"]) {
      for (const c of [{}, { look: 7 }, { look: 14 }, { tol: zec(0.5) }, { fee: 0 }, { tol: 3, fee: 1 }]) {
        const r = same(sp, file, "synthetic", mode, String(a), c);
        if (!Object.keys(c).length && mode === "shield" && k[a] !== undefined) assert.equal(r.k, k[a], `synthetic shield ${a}`);
      }
    }
  }
  assert.deepEqual(sp.check("unshield", zec(60), { lookback: 30, tol: 0, fee: 50000, now: T }).suggestions.map((s) => s.zat), [zec(60.4), zec(59.5), zec(60.5)]);
  assert.equal(sp.check("unshield", zec(20), { lookback: 30, tol: 0, fee: 50000, now: T }).verdict, "GREEN");
  assert.equal(sp.check("unshield", zec(30), { lookback: 30, tol: 0, fee: 50000, now: T }).verdict, "AMBER");
  assert.equal(sp.check("unshield", zec(50), { lookback: 30, tol: 0, fee: 50000, now: T }).verdict, "NO MATCH");
  assert.equal(sp.check("shield", zec(50), { lookback: 30, tol: 0, fee: 50000, now: T }).verdict, "RED");
} finally {
  fs.rmSync(dir, { recursive: true, force: true });
}

console.log(`PARITY OK: ${compared} JS-vs-Rust comparisons (${amounts} sample amounts, ${invalid.length} rejected inputs, synthetic edge suite); identical k, weighted, verdict, suggestions`);
