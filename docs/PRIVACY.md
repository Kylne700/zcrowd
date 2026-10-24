# zcrowd: privacy claims and the check behind each one

Rule of this repo: no privacy claim without a check that can fail. `bash tests/audit.sh` runs every check below
(no network needed; `LIVE=1` adds the live Tor leak check T3, T4, T6, which needs the network; the whole audit with
`LIVE=1`, including the parity and headless-browser steps, took 161 seconds here). It prints `AUDIT OK` only if all of them hold. A check that cannot run on a machine
is printed as `SKIP ... (NOT VERIFIED)` and counted. `bash tests/e2e.sh` runs the audit after formatting, lints,
all tests and the CLI on the committed fixtures.

## What leaves your machine

| Command | Network | What the other side learns |
|---|---|---|
| `zcrowd sync` | the first default lightwalletd server that answers (`zec.rocks`, `na.zec.rocks`, `eu.zec.rocks`; or `--server`), over TLS, through Tor (arti, inside the process). Only with `--no-tor` directly. Tor itself downloads its relay directory from, and connects to, Tor relays. | The server: the time, that someone runs zcrowd, the block range, and the ids of the public shield/unshield transactions it fetches (the same for every user of that range), and your IP address only with `--no-tor`. It learns nothing about your wallet, your amount or any address. Tor relays: that you use Tor; the exit relay also sees the server name and the timing, not the content. |
| `zcrowd check`, `report`, `verify` | none | nothing |
| `web/index.html` | none (its Content-Security-Policy forbids any request) | nothing |

Not hidden: with `--no-tor` the server sees your IP address. By default it does not, but your network provider sees
that you use Tor, the exit relay sees which server you contact and when, and someone who watches both ends can match
timing. Run `sync` on a schedule, not right before you transact. If Tor fails, `sync` stops and names `--no-tor`; it
never goes direct on its own (T1, T2).

## Claims and checks

| # | Claim | Check (all in `tests/audit.sh` unless noted) | Result on the S5 tree |
|---|---|---|---|
| P1 | `sync` asks for the same things whatever you check later: the tip, block ranges, and the ids of the candidate transactions. Its code has no input for an amount, address or txid. | Test `sync_requests_depend_only_on_range` (`crates/zcrowd-core/tests/ingest.rs`): two syncs of the same range from empty caches give identical request lists, every request is `tip`, `blocks <range>` or `tx <id>`, and the `tx` requests are exactly the candidates in order. | `ok P1` |
| P2 | `check`, `report` and `verify` cannot use the network. | (a) They run with the network removed from outside (`unshare -rn`). (b) `check.rs`, `report.rs`, `verify.rs` import no network code. (c) They cut their own network at start (`unshare(CLONE_NEWUSER \| CLONE_NEWNET)` before any thread exists); test `sandbox_blocks_connections`: a loopback connect works before and fails after. | `ok P2` x3 |
| P3 | What you type (amount, address) is never written to disk, logged, echoed back, or put in `--json` output. | Static: only `ingest.rs` (raw cache), `report.rs` (report), `sync.rs` (flows file) and `source.rs` (the Tor state directory, outside the repository, `sync` only) call file-writing APIs; arti writes inside that directory. Dynamic: a fresh random address and amount are typed via stdin and via arguments with HOME, TMPDIR and XDG_* pointing at empty directories. Afterwards no file anywhere under the repo or those directories contains them, no file was created at all, the address is not in any output, the amount is not in the `--json` output. A planted file is found by the same scan (the detector works). | `ok P3` x2 |
| P4 | zcrowd never touches key material: no seed phrase, key, viewing key or wallet file is read or accepted. | `git grep -Ei 'mnemonic\|seed\|ufvk\|viewing' crates/` is empty. The only inputs are the data file, the raw cache and what you type. | `ok P4` |
| P5 | No analytics, telemetry, update check or crash reporter. | `cargo tree --locked`: none of reqwest, sentry, posthog, segment, openssl, native-tls, hyper-tls, analytics, telemetry, updat(e). `Cargo.lock` is committed. | `ok P5`: the audit prints 525 crate versions in the tree (name and version, not lines of `cargo tree`, which repeats shared subtrees) and 624 packages in `Cargo.lock` (248 and 287 before the Tor feature) |
| P6 | TLS: the server certificate is verified against the webpki root set; plain `http://` is refused except to a lightwalletd on this machine; no certificate check is switched off. | No `accept_invalid`/`danger` in `crates/`; every default server starts with `https://`; `sync --server http://...` is refused ("refusing a plain-text connection") before any connection (run with the network removed); a local TLS server with a self-signed certificate is rejected (`invalid peer certificate: UnknownIssuer`; run with `--no-tor`, see the limits below) and nothing is written. | `ok P6` x3 |
| P7 | What is stored is public chain data only. | After a sync: every key in the flows file is one of `h t kind zat pool fee_zat n_tin n_tout txid confidence coinbase src dst`; cache files are named `<64 hex>.hex`; the directory holds exactly `flows.ndjson`, `flows.meta.json`, `tx/`. `src`/`dst` are transparent addresses that appear on the chain. | `ok P7` |
| P8 | The server is not trusted for integrity: the id of every fetched transaction is recomputed from its bytes (a wrong or unparsable one is dropped, counted, never cached); block hashes must chain without gaps. | Tests `wrong_or_broken_transactions_are_dropped_counted_and_never_cached`, `broken_block_chain_is_an_error`, `block_chain_is_checked_across_request_chunks`. `sync` prints the counts (`integrity:` line). | `ok P8`. Live one-day sync (all three pools): 2260 of 2260 transaction ids recomputed, 0 mismatches, block hash chain checked over 1297 blocks |
| P9 | The HTML report is one self-contained file. | The generated report has no `src=`, `href=`, `url(`, `http(s)://`, `@import` or `<script`. | `ok P9` |
| P10 | The web demo cannot send anything, and gives the same answers as the CLI. | Static: the CSP has `default-src 'none'`, `connect-src 'none'`, `form-action 'none'`, `base-uri 'none'`, `img-src data:`; no external reference; no `<form>`; none of fetch, XMLHttpRequest, sendBeacon, WebSocket, EventSource, storage, cookies, history, location, navigator, workers, eval, innerHTML ... in its script. Parity (`web/test/parity.mjs`): 760 comparisons against the Rust CLI (128 sample amounts, each under every pool setting, 20 rejected inputs, a synthetic edge suite with several pools) give identical pool, k, weighted, verdict, per-pool lines and suggestions. Headless Chromium (`web/test/headless.mjs`, set `PLAYWRIGHT_CORE`): zero requests while typing 1.0 and 1.23456789 and pressing Enter and while choosing each pool, URL unchanged, storage empty, injected fetch/image/beacon/WebSocket/form are blocked by the CSP, from `http://` and `file://`; the static server saw only `GET /index.html`. `node web/build.mjs` reproduces the shipped file. | `ok P10` x5 (headless: SKIP unless `PLAYWRIGHT_CORE` is set) |
| P11 | The repository carries no personal data. | No home path or personal string and no mail address in any tracked or untracked file; every commit has the same neutral author and committer; no remote; `data/` and `target/` are ignored. | `ok P11` |
| T1 | By default every request to the lightwalletd server goes through Tor (arti, inside the process). The only way to a direct connection is `--no-tor`, which prints one warning. `--tor` together with `--no-tor` is an error, and the offline fixtures mode never starts Tor. | Tests in `crates/zcrowd/tests/tor.rs`. `tor_failure_is_loud_and_never_goes_direct` runs `sync` in an empty network namespace (`unshare -rn`) under the C interposer `tests/leakcheck/leakcheck.c`, which logs every name lookup and every connect or send. The `--no-tor` control is seen looking up the server name, so the interposer does see a direct attempt; the default run makes no lookup of any name and no attempt on the server, and does try Tor relays. `tor_flags_conflict_and_offline_mode_never_starts_tor`. The success path is T3. | `ok T1/T2/T5/T7` (the audit prints SKIP where `unshare -rn` or `cc` is missing) |
| T2 | When Tor cannot start or cannot reach the server, `sync` exits with status 1 and a message that says the server was not contacted directly and names `--no-tor`. It writes no output (only Tor's own state directory). Tor start-up is limited to 120 s. | The same test: stderr has "did not bootstrap" and `--no-tor`, the exit status is 1, and the output directory does not exist (also for the `--no-tor` control); the Tor state directory does exist, with the files T4 allows. By hand, once: with a network but a local server, Tor refuses the address ("target address disabled locally") and the same message appears, nothing is written. | `ok T1/T2/T5/T7` |
| T3 | While `sync` runs over Tor, nothing looks up the server name, connects to the server's IP addresses or sends DNS (port 53). | `tests/leakcheck/run.sh` (`LIVE=1 bash tests/audit.sh`). Calibration first: a direct sync under the interposer must log the lookup of the server name and a connect to a server IP, else the check stops with LEAKCHECK BROKEN. Then a default sync of the same range with an empty Tor directory: lookups of the server name, connects to the IPs from `getent ahosts` of the three default hosts, and sends to port 53 must all be 0. Only counts are printed. | `ok T3/T4/T6` (5 runs). Calibration: 1 lookup, 1 connect to a server IP. Tor sync, every run: 0 lookups of any name, 0 of the server, 0 server-IP connects, 0 port-53 sends; 7 to 13 distinct connect destinations (Tor relays) |
| T4 | The Tor state directory holds only Tor's own files (public relay directory, guard state, locks) and nothing about the server. | `run.sh`, after the Tor sync: every file must match an allowlist (`arti-cache/dir.sqlite3` with its `-wal` and `-shm`, `dir_blobs/*`, `dir.lock`, `arti-data/state/guards.json`, `circuit_timeouts.json`, `state.lock`) and no file may contain the server name. Both checks were shown once, by hand, to fail on a planted file and a planted mention. | `ok T3/T4/T6`: 6 files, 0 mention the server name; about 39 MB; directories are mode 0700 |
| T5 | A plain-text server URL is refused before Tor starts: no relay connection, no Tor state, no output, and no advice to use `--no-tor`. | Test `plain_text_url_is_refused_before_tor_starts`, and P6 (`sync --server http://...` with the network removed). | `ok T1/T2/T5/T7` |
| T6 | The route does not change the data: a sync over Tor writes byte-identical files to a direct sync of the same range, so P1 and P8 hold for both. | `run.sh` compares `flows.ndjson` and `flows.meta.json` of the direct and the Tor sync with `cmp` (a tiny range, `--days 0`). By hand, on a fixed range (blocks 3527704..3529000: `--days 1 --end-height 3529000`): three Tor syncs (the first with an empty Tor directory) and one direct sync, each into a fresh output directory. | `ok T3/T4/T6`. By hand, all four: 1297 blocks, 6891 transactions, 172 events, 172 fetched, 0 cached, identical files. Time (debug build): Tor 28.4 s (empty Tor directory), 20.5 s, 23.3 s; direct 6.6 s; two earlier passes of three Tor runs took 22 to 40 s |
| T7 | `sync` never reads the last 10 blocks (reorg safety), even when `--end-height` asks for them. | Test `end_height_pins_the_range_and_respects_the_reorg_margin` (offline fixtures, tip 3522640): `--end-height 3522000` ends the range at 3522000; `--end-height 9999999` is cut to 3522630, with a note on stderr. | `ok T1/T2/T5/T7` |
| Q1 | Output contract (the web parity test parses it): `check` prints exactly one `verdict: <NO MATCH\|RED\|AMBER\|GREEN>` line and one `crowd: k=<n> weighted=<w>` line; `--json` has the same fields, and its `verdict` is the overall one (with `--to`, the worse of amount and address). | `crates/zcrowd/tests/cli.rs` (`contract()` in the `check` tests, JSON with `--to`) and `tests/e2e.sh`. | pass |
| Q2 | What a shield, unshield or move is worth, and in which pool, is read from the transaction itself: the value balance of each of the three shielded pools (Sapling, Orchard, Ironwood) and the transparent inputs and outputs. The books must close (transparent in + value balances = transparent out + fee, fee never negative), else the transaction is counted as skipped, never given a clamped value. | `crates/zcrowd-core/tests/decode.rs`: raw bytes of real mainnet transactions in `fixtures/raw/` (a transaction that leaves three pools at once, one that shields into Ironwood while Orchard pays out, a move Sapling to Ironwood, a two-pool unshield), each compared with what two public explorers showed on 2026-10-24; unit tests for the sign convention, the skips and the 128-bit sums. `crates/zcrowd-core/tests/sample.rs`: the 20 transactions that were compared with the explorers decode as expected in the committed sample. The Ironwood-only shield and unshield, the shielded coinbase and the two skipped transactions in `fixtures/raw/` were decoded but not compared with an explorer. | pass |
| Q3 | A crowd is counted inside one shielded pool: an unshield out of pool P is compared with the shields and moves into P only, and `--pool any` (the default) prints every pool and the worst verdict, so an Ironwood-only shield is no longer invisible. | Tests `e11` to `e15` and `worst_pool_order_and_ties` in `crates/zcrowd-core/src/crowd.rs`; `crates/zcrowd-core/tests/sample.rs` (a send-max round trip through each of more than 100 Ironwood-only shields is never a false NO MATCH; shielded coinbase; nothing entered Orchard in the sample); `crates/zcrowd/tests/cli.rs`; the web parity test runs every pool setting. | pass |
| Q4 | `sync` does not hand you an empty sample without telling you: a range of more than 100 blocks that contains no shield-like and no unshield-like transaction at all is an error, and every skipped transaction is counted by reason in the `excluded:` line. | Tests `a_long_range_without_any_candidate_is_an_error` and `candidate_kinds_cover_coinbase_and_moves` in `crates/zcrowd-core/tests/ingest.rs`, and the audit (P1) requires both to exist. A server that leaves out just one pool is not detected (see the limits). | pass |

## Honest limits (no check can remove these)

- zcrowd measures amount and timing within one shielded pool (plus address reuse with `--to`). It does not cover wallet
  and fee fingerprints, the IP address your wallet broadcasts from (use Tor in your wallet), the shielded-to-shielded
  graph, links through moves between pools beyond their amounts (a move is counted as an entry into the pool it
  enters; where the value came from is not followed), or off-chain records (an exchange knows who you are). With
  `--pool any` the worst pool decides the verdict; funds that sit in a different pool than the one you name are not
  seen by a precise `--pool` answer.
- Transactions that cross or move value in more than one pool at once are kept as low confidence ("multi-pool") and
  counted by `sync`, but no crowd includes them. Transactions whose books do not close, that move no value, or that
  carry a Sprout value are skipped and counted, never guessed. Whether the server really offers all three pools is not
  checked: `sync` stops only when a range of more than 100 blocks returns no candidate at all, so a server that leaves
  out just one pool is not detected (NOT VERIFIED).
- GREEN means "a big crowd for amount and timing". It does not mean "private".
- NO MATCH means "no shield of about that amount in the data window". It does not mean "safe": the window is the
  last `--days` of data (30 by default), and an earlier shield, or a payment pattern outside the window, is not seen.
- Address reuse is checked only with `--to`, and only against addresses seen in the data window.
- The integrity checks (P8) detect a server that contradicts itself. Compact blocks carry no proof of work, so a
  server that keeps its story consistent is not detected. The hash chain is checked inside one sync run; the link
  between two runs is not checked. Cross-check with `verify <txid>` against an explorer, or sync the same range from a
  server run by someone else (or your own) and compare the two data files. The default servers share one domain and
  are probably one operator.
- P5 is a scan by crate name. The 525 crate versions in the tree (624 packages in `Cargo.lock`) were not reviewed line
  by line, and arti, the Tor client, is not audited by us. The Tor feature added 277 crate versions to the tree and 337
  packages to the lock. `tracing-subscriber` is among them (log formatting); zcrowd uses
  no `tracing` itself and installs no subscriber, so arti's log events go nowhere.
- The sandbox in P2 (c) works on Linux where unprivileged user namespaces are allowed; otherwise it prints a note and
  continues (the commands have no network code anyway, P2 a/b). macOS and Windows are NOT VERIFIED.
- The web demo is tested in headless Chromium only. Firefox and Safari are NOT VERIFIED.
- P6 trusts the webpki root set; there is no certificate pinning. Over Tor the same roots are set by
  `zcash_client_backend` 0.25.0 (`tor/grpc.rs`; read, not run): the self-signed-certificate test uses a local server,
  which Tor refuses, so it runs with `--no-tor`, and a bad certificate over Tor is not tested.
- Tor hides your IP from the server, not the fact that you use Tor. Your network provider sees the connections to Tor
  relays (Tor's start-up talks to them directly, by design). The exit relay sees the server name, the port, and the
  timing and volume of the encrypted session. Someone who watches both your network and the server can match timing.
  The default servers are probably one operator, who sees the block range whatever you do.
- T3 is an interposer, not a packet capture (strace is not installed here). It sees libc calls
  (`getaddrinfo`, `gethostbyname`, `connect`, `sendto`, `sendmsg`, `sendmmsg`), not raw system calls, and sees DNS on
  port 53 only when the program sends it through those calls. Zero name lookups of any kind in Tor mode fits the design
  (the exit relay resolves the server name), but it is evidence, not a proof against code that bypasses libc.
- The Tor state directory is created by `source.rs` and written by arti. It is outside the repository, P3's scan lists
  `source.rs` for it, and `check`, `report` and `verify` do not link `source.rs` (P2 b). It does not need to be kept:
  delete it and the next `sync` downloads the relay directory again.
- Cost and availability: the first run downloads about 39 MB; a one-day sync took 20 to 40 s over Tor (6.6 s direct; nine runs).
  A 30-day sync over Tor was not timed (NOT VERIFIED). On a network that blocks Tor, `sync` stops after the 120 s
  start-up limit and names `--no-tor`; going direct is then your decision.
- One Tor sync hung in an early run (no CPU use for 12 minutes, then killed; the cause is unknown). It did not recur in
  more than 20 later Tor syncs. Afterwards the block stream got a 60 s idle limit (`STREAM_IDLE` in `source.rs`), which
  turns a silent stall into an error; it is NOT VERIFIED that this cures that hang.
- A direct `sync` was observed at the socket level once (sampled with `ss`, filtered to the process id): the only peers
  were the chosen server on port 443 and the DNS resolver. A sampling observation can miss a short-lived socket, so this
  is supporting evidence next to P1, not a proof. Over Tor the interposer (T3) takes this role, with the limits above.
