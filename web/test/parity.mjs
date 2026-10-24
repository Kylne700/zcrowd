// Parity: the script shipped in web/index.html must give the same crowd, verdict, pools, suggestions and the same ten matches
// in the same order (newest first, ties by txid) as the Rust CLI, for every --pool setting.
// Usage: cargo build -p zcrowd && node web/test/parity.mjs   (ZCROWD_BIN overrides the binary path)
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import vm from "node:vm";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { pageData, pageEntries, render } from "../build.mjs";

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

function load(html, events) {
  const scripts = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)];
  assert.equal(scripts.length, 1, "exactly one inline script");
  const ctx = vm.createContext({}); // no document: only the scoring part of the script runs
  vm.runInContext(scripts[0][1], ctx);
  const ev = (code) => vm.runInContext(code, ctx);
  const plain = (x) => JSON.parse(JSON.stringify(x)); // drop the other realm's prototypes
  const [parse, check] = [ev("parseZec"), ev("check")];
  const order = pageEntries(events).map((e) => e.txid); // the page keeps no txids, only each match's rank in this list
  return { parse, check: (...a) => plain(check(...a)), now: ev("NOW"), txids: (r) => r.matches.map((m) => order[m.rank]) };
}

function cli(data, mode, amount, c) {
  const flags = [];
  if (c.tol !== undefined) flags.push(`--tolerance-zat=${c.tol}`);
  if (c.fee !== undefined) flags.push(`--fee-allowance-zat=${c.fee}`);
  if (c.look !== undefined) flags.push(`--lookback-days=${c.look}`);
  if (c.pool !== undefined) flags.push(`--pool=${c.pool}`);
  const r = spawnSync(BIN, ["check", `--${mode}=${amount}`, "--json", "--data", data, ...flags], { cwd: ROOT, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
  return r.status === 0 ? JSON.parse(r.stdout) : null;
}

let compared = 0;
const ties = { sample: 0, synthetic: 0 }; // comparisons whose listed matches include two with the same time
function same(page, data, label, mode, amount, c = {}) {
  const want = cli(data, mode, amount, c);
  const settings = { lookback: c.look ?? 30, tol: c.tol ?? 0, fee: c.fee ?? 50000, now: page.now };
  if (mode === "shield" && c.pool === "orchard") { // a shield cannot go into Orchard (ZIP 258): the CLI and the page both refuse
    assert.equal(want, null, `${label}: the CLI accepted a shield into Orchard (${amount})`);
    assert.throws(() => page.check(mode, page.parse(amount), settings, "orchard"), /Orchard/, `${label}: the page accepted a shield into Orchard (${amount})`);
    compared++;
    return null;
  }
  assert.ok(want, `${label}: the CLI rejected a valid amount ${amount}`);
  const got = page.check(mode, page.parse(amount), settings, c.pool ?? "any");
  const view = (r, matches) => ({ pool: r.pool, k: r.k, weighted: r.weighted, verdict: r.verdict, odd_amount: r.odd_amount, matches, pools: r.pools, other_pools_k: r.other_pools_k, suggestions: r.suggestions });
  assert.deepStrictEqual(view(got, page.txids(got)), view(want, want.matches), `${label}: ${mode} ${amount} ${JSON.stringify(c)}`);
  compared++;
  if (got.matches.some((m, i) => i > 0 && m.t === got.matches[i - 1].t)) ties[label === "synthetic" ? label : "sample"]++;
  return got;
}

const fmt = (z) => `${Math.floor(z / 1e8)}.${String(z % 1e8).padStart(8, "0")}`;
const POOLS = ["sapling", "orchard", "ironwood"];
const POOL_SETTINGS = ["any", ...POOLS];

// ---- 1. the real sample ----
const page = load(shipped, sampleEvents);
let amounts = 0;
for (const pool of POOL_SETTINGS) {
  for (const a of ["1.0", "1.23456789", "1.2497", "1.24985", "1.25", "2.5", "0.001", "125", "5", "0.0001", "100", "3.74975", "4.9997", "1.", ".5", " 007.50 ", "21000000", "0.00000001", "1.00015", "0.01015", "49.99999999"]) {
    same(page, SAMPLE, "sample", "unshield", a, { pool });
    amounts++;
  }
  for (const a of ["1.24985", "1.0", "2.4998", "125", "4.9997"]) { same(page, SAMPLE, "sample", "shield", a, { pool }); amounts++; }
  for (const c of [{ tol: 10000 }, { tol: 100000000 }, { fee: 0 }, { fee: 10000 }, { look: 7 }, { look: 14 }, { look: 45 }]) {
    same(page, SAMPLE, "sample", "unshield", "1.2497", { ...c, pool });
    same(page, SAMPLE, "sample", "shield", "1.24985", { ...c, pool });
  }
}

// real entries (shields and moves) whose amount no other entry into the same pool has, so the window edges are exact
const entries = sampleEvents.filter((e) => (e.kind === "shield" || e.kind === "move") && e.confidence === "high" && POOLS.includes(e.pool));
const seen = new Map();
for (const e of entries) seen.set(e.pool + e.zat, (seen.get(e.pool + e.zat) ?? 0) + 1);
const lone = (pool, pred) => entries.find((e) => e.pool === pool && seen.get(e.pool + e.zat) === 1 && e.zat > 1e7 && pred(e) && !entries.some((o) => o !== e && o.pool === pool && Math.abs(o.zat - e.zat) <= 100000));
let edgePools = 0;
for (const pool of POOLS) {
  const recent = lone(pool, (e) => sampleCov.last_t - e.t < 10 * DAY);
  const old = lone(pool, (e) => sampleCov.last_t - e.t > 20 * DAY && sampleCov.last_t - e.t < 29 * DAY);
  if (!recent || !old) { assert.notEqual(pool, "ironwood", "the sample has no isolated recent/old Ironwood entry to test edges with"); continue; }
  edgePools++;
  const S = recent.zat;
  const edge = (mode, z, want, c = {}) => assert.equal(same(page, SAMPLE, "edge " + pool, mode, fmt(z), { ...c, pool }).k, want, `${pool} ${mode} ${z} ${JSON.stringify(c)}`);
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
}
assert.ok(edgePools >= 2, "the sample has isolated entries in fewer than two pools");
assert.ok(ties.sample > 0, "no sample comparison listed two matches of the same time: the txid tie-break is not tested on real data");

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
const ev = (kind, zat, t, extra = {}) => ({ h: 3_000_000 + n, t, kind, zat, pool: "sapling", fee_zat: null, n_tin: kind === "move" ? 0 : 1, n_tout: 0, txid: (n++).toString(16).padStart(64, "0"), confidence: "high", ...extra });
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
// 15 entries of one amount in the same second, txids descending (the data order is not the txid order): the ten listed are
// picked by txid, as crowd.rs does
const tied = [];
for (let j = 0; j < 15; j++) { const e = ev("shield", zec(90), T - 2 * DAY, { txid: (0xabcdef - j).toString(16).padStart(64, "0") }); tied.push(e.txid); synth.push(e); }
// several pools: 25 ZEC entered Sapling once, Orchard 4 times (one a coinbase), Ironwood 11 times (5 of them moves from another pool)
const into = (pool, kind, zat, i, extra = {}) => ev(kind, zat, T - i * 3600, { pool, ...extra });
synth.push(into("sapling", "shield", zec(25), 0));
for (let i = 0; i < 4; i++) synth.push(into("orchard", "shield", zec(25), i, i === 0 ? { coinbase: true } : {}));
for (let i = 0; i < 6; i++) synth.push(into("ironwood", "shield", zec(25), i));
for (let i = 0; i < 5; i++) synth.push(into("ironwood", "move", zec(25), i));
// no entries: a low-confidence event of any pool, a multi-pool one, an unshield (from the pool) and a Low-confidence move
synth.push(into("mixed", "shield", zec(25), 1, { confidence: "low" }), into("ironwood", "shield", zec(25), 2, { confidence: "low" }), into("ironwood", "unshield", zec(25), 3), into("orchard", "move", zec(25), 4, { confidence: "low" }));
// ties between pools: 26 ZEC entered each pool twice; 27 ZEC entered Orchard and Ironwood twice, Sapling never
for (const pool of POOLS) for (let i = 0; i < 2; i++) synth.push(into(pool, "shield", zec(26), i));
for (const pool of ["orchard", "ironwood"]) for (let i = 0; i < 2; i++) synth.push(into(pool, "move", zec(27), i));
const cov = { version: 2, first_h: 3_000_000, first_t: T - 31 * DAY, last_h: 3_000_000 + n, last_t: T };
const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zcrowd-parity-"));
try {
  const file = path.join(dir, "synth.ndjson");
  fs.writeFileSync(file, synth.map((e) => JSON.stringify(e)).join("\n") + "\n");
  fs.writeFileSync(path.join(dir, "synth.meta.json"), JSON.stringify(cov));
  const sp = load(render(template, pageData(synth, cov)), synth);
  const k = { 10: 4, 20: 10, 30: 9, 40: 20, 50: 0, 60: 1, 70: 1, 80: 0, 90: 15 }; // Sapling only
  const P = { lookback: 30, tol: 0, fee: 50000, now: T };
  for (const a of [10, 20, 30, 40, 40.5, 50, 60, 70, 80, 90, 25, 26, 27]) {
    for (const pool of POOL_SETTINGS) {
      for (const mode of ["unshield", "shield"]) {
        for (const c of [{}, { look: 7 }, { look: 14 }, { tol: zec(0.5) }, { fee: 0 }, { tol: 3, fee: 1 }]) {
          const r = same(sp, file, "synthetic", mode, String(a), { ...c, pool });
          if (!Object.keys(c).length && mode === "shield" && pool === "sapling" && k[a] !== undefined) assert.equal(r.k, k[a], `synthetic shield ${a}`);
        }
      }
    }
  }
  const smallest = [...tied].sort().slice(0, 10);
  assert.deepEqual(sp.txids(sp.check("unshield", zec(90), P, "sapling")), smallest);
  assert.notDeepEqual(tied.slice(0, 10), smallest, "the tied txids are in data order: the test cannot tell the tie-break from the data order");
  assert.deepEqual(sp.check("unshield", zec(60), P, "sapling").suggestions.map((s) => s.zat), [zec(60.4), zec(59.5), zec(60.5)]);
  assert.equal(sp.check("unshield", zec(20), P, "sapling").verdict, "GREEN");
  assert.equal(sp.check("unshield", zec(30), P, "sapling").verdict, "AMBER");
  assert.equal(sp.check("unshield", zec(50), P).verdict, "NO MATCH");
  assert.equal(sp.check("shield", zec(50), P).verdict, "RED");
  // a crowd is the entries into one pool: shields and moves count (a shielded coinbase too), nothing else does
  const r25 = sp.check("unshield", zec(25), P);
  assert.deepEqual(r25.pools.map((o) => [o.pool, o.k, o.verdict]), [["sapling", 1, "RED"], ["orchard", 4, "AMBER"], ["ironwood", 11, "GREEN"]]);
  assert.deepEqual([r25.pool, r25.verdict, r25.other_pools_k], ["sapling", "RED", 15]); // any = the worst pool
  assert.deepEqual(["sapling", "orchard", "ironwood"].map((x) => sp.check("unshield", zec(25), P, x).verdict), ["RED", "AMBER", "GREEN"]);
  // a shield cannot go into Orchard (ZIP 258): `any` is the worse of Sapling and Ironwood, whatever Orchard holds
  const s25 = sp.check("shield", zec(25), P);
  assert.deepEqual(s25.pools.map((o) => [o.pool, o.k, o.verdict]), [["sapling", 1, "AMBER"], ["ironwood", 11, "GREEN"]]);
  assert.deepEqual([s25.pool, s25.verdict, s25.other_pools_k], ["sapling", "AMBER", 11]);
  assert.throws(() => sp.check("shield", zec(25), P, "orchard"), /Orchard/);
  assert.equal(sp.check("unshield", zec(26), P).pool, "sapling"); // same verdict and weight in every pool: the first pool
  assert.equal(sp.check("unshield", zec(27), P).pool, "orchard"); // Sapling has no match, Orchard and Ironwood tie
  assert.equal(sp.check("shield", zec(27), P).pool, "sapling"); // nobody else entered Sapling with 27: RED in shield mode (Ironwood has 2: AMBER)
} finally {
  fs.rmSync(dir, { recursive: true, force: true });
}
assert.ok(ties.synthetic > 0, "no synthetic comparison listed two matches of the same time");

console.log(`PARITY OK: ${compared} JS-vs-Rust comparisons (${amounts} sample amounts, ${invalid.length} rejected inputs, synthetic edge suite); identical pool, k, weighted, verdict, pools, suggestions and ordered match txids (${ties.sample} sample and ${ties.synthetic} synthetic comparisons with a time tie)`);
