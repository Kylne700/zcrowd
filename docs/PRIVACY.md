# zcrowd: privacy claims and the check behind each one

Rule of this repo: no privacy claim without a check that can fail. `bash tests/audit.sh` runs every check below
(about 15 seconds, no network needed). It prints `AUDIT OK` only if all of them hold. A check that cannot run on a
machine is printed as `SKIP ... (NOT VERIFIED)` and counted. `bash tests/e2e.sh` runs the audit after formatting, lints,
all tests and the CLI on the committed fixtures.

## What leaves your machine

| Command | Network | What the other side learns |
|---|---|---|
| `zcrowd sync` | one lightwalletd server over TLS (default `zec.rocks`, or `--server`) | your IP address, the time, that someone runs zcrowd, the block range, and the ids of the public shield/unshield transactions it fetches. This is the same for every user of that range. It learns nothing about your wallet, your amount or any address. |
| `zcrowd check`, `report`, `verify` | none | nothing |
| `web/index.html` | none (its Content-Security-Policy forbids any request) | nothing |

Not hidden: the server sees your IP address during `sync`. Run `sync` on a schedule (not right before you transact),
through a VPN or Tor at system level. `torsocks` use is NOT VERIFIED. Built-in Tor support is not part of this
commit.

## Claims and checks

| # | Claim | Check (all in `tests/audit.sh` unless noted) | Result on the S5 tree |
|---|---|---|---|
| P1 | `sync` asks for the same things whatever you check later: the tip, block ranges, and the ids of the candidate transactions. Its code has no input for an amount, address or txid. | Test `sync_requests_depend_only_on_range` (`crates/zcrowd-core/tests/ingest.rs`): two syncs of the same range from empty caches give identical request lists, every request is `tip`, `blocks <range>` or `tx <id>`, and the `tx` requests are exactly the candidates in order. | `ok P1` |
| P2 | `check`, `report` and `verify` cannot use the network. | (a) They run with the network removed from outside (`unshare -rn`). (b) `check.rs`, `report.rs`, `verify.rs` import no network code. (c) They cut their own network at start (`unshare(CLONE_NEWUSER \| CLONE_NEWNET)` before any thread exists); test `sandbox_blocks_connections`: a loopback connect works before and fails after. | `ok P2` x3 |
| P3 | What you type (amount, address) is never written to disk, logged, echoed back, or put in `--json` output. | Static: only `ingest.rs` (raw cache), `report.rs` (report) and `sync.rs` (flows file) call file-writing APIs. Dynamic: a fresh random address and amount are typed via stdin and via arguments with HOME, TMPDIR and XDG_* pointing at empty directories. Afterwards no file anywhere under the repo or those directories contains them, no file was created at all, the address is not in any output, the amount is not in the `--json` output. A planted file is found by the same scan (the detector works). | `ok P3` x2 |
| P4 | zcrowd never touches key material: no seed phrase, key, viewing key or wallet file is read or accepted. | `git grep -Ei 'mnemonic\|seed\|ufvk\|viewing' crates/` is empty. The only inputs are the data file, the raw cache and what you type. | `ok P4` |
| P5 | No analytics, telemetry, update check or crash reporter. | `cargo tree --locked`: none of reqwest, sentry, posthog, segment, openssl, native-tls, hyper-tls, analytics, telemetry, updat(e). `Cargo.lock` is committed. | `ok P5` (336 distinct crates) |
| P6 | TLS: the server certificate is verified against the webpki root set; plain `http://` is refused except to a lightwalletd on this machine; no certificate check is switched off. | No `accept_invalid`/`danger` in `crates/`; every default server starts with `https://`; `sync --server http://...` is refused ("refusing a plain-text connection") before any connection (run with the network removed); a local TLS server with a self-signed certificate is rejected (`invalid peer certificate: UnknownIssuer`) and nothing is written. | `ok P6` x3 |
| P7 | What is stored is public chain data only. | After a sync: every key in the flows file is one of `h t kind zat pool fee_zat n_tin n_tout txid confidence coinbase src dst`; cache files are named `<64 hex>.hex`; the directory holds exactly `flows.ndjson`, `flows.meta.json`, `tx/`. `src`/`dst` are transparent addresses that appear on the chain. | `ok P7` |
| P8 | The server is not trusted for integrity: the id of every fetched transaction is recomputed from its bytes (a wrong or unparsable one is dropped, counted, never cached); block hashes must chain without gaps. | Tests `wrong_or_broken_transactions_are_dropped_counted_and_never_cached`, `broken_block_chain_is_an_error`, `block_chain_is_checked_across_request_chunks`. `sync` prints the counts (`integrity:` line). | `ok P8`. Live one-day sync: 173 of 173 transaction ids recomputed, 0 mismatches, block hash chain checked over 1297 blocks |
| P9 | The HTML report is one self-contained file. | The generated report has no `src=`, `href=`, `url(`, `http(s)://`, `@import` or `<script`. | `ok P9` |
| P10 | The web demo cannot send anything, and gives the same answers as the CLI. | Static: the CSP has `default-src 'none'`, `connect-src 'none'`, `form-action 'none'`, `base-uri 'none'`, `img-src data:`; no external reference; no `<form>`; none of fetch, XMLHttpRequest, sendBeacon, WebSocket, EventSource, storage, cookies, history, location, navigator, workers, eval, innerHTML ... in its script. Parity (`web/test/parity.mjs`): 160 comparisons against the Rust CLI (38 sample amounts, 20 rejected inputs, a synthetic edge suite) give identical k, weighted, verdict and suggestions. Headless Chromium (`web/test/headless.mjs`, set `PLAYWRIGHT_CORE`): zero requests while typing 1.0 and 1.23456789 and pressing Enter, URL unchanged, storage empty, injected fetch/image/beacon/WebSocket/form are blocked by the CSP, from `http://` and `file://`; the static server saw only `GET /index.html`. `node web/build.mjs` reproduces the shipped file. | `ok P10` x5 (headless: SKIP unless `PLAYWRIGHT_CORE` is set) |
| P11 | The repository carries no personal data. | No home path or personal string and no mail address in any tracked or untracked file; every commit has the same neutral author and committer; no remote; `data/` and `target/` are ignored. | `ok P11` |
| Q1 | Output contract (the web parity test parses it): `check` prints exactly one `verdict: <NO MATCH\|RED\|AMBER\|GREEN>` line and one `crowd: k=<n> weighted=<w>` line; `--json` has the same fields. | `crates/zcrowd/tests/cli.rs` (`contract()` in the three `check` tests) and `tests/e2e.sh`. | pass |

## Honest limits (no check can remove these)

- zcrowd measures amount and timing only. It does not cover wallet and fee fingerprints, the IP address your wallet
  broadcasts from (use Tor in your wallet), the shielded-to-shielded graph, moves between the Sapling and Orchard
  pools, or off-chain records (an exchange knows who you are).
- GREEN means "a big crowd for amount and timing". It does not mean "private".
- NO MATCH means "no shield of about that amount in the data window". It does not mean "safe": the window is the
  last `--days` of data (30 by default), and an earlier shield, or a payment pattern outside the window, is not seen.
- Address reuse is checked only with `--to`, and only against addresses seen in the data window.
- The integrity checks (P8) detect a server that contradicts itself. Compact blocks carry no proof of work, so a
  server that keeps its story consistent is not detected. Cross-check with `verify <txid>` against an explorer, or
  sync the same range from a second server (`na.zec.rocks`, `eu.zec.rocks`) and compare the two data files.
- P5 is a scan by crate name. The 336 crates in the tree were not reviewed line by line.
- The sandbox in P2 (c) works on Linux where unprivileged user namespaces are allowed; otherwise it prints a note and
  continues (the commands have no network code anyway, P2 a/b). macOS and Windows are NOT VERIFIED.
- The web demo is tested in headless Chromium only. Firefox and Safari are NOT VERIFIED.
- P6 trusts the webpki root set; there is no certificate pinning.
- `sync` was observed at the socket level once (sampled with `ss`, filtered to the process id): the only peers were the
  chosen server on port 443 and the DNS resolver. A sampling observation can miss a short-lived socket, so this is
  supporting evidence next to P1, not a proof.
