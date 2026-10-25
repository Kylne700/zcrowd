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
# the stable output contract: exactly one verdict line and one crowd line, whatever else is printed
contract() {
  [ "$(grep -cE '^verdict: (NO MATCH|RED|AMBER|GREEN)( |$)' "$1")" -eq 1 ] || die "$2: not exactly one verdict line"
  [ "$(grep -cE '^crowd: k=[0-9]+ weighted=[0-9]+\.[0-9]( |$)' "$1")" -eq 1 ] || die "$2: not exactly one crowd line"
}

cargo fmt --check >"$T/fmt" 2>&1 || { head -20 "$T/fmt" >&2; die "cargo fmt --check (run cargo fmt)"; }
ok "cargo fmt --check"
cargo build -q
BIN=${CARGO_TARGET_DIR:-$PWD/target}/debug/zcrowd
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
"$BIN" check --shield 15.01006756 --pool sapling --data "$T/s/flows.ndjson" >"$T/c0" 2>&1 || die "check on synced file"
contract "$T/c0" "check --shield on synced file"
grep -q '^crowd: k=1 weighted=1\.0 ' "$T/c0" && grep -q '^verdict: AMBER' "$T/c0" || die "check --shield 15.01006756 --pool sapling: expected k=1 (the one shield of that amount) AMBER"
# one line per pool, the worst pool decides. 1.0 ZEC has no Sapling or Orchard crowd, 160 entries into Ironwood
printf '1.0\n' | "$BIN" check --unshield - --data "$D" >"$T/c1" 2>&1 || die "check 1.0"
contract "$T/c1" "check 1.0"
grep -q '^pool sapling: k=0 weighted=0\.0 -> NO MATCH' "$T/c1" && grep -q '^pool orchard: k=0 weighted=0\.0 -> NO MATCH' "$T/c1" \
  && grep -q '^pool ironwood: k=160 weighted=111\.0 -> GREEN' "$T/c1" || die "check 1.0: expected one line per pool (sapling and orchard NO MATCH, ironwood k=160 GREEN)"
grep -q '^crowd: k=160 weighted=111\.0 ' "$T/c1" && grep -q '^verdict: GREEN' "$T/c1" || die "check 1.0: expected the worst pool (ironwood) k=160 weighted=111.0 GREEN"
printf '2.74012345\n' | "$BIN" check --unshield - --data "$D" >"$T/c1b" 2>&1 || die "check 2.74012345"
contract "$T/c1b" "check 2.74012345"
grep -q '^verdict: NO MATCH' "$T/c1b" || die "check 2.74012345: expected NO MATCH (no entry of that amount in any pool)"
printf '1.2497\n' | "$BIN" check --unshield - --data "$D" >"$T/c2" 2>&1 || die "check 1.2497"
contract "$T/c2" "check 1.2497"
grep -q '^pool ironwood: k=591 weighted=438\.0 -> GREEN' "$T/c2" && grep -q '^crowd: k=157 weighted=114\.5 ' "$T/c2" && grep -q '^verdict: GREEN' "$T/c2" \
  || die "check 1.2497: expected the worst pool (sapling) k=157 weighted=114.5 GREEN, next to ironwood k=591"
# --pool answers for one pool; a crowd in another pool is no cover (125 ZEC: 32 entries into Sapling, none into Ironwood)
printf '1.2497\n' | "$BIN" check --unshield - --pool ironwood --data "$D" >"$T/c2i" 2>&1 || die "check --pool ironwood"
contract "$T/c2i" "check 1.2497 --pool ironwood"
grep -q '^crowd: k=591 weighted=438\.0 ' "$T/c2i" && grep -q '^verdict: GREEN' "$T/c2i" || die "check 1.2497 --pool ironwood: expected k=591 weighted=438.0 GREEN"
printf '1.2497\n' | "$BIN" check --unshield - --pool orchard --data "$D" >"$T/c2o" 2>&1 || die "check --pool orchard"
grep -q '^verdict: NO MATCH' "$T/c2o" || die "check 1.2497 --pool orchard: expected NO MATCH (nothing entered Orchard in the sample)"
for pp in sapling ironwood; do
  printf '125\n' | "$BIN" check --unshield - --pool $pp --data "$D" >"$T/c125$pp" 2>&1 || die "check 125 --pool $pp"
  contract "$T/c125$pp" "check 125 --pool $pp"
done
grep -q '^crowd: k=32 weighted=24\.0 ' "$T/c125sapling" && grep -q '^verdict: GREEN' "$T/c125sapling" \
  && grep -q '^crowd: k=0 weighted=0\.0 ' "$T/c125ironwood" && grep -q '^verdict: NO MATCH' "$T/c125ironwood" \
  || die "check 125: expected Sapling k=32 GREEN and Ironwood NO MATCH"
# stdout only: where a layer of the sandbox is missing, its note is on stderr and is no part of the JSON
printf '1.2497\n' | "$BIN" check --unshield - --json --data "$D" >"$T/c3" 2>"$T/c3.err" || { cat "$T/c3.err" >&2; die "check --json"; }
python3 - "$T/c3" <<'PY' || die "check --json: fields differ from the text output"
import json, sys
j = json.load(open(sys.argv[1]))
assert (j["k"], j["weighted"], j["verdict"], j["pool"]) == (157, 114.5, "GREEN", "sapling"), j
assert [(p["pool"], p["k"]) for p in j["pools"]] == [("sapling", 157), ("orchard", 0), ("ironwood", 591)], j["pools"]
PY
# the review's false NO MATCH: the send-max round trip through an Ironwood-only shield. 43bdcfaf... shielded 0.27967119 ZEC into Ironwood;
# unshielding it again with the 15000 zat fee gives 0.27952119 ZEC. The Ironwood-blind sample said NO MATCH for this amount.
printf '0.27952119\n' | "$BIN" check --unshield - --data "$D" >"$T/c2r" 2>&1 || die "check 0.27952119"
contract "$T/c2r" "check 0.27952119"
grep -q '^pool ironwood: k=3 weighted=2\.5 -> AMBER' "$T/c2r" && grep -Fq '43bdcfaf262ba320576337dc63683bf4817ca6b03b0ff3a45ac156cbbd62e9c9' "$T/c2r" \
  && ! grep -q '^verdict: NO MATCH' "$T/c2r" || die "check 0.27952119: the Ironwood-only shield 43bdcfaf... must be found (k=3 AMBER), not NO MATCH"
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
# --json must carry the same overall verdict as the text (the amount alone is GREEN, the reused address makes it RED)
printf '1.2497\n%s\n' "$ADDR" | "$BIN" check --unshield - --to - --json --data "$D" >"$T/c5" 2>"$T/c5.err" || { cat "$T/c5.err" >&2; die "check --to --json"; }
python3 - "$T/c5" <<'PY' || die "check --to --json: verdict is not the overall one"
import json, sys
j = json.load(open(sys.argv[1]))
assert (j["verdict"], j["amount_verdict"], j["address_verdict"]) == ("RED", "GREEN", "RED"), j
PY
if grep -qF "$ADDR" "$T/c5" "$T/c5.err"; then die "check --to --json echoed the address"; fi
# the README RED example: 3.73 ZEC has exactly one earlier shield of about that size in the sample, and the sample has
# no unshield of about that size at all (so the README shows a lone shield, not a linked pair)
python3 - "$D" <<'PY' || die "the sample holds an unshield of about 3.73 ZEC: the README example would show a real pair"
import json, sys
assert not [e for e in map(json.loads, open(sys.argv[1])) if e["kind"] == "unshield" and abs(e["zat"] - 373_000_000) <= 200_000]
PY
printf '3.73\n' | "$BIN" check --unshield - --data "$D" >"$T/c6" 2>&1 || die "check 3.73"
contract "$T/c6" "check 3.73"
grep -q '^crowd: k=1 weighted=1\.0 ' "$T/c6" && grep -q '^verdict: RED' "$T/c6" \
  && grep -Fq '6bdcbce819dd2b5a8e80126f7274d799af98d4d4d426c69b54e67eb4bd0e69b6' "$T/c6" \
  || die "check 3.73: the README example (k=1 weighted=1.0, RED, one match) changed"
ok "check: one verdict line and one crowd line each time, a line per pool; 1.0 GREEN (ironwood k=160); 1.2497 k=157 weighted=114.5 GREEN (text and --json); --pool; 125 ZEC Sapling k=32 vs Ironwood NO MATCH; the Ironwood-only round trip is found; 3.73 k=1 RED; --to a shield source address RED (text and --json overall verdict)"

# report and verify
"$BIN" report --data "$D" --out "$T/r.html" >/dev/null 2>&1 || die "report"
grep -Fq '<tr><td>RED</td><td>1412</td><td>8.0%</td></tr>' "$T/r.html" || die "report: the README figure (1412 RED, 8.0%) is not what the report shows"
grep -Fq '<tr><td>NO MATCH</td><td>3695</td><td>20.8%</td></tr>' "$T/r.html" || die "report: NO MATCH row differs from the README figure"
ok "report on the committed sample: 17739 unshields in the last 14 days = NO MATCH 3695 (20.8%), RED 1412 (8.0%), as the README says"
"$BIN" verify 954c19b2c73e667170c28b8d3a94fd96582cba736a5ed7275a10fc0c3e64e149 --data "$D" --raw-dir fixtures/raw >"$T/v" 2>&1 || die "verify"
grep -q '^MATCH' "$T/v" || die "verify: no MATCH line"
ok "verify of a fixture transaction: MATCH (decoded from the cached raw bytes, offline)"

# library use in five lines
cargo run -q -p zcrowd-core --example five_lines 2>/dev/null | grep -Fxq 'crowd 591 -> GREEN' || die "example five_lines"
ok "example five_lines prints: crowd 591 -> GREEN"

if [ "${LIVE:-}" = 1 ]; then
  mkdir -p "$T/live"
  "$BIN" sync --days 1 --tor-dir "$T/tor" --out "$T/live/flows.ndjson" >"$T/live.out" 2>&1 || { cat "$T/live.out" >&2; die "LIVE sync"; }
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
