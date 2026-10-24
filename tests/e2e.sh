#!/usr/bin/env bash
# End to end: format, build, lint, every test, the real binary on the committed fixtures, then tests/audit.sh.
# Prints "E2E OK". Offline unless LIVE=1 (adds a real one-day sync against a public lightwalletd).
# PLAYWRIGHT_CORE=<a playwright-core directory> adds the headless browser test (run inside audit.sh).
set -euo pipefail
cd "$(dirname "$0")/.."

T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
ok() { printf 'ok    %s\n' "$*"; }
die() { printf 'FAIL  %s\n' "$*" >&2; exit 1; }
# spec 41 A10: exactly one verdict line and one crowd line, whatever else is printed
contract() {
  [ "$(grep -cE '^verdict: (NO MATCH|RED|AMBER|GREEN)( |$)' "$1")" -eq 1 ] || die "$2: not exactly one verdict line"
  [ "$(grep -cE '^crowd: k=[0-9]+ weighted=[0-9]+\.[0-9]( |$)' "$1")" -eq 1 ] || die "$2: not exactly one crowd line"
}

cargo fmt --check >"$T/fmt" 2>&1 || { head -20 "$T/fmt" >&2; die "cargo fmt --check (run cargo fmt)"; }
ok "cargo fmt --check"
cargo build -q
BIN=$PWD/target/debug/zcrowd
cargo clippy -q --all-targets -- -D warnings >"$T/clippy" 2>&1 || { head -30 "$T/clippy" >&2; die "cargo clippy -D warnings"; }
ok "cargo clippy --all-targets -D warnings"
cargo test -q >"$T/test.out" 2>&1 || { tail -40 "$T/test.out" >&2; die "cargo test"; }
if grep -qE 'FAILED|panicked|[1-9][0-9]* failed' "$T/test.out"; then die "cargo test reports failures"; fi
N=$(awk '/^test result: ok\./ { p += $4 } END { print p + 0 }' "$T/test.out")
[ "$N" -gt 20 ] || die "cargo test ran only $N tests"
ok "cargo test: $N tests passed, 0 failed"

# offline sync from the committed fixtures: 3 events, range ends at the fixture tip minus 10 blocks, txids recomputed
D=fixtures/flows-sample.ndjson
mkdir -p "$T/s"
ZCROWD_OFFLINE=1 "$BIN" sync --days 2 --out "$T/s/flows.ndjson" >"$T/sync.out" 2>&1 || { cat "$T/sync.out" >&2; die "offline sync"; }
grep -Eq 'range=[0-9]+\.\.3522630( |$)' "$T/sync.out" || die "offline sync: range does not end at tip-10"
grep -Fq 'events=3 shields=2 unshields=1' "$T/sync.out" || die "offline sync: unexpected event counts"
grep -Eq '^integrity: .* for 3 of 3 candidates, 0 did not match' "$T/sync.out" || die "offline sync: integrity line"
python3 - "$T/s/flows.ndjson" <<'PY' || die "offline sync: the shield 954c19b2... was not decoded as expected"
import json, sys
ev = {e["txid"]: e for e in map(json.loads, open(sys.argv[1]))}
s = ev["954c19b2c73e667170c28b8d3a94fd96582cba736a5ed7275a10fc0c3e64e149"]
assert s["kind"] == "shield" and s["zat"] == 1501006756, s
assert len(ev) == 3
PY
ok "offline sync from fixtures: 3 events, range ends at tip-10 (3522630), 3 of 3 txids recomputed, 0 mismatches"

# check on the freshly synced file and on the committed sample, via stdin (nothing typed goes into a process argument)
"$BIN" check --shield 15.01006756 --data "$T/s/flows.ndjson" >"$T/c0" 2>&1 || die "check on synced file"
contract "$T/c0" "check --shield on synced file"
grep -q '^crowd: k=1 weighted=1\.0 ' "$T/c0" && grep -q '^verdict: AMBER' "$T/c0" || die "check --shield 15.01006756: expected k=1 (the one shield of that amount) AMBER"
printf '1.0\n' | "$BIN" check --unshield - --data "$D" >"$T/c1" 2>&1 || die "check 1.0"
contract "$T/c1" "check 1.0"
grep -q '^verdict: NO MATCH' "$T/c1" || die "check 1.0: expected NO MATCH"
printf '1.2497\n' | "$BIN" check --unshield - --data "$D" >"$T/c2" 2>&1 || die "check 1.2497"
contract "$T/c2" "check 1.2497"
grep -q '^crowd: k=157 weighted=114\.5 ' "$T/c2" && grep -q '^verdict: GREEN' "$T/c2" || die "check 1.2497: expected k=157 weighted=114.5 GREEN"
printf '1.2497\n' | "$BIN" check --unshield - --json --data "$D" >"$T/c3" 2>&1 || die "check --json"
python3 - "$T/c3" <<'PY' || die "check --json: fields differ from the text output"
import json, sys
j = json.load(open(sys.argv[1]))
assert (j["k"], j["weighted"], j["verdict"]) == (157, 114.5, "GREEN"), j
PY
ADDR=$(python3 - <<'PY'
import json
for l in open("fixtures/flows-sample.ndjson"):
    e = json.loads(l)
    if e["kind"] == "shield" and e.get("src"):
        print(e["src"][0])
        break
PY
)
printf '1.2497\n%s\n' "$ADDR" | "$BIN" check --unshield - --to - --data "$D" >"$T/c4" 2>&1 || die "check --to"
contract "$T/c4" "check --to"
grep -q '^verdict: RED' "$T/c4" && grep -q '^address: RED - this address shielded on ' "$T/c4" || die "check --to: reuse of a shield source address must be RED"
if grep -qF "$ADDR" "$T/c4"; then die "check --to echoed the address"; fi
ok "check: one verdict line and one crowd line each time; 1.0 NO MATCH; 1.2497 k=157 weighted=114.5 GREEN (text and --json); --to a shield source address RED"

# report and verify
"$BIN" report --data "$D" --out "$T/r.html" >/dev/null 2>&1 || die "report"
grep -Fq '<tr><td>RED</td><td>253</td><td>17.6%</td></tr>' "$T/r.html" || die "report: the README figure (253 RED, 17.6%) is not what the report shows"
grep -Fq '<tr><td>NO MATCH</td><td>828</td><td>57.5%</td></tr>' "$T/r.html" || die "report: NO MATCH row differs from the README figure"
ok "report on the committed sample: 1440 unshields in the last 14 days = NO MATCH 828 (57.5%), RED 253 (17.6%), as the README says"
"$BIN" verify 954c19b2c73e667170c28b8d3a94fd96582cba736a5ed7275a10fc0c3e64e149 --data "$D" --raw-dir fixtures/raw >"$T/v" 2>&1 || die "verify"
grep -q '^MATCH' "$T/v" || die "verify: no MATCH line"
ok "verify of a fixture transaction: MATCH (decoded from the cached raw bytes, offline)"

# library use in five lines
cargo run -q -p zcrowd-core --example five_lines 2>/dev/null | grep -Fxq 'crowd 157 -> GREEN' || die "example five_lines"
ok "example five_lines prints: crowd 157 -> GREEN"

if [ "${LIVE:-}" = 1 ]; then
  mkdir -p "$T/live"
  "$BIN" sync --days 1 --out "$T/live/flows.ndjson" >"$T/live.out" 2>&1 || { cat "$T/live.out" >&2; die "LIVE sync"; }
  grep -Eq '^integrity: .* for ([0-9]+) of \1 candidates, 0 did not match .*block hash chain checked over [0-9]+ blocks' "$T/live.out" \
    || { cat "$T/live.out" >&2; die "LIVE sync: integrity line"; }
  printf '1.0\n' | "$BIN" check --unshield - --data "$T/live/flows.ndjson" >"$T/live.chk" 2>&1 || die "LIVE check"
  contract "$T/live.chk" "LIVE check"
  ok "LIVE one-day sync: $(grep -E '^integrity:' "$T/live.out" | sed 's/^integrity: //')"
else
  printf 'skip  LIVE=1 not set: no real server was contacted by this run (NOT VERIFIED here)\n'
fi

bash tests/audit.sh
echo "E2E OK"
