// Builds web/index.html: one self-contained page (CSP-locked; code and sample data inline). Usage: node web/build.mjs
import fs from "node:fs";
import { fileURLToPath } from "node:url";

const at = (p) => new URL(p, import.meta.url);
const read = (p) => fs.readFileSync(at(p), "utf8");
const MARK = "/*__DATA__*/null";

const POOLS = ["sapling", "orchard", "ironwood"]; // the index is the pool number the page script uses

// The events the page scores: high-confidence entries into a pool (a shield or a move), newest first and ties by txid, the
// order crowd.rs lists its matches in. The page keeps each entry's position as its rank, so it picks the same ten as the CLI.
export function pageEntries(events) {
  const cmp = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
  return events.filter((e) => (e.kind === "shield" || e.kind === "move") && e.confidence === "high" && POOLS.includes(e.pool)).sort((a, b) => b.t - a.t || cmp(a.txid, b.txid));
}

// The low-confidence shields and moves (one transaction into more than one pool): no entry of any pool, so the page never counts
// them in a crowd, only says how many lie in the windows of a check (crowd.rs `uncounted`, NOTES S9e F8 h). Time and amount.
export function pageLow(events) {
  return events.filter((e) => (e.kind === "shield" || e.kind === "move") && e.confidence === "low").map((e) => [e.t, e.zat]);
}

// Only what the scoring reads: time, amount and pool of each entry, in that order, and time and amount of each low-confidence one.
// No txids, no addresses.
export function pageData(events, cov) {
  const s = pageEntries(events).map((e) => [e.t, e.zat, POOLS.indexOf(e.pool)]);
  const low = pageLow(events);
  for (const [t, z] of [...s, ...low]) if (!Number.isSafeInteger(t) || !Number.isSafeInteger(z) || z <= 0) throw new Error(`not a safe integer: ${t} ${z}`);
  return { cov, s, low };
}

export function render(template, data) {
  if (!template.includes(MARK)) throw new Error("data marker missing in template");
  return template.replace(MARK, () => JSON.stringify(data));
}

if (process.argv[1] && fileURLToPath(import.meta.url) === fs.realpathSync(process.argv[1])) {
  const events = read("../fixtures/flows-sample.ndjson").split("\n").filter(Boolean).map((l) => JSON.parse(l));
  const data = pageData(events, JSON.parse(read("../fixtures/flows-sample.meta.json")));
  fs.writeFileSync(at("index.html"), render(read("index.template.html"), data));
  console.log(`web/index.html: ${data.s.length} entries, ${data.low.length} low-confidence, ${fs.statSync(at("index.html")).size} bytes`);
}
