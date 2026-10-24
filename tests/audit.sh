#!/usr/bin/env bash
# Checks the claims of docs/PRIVACY.md one by one. Prints "AUDIT OK" when every claim holds, stops at the first that does not.
# Needs: cargo, git, python3. Uses when present: unshare, cc, openssl, node. A check that cannot run is printed as SKIP and
# counted as NOT VERIFIED. PLAYWRIGHT_CORE=<a playwright-core directory> adds the headless browser test of web/index.html.
set -euo pipefail
cd "$(dirname "$0")/.."

T=$(mktemp -d)
SRV=""
cleanup() { [ -z "$SRV" ] || kill "$SRV" 2>/dev/null || true; rm -rf "$T"; }
trap cleanup EXIT
SKIPS=0
ok() { printf 'ok    %s\n' "$*"; }
skip() { SKIPS=$((SKIPS + 1)); printf 'SKIP  %s (NOT VERIFIED)\n' "$*"; }
die() { printf 'FAIL  %s\n' "$*" >&2; exit 1; }
# cargo test must pass AND run at least one test
must_pass() {
  local label=$1
  shift
  "$@" >"$T/test.out" 2>&1 || { tail -30 "$T/test.out" >&2; die "$label"; }
  grep -Eq 'test result: ok\. [1-9][0-9]* passed' "$T/test.out" || die "$label: no test ran"
}

cargo build -q
BIN=${ZCROWD_BIN:-$PWD/target/debug/zcrowd}
DATA=fixtures/flows-sample.ndjson
TXID=954c19b2c73e667170c28b8d3a94fd96582cba736a5ed7275a10fc0c3e64e149
ok "build"

# P1: the requests sync makes depend only on the block range
cargo test -q -p zcrowd-core --test ingest -- --list >"$T/list" 2>/dev/null
for t in sync_requests_depend_only_on_range wrong_or_broken_transactions_are_dropped_counted_and_never_cached \
  broken_block_chain_is_an_error block_chain_is_checked_across_request_chunks; do
  grep -q "^$t: test" "$T/list" || die "P1/P8: test $t is missing"
done
must_pass "P1/P8 ingest tests" cargo test -q -p zcrowd-core --test ingest
ok "P1 sync requests depend only on the block range (test sync_requests_depend_only_on_range)"

# P2: check, report and verify have no network
if unshare -rn true 2>/dev/null; then
  unshare -rn "$BIN" check --unshield 1.0 --data "$DATA" >"$T/o" 2>&1 && grep -q '^verdict: ' "$T/o" || die "P2 check under unshare -rn"
  unshare -rn "$BIN" report --data "$DATA" --out "$T/net.html" >"$T/o" 2>&1 || die "P2 report under unshare -rn"
  unshare -rn "$BIN" verify "$TXID" --data "$DATA" --raw-dir fixtures/raw >"$T/o" 2>&1 && grep -q '^MATCH' "$T/o" || die "P2 verify under unshare -rn"
  ok "P2 check, report and verify run with the network removed from outside (unshare -rn)"
else
  skip "P2 unshare -rn is not available: outside network cut not tested"
fi
if git grep -nE 'use .*(source|tonic|tokio|std::net)|Lwd|TcpStream|UdpSocket' -- crates/zcrowd/src/check.rs crates/zcrowd/src/report.rs crates/zcrowd/src/verify.rs; then
  die "P2 check, report or verify refers to network code"
fi
ok "P2 check.rs, report.rs and verify.rs import no network code (no source, tonic, tokio, std::net, Lwd, sockets)"
must_pass "P2 sandbox test" cargo test -q -p zcrowd --test cli sandbox_blocks_connections
ok "P2 the process cuts its own network: a loopback connection works before and fails after (test sandbox_blocks_connections)"

# P3: what you type is never written. Static part: only sync, report, the raw cache and the Tor state directory write files.
W=$(git grep -lE 'fs::write|File::create|OpenOptions|create_dir|DirBuilder|fs::rename|fs::copy|fs::remove' -- 'crates/*/src/*.rs' | sort | tr '\n' ' ')
[ "$W" = "crates/zcrowd-core/src/ingest.rs crates/zcrowd-core/src/source.rs crates/zcrowd/src/report.rs crates/zcrowd/src/sync.rs " ] || die "P3 the set of files that write has changed: $W"
ok "P3 only ingest.rs (raw cache), report.rs (report), sync.rs (flows) and source.rs (Tor state directory, sync only) write files; check.rs and verify.rs cannot"

# P3 dynamic part: a fresh random address and amount, typed through stdin, must not appear in any file afterwards
# and no file may be created at all (a transformed write would escape a content search).
read -r ADDR AMT ZAT H160 < <(python3 - <<'PY'
import hashlib, os, random
A = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
h = os.urandom(20)
p = b"\x1c\xb8" + h
p += hashlib.sha256(hashlib.sha256(p).digest()).digest()[:4]
n, s = int.from_bytes(p, "big"), ""
while n:
    n, r = divmod(n, 58)
    s = A[r] + s
z = random.SystemRandom().randrange(10**8, 10**9)
print(s, "%d.%08d" % (z // 10**8, z % 10**8), z, h.hex())
PY
)
[[ $ADDR == t1* ]] || die "P3 test address generator is broken: $ADDR"
# scan <dir> [pathspec...]: succeeds when any typed value occurs in a file under <dir>
scan() { local d=$1; shift; (cd "$d" && git grep --no-index -lF -e "$ADDR" -e "$AMT" -e "$ZAT" -e "$H160" -- . "$@"); }
if scan . ':!target' >/dev/null 2>&1; then die "P3 the random test values were already in the tree (run again)"; fi
# the detector itself must work: a file holding the values is found
mkdir -p "$T/canary" && printf '%s %s %s %s\n' "$ADDR" "$AMT" "$ZAT" "$H160" >"$T/canary/leak"
scan "$T/canary" >/dev/null 2>&1 || die "P3 the leak detector does not find a planted file"
mkdir -p "$T/env/home" "$T/env/tmp" "$T/env/cache" "$T/env/data" "$T/env/config" "$T/env/state"
typed() { # args... ; stdin from the caller
  env HOME="$T/env/home" TMPDIR="$T/env/tmp" XDG_CACHE_HOME="$T/env/cache" XDG_DATA_HOME="$T/env/data" \
    XDG_CONFIG_HOME="$T/env/config" XDG_STATE_HOME="$T/env/state" "$BIN" "$@"
}
sleep 1
touch "$T/stamp"
sleep 1
printf '%s\n%s\n' "$AMT" "$ADDR" | typed check --unshield - --to - --data "$DATA" >"$T/t1" 2>&1 || die "P3 check --unshield - --to -"
printf '%s\n%s\n' "$AMT" "$ADDR" | typed check --unshield - --to - --json --data "$DATA" >"$T/t2" 2>&1 || die "P3 check --json"
printf '%s\n' "$AMT" | typed check --shield - --data "$DATA" >"$T/t3" 2>&1 || die "P3 check --shield -"
typed check --unshield "$AMT" --to "$ADDR" --data "$DATA" >"$T/t4" 2>&1 || die "P3 check with arguments"
grep -q '^verdict: ' "$T/t1" && grep -q '^crowd: k=' "$T/t1" || die "P3 check printed no verdict"
for f in t1 t2 t3 t4; do
  if grep -qF -e "$ADDR" -e "$H160" "$T/$f"; then die "P3 the address was echoed in the output ($f)"; fi
done
if grep -qF -e "$AMT" -e "$ZAT" "$T/t2"; then die "P3 the typed amount is in the --json output"; fi
grep -q 'shell history' "$T/t4" || die "P3 passing values as arguments printed no warning"
if scan . ':!target'; then die "P3 a typed value was written into the repository tree"; fi
if scan "$T/env"; then die "P3 a typed value was written into the fresh HOME, TMPDIR or XDG directories"; fi
[ -z "$(find "$T/env" -type f)" ] || die "P3 files appeared in the fresh HOME, TMPDIR or XDG directories"
NEW=$(find . -newer "$T/stamp" -type f -not -path './target/*' -not -path './.git/*' | head -5)
[ -z "$NEW" ] || die "P3 check created or changed files: $NEW"
ok "P3 random address and amount typed via stdin and arguments: not in any file, no file created, address not echoed, not in --json"

# P4: key material is never read
if git grep -nEi 'mnemonic|seed|ufvk|viewing' -- crates; then die "P4 key-material words in crates/"; fi
ok "P4 no mnemonic, seed, viewing-key or ufvk anywhere in crates/"

# P5: no analytics, telemetry, update checks or crash reporters
git ls-files --error-unmatch Cargo.lock >/dev/null 2>&1 || die "P5 Cargo.lock is not tracked"
cargo tree --locked --prefix none >"$T/tree" 2>"$T/tree.err" || { cat "$T/tree.err" >&2; die "P5 cargo tree failed"; }
[ "$(wc -l <"$T/tree")" -gt 100 ] || die "P5 cargo tree output looks empty"
if grep -Ei '^(reqwest|sentry|posthog|segment|openssl|openssl-sys|native-tls|hyper-tls) |analytics|telemetry|updat' "$T/tree"; then
  die "P5 a forbidden dependency is in the tree"
fi
ok "P5 $(sort -u "$T/tree" | wc -l) distinct crates in cargo tree, none of reqwest, sentry, posthog, segment, openssl, native-tls, analytics, telemetry, update; Cargo.lock tracked"

# P6: TLS is verified, plain text is refused
if git grep -nE 'accept_invalid|danger' -- crates; then die "P6 certificate checks are switched off somewhere"; fi
git grep -q 'with_webpki_roots' -- crates/zcrowd-core/src/source.rs || die "P6 no webpki roots in source.rs"
if git grep -n 'pub const DEFAULT_SERVERS' -- crates/zcrowd-core/src/source.rs | grep -o '"[^"]*"' | grep -v '^"https://'; then
  die "P6 a default server is not https"
fi
if unshare -rn true 2>/dev/null; then
  unshare -rn "$BIN" sync --server http://zcrowd-audit.invalid --days 1 --out "$T/plain/flows.ndjson" >"$T/o" 2>&1 && die "P6 plain http was accepted"
  grep -q 'refusing a plain-text connection' "$T/o" || { cat "$T/o" >&2; die "P6 plain http refusal message missing"; }
  ok "P6 sync --server http://... is refused before any connection (run with the network removed)"
else
  skip "P6 plain-http refusal (needs unshare -rn so a regression cannot reach a real host)"
fi
if command -v openssl >/dev/null; then
  (cd "$T" && openssl req -x509 -newkey rsa:2048 -nodes -keyout k.pem -out c.pem -days 1 -subj "/CN=localhost" \
    -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" -addext "basicConstraints=critical,CA:FALSE" >/dev/null 2>&1)
  cat >"$T/srv.py" <<'PY'
import socket, ssl, sys
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain(sys.argv[1] + "/c.pem", sys.argv[1] + "/k.pem")
ctx.set_alpn_protocols(["h2"])
s = socket.socket(); s.bind(("127.0.0.1", 0)); s.listen(5)
print(s.getsockname()[1], flush=True)
s.settimeout(40)
try:
    while True:
        c, _ = s.accept()
        try:
            ctx.wrap_socket(c, server_side=True).recv(1)
        except Exception:
            pass
except socket.timeout:
    pass
PY
  python3 "$T/srv.py" "$T" >"$T/srv.port" &
  SRV=$!
  for _ in $(seq 50); do [ -s "$T/srv.port" ] && break; sleep 0.1; done
  PORT=$(head -1 "$T/srv.port")
  timeout 30 "$BIN" sync --no-tor --server "https://127.0.0.1:$PORT" --days 1 --out "$T/tls/flows.ndjson" >"$T/o" 2>&1 && die "P6 a self-signed server was accepted"
  grep -q 'UnknownIssuer' "$T/o" || { cat "$T/o" >&2; die "P6 the self-signed server was not rejected for its certificate"; }
  [ ! -e "$T/tls/flows.ndjson" ] || die "P6 data was written from an untrusted server"
  kill "$SRV" 2>/dev/null || true
  SRV=""
  ok "P6 a local TLS server with a self-signed certificate is rejected (invalid peer certificate: UnknownIssuer)"
else
  skip "P6 self-signed certificate rejection (needs openssl)"
fi

# P7: what is stored is public chain data
mkdir -p "$T/sync"
ZCROWD_OFFLINE=1 "$BIN" sync --days 2 --out "$T/sync/flows.ndjson" >"$T/o" 2>&1 || { cat "$T/o" >&2; die "P7 offline sync"; }
python3 - "$T/sync" <<'PY' || die "P7 stored data has fields or files beyond public chain data"
import json, os, re, sys
d = sys.argv[1]
allowed = {"h", "t", "kind", "zat", "pool", "fee_zat", "n_tin", "n_tout", "txid", "confidence", "coinbase", "src", "dst"}
rows = [json.loads(l) for l in open(d + "/flows.ndjson")]
assert rows, "no events"
assert all(set(r) <= allowed for r in rows), "unexpected keys"
names = os.listdir(d + "/tx")
assert names and all(re.fullmatch(r"[0-9a-f]{64}\.hex", n) for n in names), "cache files are not named by txid"
assert sorted(os.listdir(d)) == ["flows.meta.json", "flows.ndjson", "tx"], "unexpected files"
PY
ok "P7 flows file keys, cache file names and the directory listing contain only public chain fields"
ok "P8 wrong txid dropped and counted, broken prevHash and gaps are errors (tests wrong_or_broken_transactions_are_dropped_counted_and_never_cached, broken_block_chain_is_an_error, block_chain_is_checked_across_request_chunks)"

# P9: the report is one self-contained file
"$BIN" report --data "$DATA" --out "$T/r.html" >/dev/null 2>&1 || die "P9 report"
if grep -nE 'src=|href=|url\(|https?://|@import|<script' "$T/r.html"; then die "P9 the report loads or contains something external"; fi
ok "P9 report HTML has no src, href, url(), http(s), @import or script"

# P10: the web demo cannot send anything
H=web/index.html
grep -q 'http-equiv="Content-Security-Policy"' "$H" || die "P10 no CSP meta"
for d in "default-src 'none'" "connect-src 'none'" "form-action 'none'" "base-uri 'none'" "img-src data:"; do
  grep -qF "$d" "$H" || die "P10 CSP lacks $d"
done
if grep -oE '(src|href|action|poster|srcset)="[^"]*"' "$H" | grep -v '="data:'; then die "P10 the page references an external resource"; fi
if grep -nE 'https?://|url\(|@import' "$H"; then die "P10 the page contains a URL"; fi
sed -n '/<script>/,/<\/script>/p' "$H" >"$T/page.js"
[ "$(wc -c <"$T/page.js")" -gt 10000 ] || die "P10 could not extract the page script"
if grep -nE 'fetch\(|XMLHttpRequest|sendBeacon|WebSocket|EventSource|localStorage|sessionStorage|indexedDB|document\.cookie|history\.|location|navigator\.|importScripts|new Worker|postMessage|window\.open|\.submit\(|eval\(|new Function|innerHTML|document\.write' "$T/page.js"; then
  die "P10 the page script uses a network, storage or navigation API"
fi
if ! git grep -q '<form' -- "$H"; then :; else die "P10 the page has a <form>"; fi
ok "P10 web/index.html: CSP with connect-src, form-action, base-uri none; no external reference; no network, storage or navigation API in its script; no <form>"
if command -v node >/dev/null; then
  node web/test/parity.mjs >"$T/o" 2>&1 || { tail -20 "$T/o" >&2; die "P10 JS/Rust parity"; }
  ok "P10 $(grep '^PARITY OK' "$T/o")"
  node web/build.mjs >/dev/null 2>&1 && git diff --quiet -- web/index.html || die "P10 web/index.html is not what web/build.mjs produces"
  ok "P10 web/index.html equals what web/build.mjs builds from the template and the committed sample"
else
  skip "P10 JS/Rust parity and rebuild check (needs node)"
fi
if [ -n "${PLAYWRIGHT_CORE:-}" ] && command -v node >/dev/null; then
  node web/test/headless.mjs >"$T/o" 2>&1 || { tail -30 "$T/o" >&2; die "P10 headless browser test"; }
  grep -q '^HEADLESS OK' "$T/o" || die "P10 headless test did not finish"
  ok "P10 headless Chromium: zero requests while typing, CSP blocks injected fetch, image, beacon, WebSocket and form (http:// and file://)"
else
  skip "P10 headless browser test (set PLAYWRIGHT_CORE to a playwright-core directory)"
fi

# P11: repository hygiene
if git grep -nI --untracked --exclude-standard -E '/ho''me/|@gm''ail|iv''ory' -- .; then die "P11 a personal string is in the repository"; fi
if git grep -nI --untracked --exclude-standard -E '[[:alnum:]._%+-]+@[[:alnum:].-]+\.[[:alpha:]]{2,}' -- .; then die "P11 an email-like string is in the repository"; fi
IDS=$(git log --format='%an|%cn|%ae|%ce' | tr '|' '\n' | sort -u)
echo "$IDS" | grep -qx 'Kylne700' || die "P11 commit name is not the neutral name"
[ "$(echo "$IDS" | grep -vc -E '^(Kylne700|[0-9]+\+Kylne700@users\.noreply\.github\.com)$')" -eq 0 ] || die "P11 a commit has another author or committer identity: $IDS"
[ -z "$(git remote)" ] || die "P11 the repository has a remote"
git check-ignore -q data/x && git check-ignore -q target/x || die "P11 data/ and target/ are not ignored"
ok "P11 no home path, mail address or personal string in tracked or untracked files; $(git rev-list --count HEAD) commits, one neutral identity; no remote; data/ and target/ ignored"

# T1-T7: sync goes through Tor by default and never falls back to a direct connection
if unshare -rn true 2>/dev/null && command -v cc >/dev/null; then
  must_pass "T1/T2/T5/T7 Tor tests" cargo test -q -p zcrowd --test tor
  ok "T1/T2/T5/T7 empty network namespace: the default sync fails loudly (names --no-tor), looks up no server name, resolves no name locally, writes nothing; --no-tor is the control the shim sees; also flag conflicts, plain-text URL refused before Tor starts, --end-height reorg margin (crates/zcrowd/tests/tor.rs)"
else
  skip "T1/T2/T5/T7 Tor failure test (needs unshare -rn and a C compiler for the leak-check shim)"
fi
if [ "${LIVE:-}" = 1 ]; then
  tests/leakcheck/run.sh "$BIN" >"$T/o" 2>&1 || { cat "$T/o" >&2; die "T3/T4/T6 live leak check"; }
  grep -q '^LEAKCHECK OK' "$T/o" || { cat "$T/o" >&2; die "T3/T4/T6 live leak check did not finish"; }
  ok "T3/T4/T6 $(grep '^Tor sync:' "$T/o" | sed 's/^Tor sync: //'); $(grep '^Tor directory:' "$T/o" | sed 's/^Tor directory: //')"
else
  skip "T3/T4/T6 live Tor leak check and Tor directory contents (needs the network and a C compiler: LIVE=1 bash tests/audit.sh)"
fi

if [ "$SKIPS" -eq 0 ]; then
  echo "AUDIT OK"
else
  echo "AUDIT OK ($SKIPS check(s) skipped and NOT VERIFIED, see the SKIP lines above)"
fi
