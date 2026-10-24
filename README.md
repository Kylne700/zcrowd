# zcrowd

**How big is the crowd your Zcash unshield hides in? Find out offline, before you send.**

A Rust library and command line tool. It reads the public record of every shield (transparent to shielded) and
unshield (shielded to transparent) on Zcash mainnet, and tells you how many earlier shields look like the amount you
are about to unshield. Track: CORE & TOOLING (ZECATHON). Licence: MIT.

AI disclosure: built by Kylne700 with an AI coding assistant (Claude).

## 1. The leak

Shielded balances are private. The moment money crosses between the transparent and the shielded pool, the amount and
the time are public. If you shield 3.73 ZEC and later unshield about 3.73 ZEC, and nobody else shielded about that amount
in between, an observer who matches by amount sees a single candidate for the other half of the pair. Nothing is
broken; the amount is the fingerprint. Whether your amount sits in a crowd depends on how many other shields of about
that amount came before it.

Measured on the committed sample (`fixtures/flows-sample.ndjson`: 6,227 real events, blocks 3494295..3528999, 2026-09-24
to 2026-10-24). Each of the 1,440 unshields of the last 14 days, scored against the shields of the 14 days before it:

| verdict | unshields | share | meaning |
|---|---|---|---|
| NO MATCH | 828 | 57.5% | no shield of about that amount in the 14 days before (in this data) |
| RED | 253 | 17.6% | exactly one earlier shield of about that amount in the 14 days before |
| AMBER | 211 | 14.7% | a small crowd (more than one, fewer than ten) |
| GREEN | 148 | 10.3% | a crowd of ten or more |

So for 17.6% of recent unshields an amount-matching observer has exactly one earlier shield to look at. This does not
prove those two transactions belong to one person. It shows the size of the crowd an observer would face.
Reproduce: `cargo run -p zcrowd -- report --data fixtures/flows-sample.ndjson` and open `report.html`
(`tests/e2e.sh` asserts the RED and NO MATCH rows).

## 2. Quick start (30 seconds)

```
cargo run -p zcrowd -- check --unshield 1.0 --data fixtures/flows-sample.ndjson
```

Real output:

```
note: values given as arguments are visible in shell history and the process list; use the flag without a value (or `-`) to type it privately
check: unshield (recipient gets this amount) 1.00000000 ZEC; fee allowance 50000 zat, tolerance 0 zat, lookback 30 days
data: blocks 3494295..3528999 (2026-09-24 06:39 UTC .. 2026-10-24 13:05 UTC), 6227 events
crowd: k=0 weighted=0.0 (earlier shields of about this amount, allowing for the fee; shields older than 14 days count half)
verdict: NO MATCH - no shield of about this amount in the last 30 days; amount matching cannot tie this unshield to a shield in that window
Not checked: address reuse. Never unshield to an address you shielded from (add --to -).
popular nearby amounts: none within 10% with a bigger crowd
Covers amount + timing (+ address reuse with --to). Not covered: wallet/fee fingerprints, your IP when your wallet broadcasts (use Tor in your wallet), z->z graph, cross-pool (Sapling<->Orchard) moves, off-chain records.
```

The sample data goes stale. It ends at 2026-10-24 13:05 UTC, and `check` warns when the newest block is more than two
hours old, so when you run this later the output has extra `warning:` lines about missing recent shields. That is
expected; fetch fresh data (section 3).

Type the amount privately instead of putting it in the command line (no echo on a terminal, one line on stdin):

```
cargo run -p zcrowd -- check --unshield --data fixtures/flows-sample.ndjson
printf '%s\n%s\n' 1.2497 "$ADDRESS" | cargo run -p zcrowd -- check --unshield - --to - --data fixtures/flows-sample.ndjson
```

Other flags: `--shield X` (planning to shield: how many other shields of about X will be your decoys later),
`--tolerance-zat`, `--fee-allowance-zat`, `--lookback-days`, `--json`. `report` writes one self-contained HTML file.
`verify <txid>` re-decodes one cached raw transaction offline and compares it with the stored event.

## 3. Fresh data

```
cargo run -p zcrowd -- sync          # last 30 days, over Tor, from the first default lightwalletd server that answers; writes data/flows.ndjson and data/tx/
```

Measured here (debug build, direct connection, before Tor was the default): a 30-day sync found 198,933 transactions in
34,705 blocks, 6,227 of them crossed the turnstile; it took 140 s with 1,406 of those already cached (4,821 fetched). A
completely cold run fetches all of them: its time was not measured (NOT VERIFIED). A 30-day sync over Tor was not
measured either (NOT VERIFIED); a one-day sync took 20 to 40 s over Tor against 6.6 s direct (section 7). The raw
transaction cache is about 80 MB for 30 days (measured: 6,227 files,
76 MB of content, about 95 MB of disk with block rounding; the events file adds 1.5 MB). `sync` stops 10 blocks below
the tip (reorg safety) and uses at most 8 parallel fetches. It takes its data from the first server that answers as
mainnet (`--server`, repeatable; the default list is `zec.rocks`, `na.zec.rocks`, `eu.zec.rocks`).

Run `sync` on a schedule, not right before you transact. By default it goes through Tor, so the server does not see
your IP address; with `--no-tor` it does, and sees that you run zcrowd (see "Network privacy" in section 7).

## 4. How it works

```
lightwalletd (public, TLS) --compact blocks--> candidates --full raw tx--> events --> check / report / web
                               block hashes       txid recomputed          offline, no network
                               must chain         from the bytes
```

1. `sync` asks the first server that answers for the compact blocks of the last N days and picks the transactions that cross the
   turnstile: transparent inputs with shielded outputs (shield) or the reverse (unshield). It fetches each one in full,
   recomputes its id from the bytes, and reads the exact net amount from the Sapling and Orchard value balances. Each
   event stores time, height, kind, amount, pool, txid and the transparent addresses involved (all public on the chain).
2. `check --unshield X` counts the crowd: earlier shields in the last 30 days whose amount is in `[X, X + 50,000 zat]`
   (a "send max" round trip pays the fee out of the shielded note, so the recipient gets slightly less than was
   shielded). Shields older than 14 days count half. `weighted` is that sum.
3. Verdict: NO MATCH (k = 0), RED (weighted at most 1), AMBER (below 10), GREEN (10 or more). GREEN means a big crowd for
   amount and timing only, not "private". An amount with five or more decimals is only a note ("easy to recognise
   later, round amounts blend in"), not a verdict driver.
4. `--to ADDRESS` adds address reuse, the strongest real link: RED if that address shielded before, AMBER if it received
   earlier unshields. The address is never written, logged or sent.
5. For `--shield X` the crowd is other shields of about X (your later whole-note unshield hides among them).

The adversary model is one sentence: an observer matches an unshield to earlier shields by amount, allowing for the fee,
and weighs recent matches more. zcrowd measures this and only this.

## 5. Real output: RED and GREEN

RED (header and footer lines and part of the nearby list left out). 3.73 ZEC is an ordinary two-decimal amount, and
only one shield of about that size happened in the 30 days of the sample (the sample holds no unshield of about that
size, so this example shows no linked pair):

```
$ cargo run -p zcrowd -- check --unshield 3.73 --data fixtures/flows-sample.ndjson
crowd: k=1 weighted=1.0 (earlier shields of about this amount, allowing for the fee; shields older than 14 days count half)
matches (newest first, up to 10):
  2026-10-19 13:50 UTC  5.0 days ago  3.73000000 ZEC  6bdcbce819dd2b5a8e80126f7274d799af98d4d4d426c69b54e67eb4bd0e69b6
verdict: RED - one match: if it is yours you are linked; if not, observers may wrongly link you to it
popular nearby amounts (within 10%, bigger crowd):
  3.74975000 ZEC -> crowd 34 (weighted 25.5, GREEN)
  3.75121300 ZEC -> crowd 21 (weighted 17.5, GREEN)
  do not send several chunks at the same time or to the same address; do not unshield the exact amount you shielded; waiting helps.
```

GREEN (header and footer lines left out, and 7 of the 10 match lines):

```
$ printf '1.2497\n' | cargo run -p zcrowd -- check --unshield - --data fixtures/flows-sample.ndjson
crowd: k=157 weighted=114.5 (earlier shields of about this amount, allowing for the fee; shields older than 14 days count half)
matches (newest first, up to 10):
  2026-10-24 11:17 UTC  1.8 h ago  1.25000000 ZEC  2f77a813749805567311ae2f3993c63c050efba380f588010425a5890bfc3e47
  2026-10-24 07:32 UTC  5.6 h ago  1.25018331 ZEC  1b5e456ff6659ff015913b64a513d09aa5c2dc71d564aa01d63b2c351e5501dd
  2026-10-24 04:22 UTC  8.7 h ago  1.24985000 ZEC  d9058da18ba2c80a2cc425df1cd843d5b7d181a6bf23bc71b2502b46d0525f63
  ...
verdict: GREEN - big crowd for amount + timing only, not "private"
```

With an address that shielded earlier (taken from the sample), the overall verdict is the worst of amount and address:

```
verdict: RED (worst of amount GREEN and address RED)
amount: GREEN - big crowd for amount + timing only, not "private"
address: RED - this address shielded on 2026-10-24 07:26 UTC; unshielding back to it links you directly, whatever the amount
```

Every `check` prints exactly one `verdict: ...` line and one `crowd: k=<n> weighted=<w>` line. `--json` has the same
fields, with `verdict` the overall one (the worse of amount and address, so RED above, not the amount's GREEN), plus
`amount_verdict` and `address_verdict`; it never contains the amount or address you typed.

## 6. Library use in five lines

```rust
let data = std::path::Path::new("fixtures/flows-sample.ndjson");
let events = read_ndjson(data)?;
let now = read_coverage(data).unwrap().last_t; // the data's newest block time
let p = Params { lookback_days: 30, tol_zat: 0, fee_allowance_zat: DEFAULT_FEE_ALLOWANCE, now };
let c = score(&events, Kind::Unshield, parse_zec("1.2497")?, &p); // c.k, c.weighted, c.verdict, c.matches
```

`cargo run -q -p zcrowd-core --example five_lines` runs the whole file and prints `crowd 157 -> GREEN`. `zcrowd-core`
has no network code in the scoring path: a wallet can call `score` from its send screen.

## 7. Privacy: what leaves your machine

| Command | Network | What the other side learns |
|---|---|---|
| `sync` | the first default lightwalletd server that answers, over TLS, through Tor (built into the process); only with `--no-tor` directly | the time, that someone runs zcrowd, the block range, the ids of the public transactions it fetches: the same for every user of that range. Your IP address only with `--no-tor`. Nothing about your wallet, amount or addresses. |
| `check`, `report`, `verify` | none; they have no network code and, on Linux where unprivileged user namespaces are allowed, cut their own network at start (otherwise a note is printed and they continue) | nothing |
| `web/index.html` | none; its Content-Security-Policy forbids any request | nothing |

Every claim has a check that can fail; `docs/PRIVACY.md` lists them one by one with the result. In short:
what you type is never written, logged or put in `--json` (a fresh random address and amount are searched for in every
file afterwards); no keys, seeds or viewing keys are read; no analytics, telemetry or update-check crates in the 719-crate
tree; the TLS certificate is verified against the webpki roots, plain `http://` is refused except to a lightwalletd on
this machine, and the audit shows that a self-signed certificate is rejected; `sync` goes through Tor and never falls
back to a direct connection.

### Network privacy

`sync` is the only command that uses the network. By default all of its traffic to the lightwalletd server goes through
Tor, using the Rust Tor client (arti) built into zcrowd: no Tor daemon or proxy to install, and the server sees a Tor
exit relay instead of your IP address.

- **No fallback.** If Tor cannot start or cannot reach the server, `sync` stops with an error that says so and names
  `--no-tor`. It does not contact the server and writes nothing. Going direct is always your decision.
- **`--no-tor`** is the one way to a direct connection. It prints one warning: the server then sees your IP address and
  that you run zcrowd. Use it for a lightwalletd on this machine (`--server http://127.0.0.1:9067`; Tor refuses
  local-only addresses, and `sync` then stops with the message above) or on a network that blocks Tor. (`--tor` is
  accepted and changes nothing.)
- **Tor state** lives in `$XDG_CACHE_HOME/zcrowd/tor`, else `~/.cache/zcrowd/tor` (`--tor-dir` changes it), outside the
  repository. It holds only what Tor keeps itself: the public relay directory and guard state, about 39 MB, downloaded
  on the first run. The check lists its files and finds no mention of the server in them.
- **Cost:** a one-day sync took 20 to 40 s over Tor against 6.6 s direct (nine Tor runs, debug build; the first run
  downloads the relay directory).
- **Not hidden:** your network provider sees that you use Tor. The exit relay sees which server you contact and when, not
  what you ask for (the TLS session is between you and the server). Someone who watches both ends can still match
  timing, so keep syncing on a schedule, not right before you transact. The default servers are probably one operator.
- **Checks:** `bash tests/audit.sh` runs the failure tests. `LIVE=1 bash tests/audit.sh`, or `bash
  tests/leakcheck/run.sh`, also syncs a real range under a small C interposer (`tests/leakcheck/`) that records every
  name lookup and connection: the direct sync is seen looking up and connecting to the server, the Tor sync is not, and
  both write byte-identical files. One check per claim (T1 to T7) in `docs/PRIVACY.md`.

## 8. Trust: the lightwalletd server is not trusted

A lying server could hide shields (your crowd looks smaller; NO MATCH looks possible) or invent them (the crowd looks
bigger: a false GREEN, the worse case). What zcrowd does about it:

- It recomputes the transaction id of every fetched raw transaction from its bytes (ZIP-244 for v5, the legacy rule for
  v4). A wrong or unparsable transaction is dropped, counted in the `sync` output (`integrity:` line) and never cached.
- It requires block hashes to chain: consecutive heights, each block's previous hash equal to the hash before it, and the
  range reaching its requested end. A gap, a mismatch or a truncated answer is an error, not a partial result.

What it cannot do: compact blocks carry no proof of work, so a server that keeps its story consistent is not detected.
The hash chain is checked inside one `sync` run. When a later run extends older data, the link between the two runs is
not checked (block hashes are not stored), so a different server on the second run is not noticed. Cross-check what
matters:

1. `zcrowd verify <txid>` re-decodes a cached transaction offline; compare amount and time with a block explorer.
2. Sync the same range from a second server (`--server https://eu.zec.rocks:443 --out other/flows.ndjson`) and compare
   the two files. On 2026-10-24 a one-day sync from `zec.rocks` and from `eu.zec.rocks` gave byte-identical files
   (1,297 blocks, 173 events). Both names are under one domain and are probably one operator, so this is not an
   independent check: for a real cross-check use a server run by someone else, or your own.
3. Point `--server` at your own lightwalletd (`http://127.0.0.1:9067` is allowed, nothing else over plain text; add
   `--no-tor`, because Tor refuses local-only addresses).

## 9. Honest limits

- It measures amount and timing only. Not covered: wallet and fee fingerprints, the IP address your wallet broadcasts
  from (use Tor in your wallet), the shielded-to-shielded graph, moves between the Sapling and Orchard pools, off-chain
  records (an exchange knows who you are).
- GREEN is not "private". NO MATCH is not "safe": it means no match in the data window (30 days by default). An earlier
  shield outside the window is not seen, and the report and `check` say so when the window is shorter than the lookback.
- Address reuse is only checked with `--to`, against addresses seen in the window.
- Fees are matched with a fixed allowance. If the fee taken from the shielded note is more than 50,000 zat, the
  matching shield is not found; `--fee-allowance-zat` widens the window.
- Transactions are classified by their net crossing of the turnstile. Ambiguous ones (opposite signs in the two pools)
  are marked low confidence and excluded from scores; `sync` prints how many transactions were left out and why.
- The amounts, fees and addresses of the three fixture transactions were checked by hand against the Blockchair public
  API on 2026-10-24; the 6,227 sample events were not checked one by one. The coinbase-to-shielded and cross-pool decode
  paths have no real transaction behind them: none occurred in the 30 days of sample data.

## 10. How it is verified

```
bash tests/e2e.sh     # fmt, clippy -D warnings, all tests, the CLI on the fixtures, then the audit; prints E2E OK
bash tests/audit.sh   # every privacy claim, one check each (docs/PRIVACY.md); prints AUDIT OK
LIVE=1 bash tests/audit.sh   # also the live Tor leak check (real network; the whole audit took 54 s here)
```

- 34 Rust tests: decode of real fixture transactions (shield, unshield, shield with transparent change) and address
  shapes including TEX, scoring (send-max round
  trip, NO MATCH, 12 shields GREEN, window and future exclusion, address reuse, widening never lowers k), amount parsing
  (rejects `1e3`, `+1`, `1,5`, unicode digits, more than 8 decimals, 0, more than 21M), request independence, integrity
  and chain-break handling, the sandbox, and four Tor tests (a Tor sync that cannot start never goes direct, flag
  conflicts, the reorg margin of `--end-height`, a plain-text URL is refused before Tor starts).
- Mutation checks while building: removing the txid check, the previous-hash check, the end-of-range check, trusting the
  cache, or caching a bad transaction each makes a test fail.
- `AUDIT OK` with one headless-browser step needs `PLAYWRIGHT_CORE=<playwright-core dir>` and a Chromium binary; without
  it that step prints `SKIP ... (NOT VERIFIED)`; so does the live Tor leak check without `LIVE=1`.

## 11. Web demo

`web/index.html` is one self-contained file (data, code and style inline, no libraries, no WASM). Open it from disk or
host it as a static file. It says on the page: "This page cannot send anything: its Content-Security-Policy blocks all
network requests. Check DevTools > Network." It gives the same answers as the CLI (160 comparisons, `node
web/test/parity.mjs`). It has no `--to` address check; use the CLI. Rebuild with `node web/build.mjs`. Tested in headless
Chromium (from `http://` and `file://`); Firefox and Safari are NOT VERIFIED.

## 12. Dependencies, licence, roadmap

Rust 2024 (`cargo`), `zcash_client_backend` (lightwalletd protocol; its `tor` feature brings arti, the Rust Tor client:
the `cargo tree` crate count went from 336 to 719), `zcash_primitives` / `zcash_transparent` /
`zcash_address` (transactions, ids, addresses), `tonic` with webpki roots (gRPC over TLS), `tokio`, `clap`,
`serde_json`, `anyhow`, `rpassword`, `libc`. `Cargo.lock` is committed. MIT, see `LICENSE`.

Track: CORE & TOOLING. This is plumbing for wallet and SDK authors: a small library (`zcrowd-core`) and an indexer-fed
tool that answer one question that wallets currently cannot. Roadmap: a wallet-side hook that warns on the send
screen, pool-aware scoring.

## NOT VERIFIED

`sync` over Tor on macOS and Windows, on networks that block Tor, for a 30-day range, and from servers other than
`zec.rocks`; the cause of one hang in an early Tor sync (seen once, not reproduced in more than
20 later runs; the 60 s idle limit on the block stream was added after it and is not verified to cure it); macOS and
Windows; Firefox and Safari for the web demo; release-build performance; the
coinbase-to-shielded and cross-pool decode paths on real transactions; P2SH input-address detection beyond the sources
seen in the sample; independence of the public servers; the time of a completely cold 30-day sync; packet-level network
behaviour of `sync` (the leak-check interposer sees libc calls, not raw system calls; a direct sync was sampled once with
`ss`: only the chosen server and the DNS resolver were seen).
