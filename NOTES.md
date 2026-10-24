# zcrowd build notes (for the lead and the reviewers)

Specs followed: `research/39-zecathon-design.md`, then `research/41-zcrowd-privacy-spec.md` (binding, overrides 39).
Everything below was run on this machine on 2026-10-24. Lines marked "recorded" were printed during the named stage and
not re-run later. S1..S4b are history: they come from the first, Ironwood-blind sample (6,227 events), so their counts
(events, `PARITY OK: 160`, crowd sizes) are not today's. Today's numbers are in "Ironwood stage" and in "Final run",
which is the output of `bash tests/e2e.sh` with LIVE=1 and PLAYWRIGHT_CORE set.

## Stage results (real output lines)

- S1 (workspace, lightwalletd source, candidate crawl, scoring core). Recorded: `zcrowd sync --days 7 --candidates-only` ->
  `blocks=8065 txs=46626 shield_like=689 unshield_like=703 range=3520918..3528982 secs=3.1`.
- S2 (exact amounts from full transactions, addresses, raw cache, offline verify). Recorded: 7-day sync
  `events=1406 shields=691 unshields=715 fetched_txs=1406 ... secs=40.8`; `verify 954c19b2...` MATCH. 30-day sync:
  `blocks=34705 txs=198933 shield_like=3156 (coinbase 0) unshield_like=3071 range=3494295..3528999 secs=9.5`,
  `events=6227 shields=3144 unshields=3083 fetched_txs=4821 cached_txs=1406 secs=139.6`.
- S3 (scoring per spec 41). Recorded: `check --unshield 1.0` NO MATCH; `1.23456789` NO MATCH plus the odd-amount note;
  `1.2497` crowd 157, weighted 114.5, GREEN; `1.2497 --to <an address that shielded 2026-10-19>` RED.
- S4 (HTML report). Recorded: `unshare -rn target/debug/zcrowd check|report` exit 0; report 3.4 KB without external
  references.
- S4b (web demo). Recorded: `PARITY OK: 160 JS-vs-Rust comparisons (38 sample amounts, 20 rejected inputs, synthetic edge
  suite); identical k, weighted, verdict, suggestions` (7 of 8 deliberate JS mutations caught; the survivor is
  behaviour-equivalent); `HEADLESS OK` (http and file://: 1 request at load, 0 while typing, 5 CSP violations recorded
  for the injected attempts, the static server log shows only `GET /index.html`, storage empty).
- S5 (A9, A10, audit, e2e, docs). Recorded: cross-check run once: one-day sync from `zec.rocks` and from `eu.zec.rocks` (range 3527732..3529028): byte-identical
  flows files (173 events). Mutation checks of the audit itself: a `fetch(` added to the page script -> P10 fails;
  the plain-text guard disabled -> P6 fails.

## Ironwood stage (spec 41 A11; research/43 S1-1, S1-2, S1-3, S2-4, S3-2, S3-4)

Cause: the first `sync` did not ask for the third shielded pool, Ironwood (transaction v6), and the decoder did not read
its value balance. Transactions through Ironwood were missing or showed another amount: research/44 found 4 of 20
sampled events wrong (`91c68e50...`, `c292feb1...`, `48ca0a26...`, `ce7e196a...`), research/43 S1-1 a false NO MATCH.

Commits (all by the neutral identity, Sonnet 5.5 trailer):

```
acfb52f Decode and fetch all three shielded pools (Ironwood), spec 41 A11 step 1
6acb46f Score crowds per shielded pool, add check --pool (spec 41 A11 step 3)
27bb88f Apply rustfmt and fix one clippy lint in the pool-aware scoring code
79be5c6 report: score each unshield against entries of about its amount only
b3124d7 Re-sync the sample with all three shielded pools and test it against research/44
2995ba7 Web demo: pick the pool, one line per pool, the worst pool decides (spec 41 A11)
d5d3939 tests: e2e and audit expect the Ironwood-aware sample and the per-pool output
```

(the commit that holds this file and the README / PRIVACY changes follows them.)

The sample was synced again for the same 30 days (blocks 3494295..3528999, 34,705 blocks, `--no-tor` for the bulk run
as the lead allowed): 67,977 candidates, every transaction id recomputed from the raw bytes (0 mismatches), block hash
chain checked over all 34,705 blocks. Result by case:

| case | events |
|---|---|
| shield into Ironwood | 25,506 (1,564 of them shielded coinbase) |
| shield into Sapling | 3,144 |
| shield into Orchard | 0 |
| unshield from Ironwood | 33,413 |
| unshield from Sapling | 2,496 |
| unshield from Orchard | 443 |
| low confidence, "multi-pool" (pool Mixed, kept, not counted in crowds) | 140 (all unshield-like) |
| move between pools | 2,823 (Ironwood 2,091, Sapling 732) |
| skipped, `no_net_crossing` | 4 |
| skipped, `no_transparent_part` | 8 |
| skipped, `conservation_fails`, `sprout_value`, `txid_mismatch` | 0, 0, 0 |
| total | 67,977 candidates = 67,965 events + 12 skipped |

Before and after:

| | Ironwood-blind | with all three pools |
|---|---|---|
| events in the sample | 6,227 (3,144 shields, 3,083 unshields, 0 moves) | 67,965 (28,650 shields, 36,492 unshields, 2,823 moves) |
| `fixtures/flows-sample.ndjson` | 1,595,277 bytes | 17,770,521 bytes |
| `web/index.html` | 86,615 bytes | 765,793 bytes (31,473 entries into pools) |
| `report`: unshields of the last 14 days | 1,440 | 17,827 |
| NO MATCH / RED / AMBER / GREEN | 828 (57.5%) / 253 (17.6%) / 211 (14.7%) / 148 (10.3%) | 3,642 (20.4%) / 1,405 (7.9%) / 2,561 (14.4%) / 10,219 (57.3%) |
| `check --unshield 1.0` | NO MATCH | GREEN through Ironwood (k=162, weighted 112.0) |
| research/44 events decoded as the explorers show | 16 of 20 | 20 of 20 (test `sample.rs`) |
| JS-vs-Rust comparisons | 160 | 760 |

Decisions:

- Event #7 of research/44 (`48ca0a26...`): the explorer shows the net crossing 12,272,904, the data holds the Ironwood
  shield 12,319,850. A11 counts what arrives in the pool, and Orchard pays out 46,946 in the same transaction. This is
  the one deliberate difference; `--shield X` likewise means the amount that arrives in the pool (S2-4).
- Of the 20 events, 13 decode as High events and 7 as Low "multi-pool" events (the explorer amount is then the sum of
  the leaving balances). Low events are kept, never counted in a crowd.
- A transaction whose books do not close (`t_in + sum(value balances) = t_out + fee`) is counted as a skip, never
  clamped. None occurs in the sample.
- `--pool any` prints one `pool <name>:` line per pool. The single `verdict:` / `crowd:` line, the matches and the
  suggestions come from the worst pool (RED, then AMBER, GREEN, NO MATCH; a tie goes to the smaller weighted crowd,
  then to the first of sapling, orchard, ironwood). `--json` has `pool`, `pools` and `other_pools_k`.
- S3-2: `sync` fails when a range of more than 100 blocks has no shield-like and no unshield-like candidate at all.
  S3-4: a coinbase candidate is `CompactTx.index == 0`; the decoder sets `coinbase` from the transparent bundle.
- Speed: `report` scores each unshield against a per-pool amount-sorted index (45 s -> 0.9 s on the new sample) and the
  web page binary-searches the amount window (worst key 8.6 s -> about 15 ms).
- Orchard has no entry in the sample (test `nothing_entered_orchard_in_the_sample`): Orchard shield paths are covered by
  synthetic tests only.
- Five raw fixtures were added (`43bdcfaf...` Ironwood-only shield, `6262a45c...` shielded coinbase into Ironwood,
  `f319fec5...` Ironwood-only unshield, `b2374054...` no net crossing, `eababfcb...` no transparent part). The
  reviewer's repro (40 shields of blocks 3528000..3528199) is a test over all 126 Ironwood shields of that range on the
  sample: none gives a false NO MATCH.

Left for the next stage, not touched: S1-4, S2-1, S2-2, S2-3, S3-1, S3-3, S3-5, S3-6, S3-7, S4.

NOT VERIFIED in this stage: the five new fixtures were not compared with an explorer (the 20 events of research/44 were);
the server's version or pool support is not read, so a server that leaves out only one pool is not detected; Firefox and
Safari; a 30-day sync over Tor.

## Final run (after the last code commit, d5d3939)

Real output of `PLAYWRIGHT_CORE=<playwright-core dir> LIVE=1 bash tests/e2e.sh` on 2026-10-24, all 32 lines, nothing
removed or condensed; exit code 0, 395 s in all (network: the live one-day sync and the Tor leak check):

```
ok    cargo fmt --check
ok    cargo clippy --all-targets -D warnings
ok    cargo test: 63 tests passed, 0 failed
ok    offline sync from fixtures: 3 events, range ends at tip-10 (3522630), 3 of 3 txids recomputed, 0 mismatches
ok    check: one verdict line and one crowd line each time, a line per pool; 1.0 GREEN (ironwood k=162); 1.2497 k=157 weighted=114.5 GREEN (text and --json); --pool; 125 ZEC Sapling k=40 vs Ironwood NO MATCH; the Ironwood-only round trip is found; 3.73 k=1 RED; --to a shield source address RED (text and --json overall verdict)
ok    report on the committed sample: 17827 unshields in the last 14 days = NO MATCH 3642 (20.4%), RED 1405 (7.9%), as the README says
ok    verify of a fixture transaction: MATCH (decoded from the cached raw bytes, offline)
ok    example five_lines prints: crowd 592 -> GREEN
ok    LIVE one-day sync: transaction id recomputed from the raw bytes for 2260 of 2260 candidates, 0 did not match (dropped); block hash chain checked over 1297 blocks
ok    build
ok    P1 sync requests depend only on the block range (test sync_requests_depend_only_on_range)
ok    P2 check, report and verify run with the network removed from outside (unshare -rn)
ok    P2 check.rs, report.rs and verify.rs import no network code (no source, tonic, tokio, std::net, Lwd, sockets)
ok    P2 the process cuts its own network: a loopback connection works before and fails after (test sandbox_blocks_connections)
ok    P3 only ingest.rs (raw cache), report.rs (report), sync.rs (flows) and source.rs (Tor state directory, sync only) write files; check.rs and verify.rs cannot
ok    P3 random address and amount typed via stdin and arguments: not in any file, no file created, address not echoed, not in --json
ok    P4 no mnemonic, seed, viewing-key or ufvk anywhere in crates/
ok    P5 525 crate versions in cargo tree (624 packages in Cargo.lock), none of reqwest, sentry, posthog, segment, openssl, native-tls, analytics, telemetry, update; Cargo.lock tracked
ok    P6 sync --server http://... is refused before any connection (run with the network removed)
ok    P6 a local TLS server with a self-signed certificate is rejected (invalid peer certificate: UnknownIssuer)
ok    P7 flows file keys, cache file names and the directory listing contain only public chain fields
ok    P8 wrong txid dropped and counted, broken prevHash and gaps are errors (tests wrong_or_broken_transactions_are_dropped_counted_and_never_cached, broken_block_chain_is_an_error, block_chain_is_checked_across_request_chunks)
ok    P9 report HTML has no src, href, url(), http(s), @import or script
ok    P10 web/index.html: CSP with connect-src, form-action, base-uri none; no external reference; no network, storage or navigation API in its script; no <form>
ok    P10 PARITY OK: 760 JS-vs-Rust comparisons (128 sample amounts, 20 rejected inputs, synthetic edge suite); identical pool, k, weighted, verdict, pools, suggestions
ok    P10 web/index.html equals what web/build.mjs builds from the template and the committed sample
ok    P10 headless Chromium: zero requests while typing, CSP blocks injected fetch, image, beacon, WebSocket and form (http:// and file://)
ok    P11 no home path, mail address or personal string in tracked or untracked files; 19 commits, one neutral identity; no remote; data/ and target/ ignored
ok    T1/T2/T5/T7 empty network namespace: the default sync fails loudly (names --no-tor), looks up no server name, resolves no name locally, writes no output; --no-tor is the control the shim sees; also flag conflicts, plain-text URL refused before Tor starts, --end-height reorg margin (crates/zcrowd/tests/tor.rs)
ok    T3/T4/T6 lookups (any name)=0, lookups of the server name=0, connects to server IPs=0, port-53 sends=0, distinct connect destinations (Tor relays)=9; 6 files, all arti directory data, guard state or locks; 0 mention the server name
AUDIT OK
E2E OK
```

`PLAYWRIGHT_CORE=<playwright-core dir> LIVE=1 bash tests/audit.sh` alone: `AUDIT OK`, exit code 0, 161 s, no `SKIP` line.
Not run in this stage: the audit with a mutated page or guard (done once in S5, recorded above), Firefox, Safari.

## Deviations from the specs (and why)

1. `--unshield 1.23456789` prints NO MATCH plus the odd-amount note, not RED (spec 39 check; spec 41 A4 made odd amounts a
   note and A2 made k = 0 a NO MATCH).
2. `verify` is fully offline from the raw cache (spec 41 C2) instead of fetching the transaction online (spec 39).
3. Unshield `zat` is the net pool decrease (fee included). `fee_zat` is stored only when the transaction has no
   transparent inputs (otherwise it cannot be known exactly). The scorer subtracts a known fee.
4. `now` for scoring is the data's newest block time (coverage `last_t`), not the wall clock; the wall clock only drives
   the stale-data warnings. This keeps results reproducible on the committed sample.
5. Coinbase-to-shielded transactions are counted as shields and flagged `coinbase` (detected as the first transaction of
   a block, `CompactTx.index == 0`). Low confidence ("multi-pool", pool Mixed) is used for transactions that cross or move
   value in more than one shielded pool without being a plain shield, unshield or move (spec 41 A11). Transactions that
   cannot be decoded or whose books do not close are counted by reason in the `sync` output, never silently dropped
   (before the Ironwood stage the first sentence said no coinbase occurred and Low was only for opposite-sign
   transactions: that was the Ironwood-blind sample).
6. Suggestions offer only amounts that shields actually used; "split into chunks" advice was skipped (spec 41 A7 warns
   against it).
7. `--json` omits the typed amount and address. `--amount-stdin` (spec 39) is replaced by `--unshield -` / no value
   (spec 41 C4).
8. Coverage is stored in a sidecar `<data>.meta.json`; if a new sync leaves a gap to the old range, the old data is
   dropped (a gap would make NO MATCH untrue).
9. The offline fixtures are 5 block summaries (heights 3520931, 3520943, 3520949, 3522616, 3522640) and 14 raw
   transactions in `fixtures/raw/` (4 sit behind those blocks; the other 10 are decode tests, among them 5 added in the
   Ironwood stage). The last block is padding so that tip - 10 keeps the others. The hidden `fixture` dev command that
   fetched them was used once and deleted. The sample (`flows-sample.ndjson`) contains public transparent addresses.
10. Web demo is a JS port, not WASM, so `'wasm-unsafe-eval'` is dropped from the CSP. The data is inlined (spec 41 D1
    supersedes spec 39's "fetch a data file"). It has no `--to` support.
11. Spec 39 checklist item 4 (`mnemonic|seed|ufvk|viewing|address` in `crates/`) is narrowed to
    `mnemonic|seed|ufvk|viewing`, because spec 41 B stores and accepts addresses on purpose.
12. The report gained a table of how the unshields of the last 14 days score (the number in the README).
13. E10 hand check: the three fixture transactions were compared with the Blockchair API on 2026-10-24 (in the test
    comments).
14. `docs/demo-script.md` was not written: the video belongs to S6.
15. New, beyond the spec (found while testing): tonic silently sends plain-text HTTP/2 for an `http://` URL even with a
    TLS config. `require_tls` (in `crates/zcrowd-core/src/source.rs`) refuses `http://` unless the host is `localhost`
    or a loopback IP. Test `plain_text_only_to_this_machine`; the audit runs the refusal with the network removed.
16. A9 (spec 41): transaction ids are recomputed (ZIP-244 / legacy) and mismatches dropped and counted; the block hash
    chain is checked only on the network path (`BlockSource::strict_chain`; the offline fixtures are single blocks).
    `BlockSummary` gained optional `hash` / `prev_hash` fields (old fixtures still load). `sync` prints an `integrity:`
    line.
17. A10 (spec 41): the `crowd:` line is `crowd: k=<n> weighted=<w>` followed by an explanation in parentheses. The web
    parity test uses `check --json` as its stable format; the text contract is asserted in `crates/zcrowd/tests/cli.rs`
    and `tests/e2e.sh`.
18. Audit differs from the spec-39 list: it numbers claims P1..P11 (docs/PRIVACY.md), adds "no file is created by
    `check`" (stamp-file test), a leak-detector canary, and runs the plain-text refusal under `unshare -rn`.
19. Commit trailers: S1..S3 carry the Opus 5.5 co-author trailer, S4 onward the Sonnet 5.5 one (instruction changed
    between stages). Author and committer are the neutral identity in every commit; no remote exists.
20. Review follow-up (one commit after the S5 commit), real bug: `check --json --to <address>` printed the verdict of the
    amount alone, so a reused address showed a false GREEN in JSON while the text said RED. Fixed in `check.rs`:
    `verdict` is now the overall one (the worse of amount and address), and `amount_verdict` and `address_verdict`
    (null without `--to`) were added. Covered by `cli_check_shield_mode_and_address_reuse` (JSON with `--to` gives
    RED / GREEN / RED and does not contain the address) and a step in `tests/e2e.sh`. The web parity test is
    unaffected (no `--to`, so the overall verdict equals the amount verdict).
21. README examples: the first RED example turned out to be the amount of a real round trip in the sample (an unshield of
    almost exactly the same amount a few minutes after the shield), and the second pick (a round 3.5 ZEC) also has a
    later unshield of the same net amount. Both were dropped. The final example, `check --unshield 3.73`, is a lone
    shield with no unshield of about that size anywhere in the sample; `tests/e2e.sh` asserts that, so the README never
    shows a linked pair. The web demo's RED example buttons use the newest unique shield minus 15,000 zat; the page holds
    only entries into pools (shields and moves, no unshields), so it cannot show a pair either.
22. README wording: "about 2.5 minutes cold" was removed (never measured). The measured 30-day sync with all three pools
    took 2,048 s direct (`--no-tor`; 61,750 transactions fetched, 6,227 already cached). `sync` uses the first default server that answers; each server it
    tries sees your IP (README section 7, docs/PRIVACY.md), it is not "one server".
23. Limit added (README section 8, docs/PRIVACY.md): the block hash chain is checked inside one `sync` run. When a later
    run extends older data, the link between the two runs is not checked (block hashes are not stored), so a different
    server on a later run is not noticed.

## NOT VERIFIED

- `sync` over Tor on macOS and Windows, on networks that block Tor, for a 30-day range, and from servers other than
  `zec.rocks`; the cause of one hang in an early Tor sync. macOS, Windows. Firefox and Safari for the web demo.
- Release-build behaviour and speed (only the debug profile was built).
- P2SH input-address detection beyond the t3 sources seen in the sample. Whether a lightwalletd server offers all three
  pools: `sync` fails only when a range of more than 100 blocks returns no candidate at all, so a server that leaves out
  just one pool is not detected.
- A lightwalletd server that lies consistently (compact blocks carry no proof of work). Independence of the default
  servers: `na.zec.rocks` and `eu.zec.rocks` share the domain of `zec.rocks` and are probably one operator (their output
  was byte-identical once). Cold-sync time (a run with nothing cached; the measured 2,048 s had 6,227 of 67,977 transactions cached and went direct, not over Tor). The link between two sync runs (deviation 23).
- Packet-level behaviour of `sync`: a live one-day sync was sampled with `ss` (34 polls, filtered to the process id).
  Peers seen: the chosen server on 443 and the DNS resolver on UDP 53. Sampling can miss short-lived sockets. (A first
  attempt filtered by process name also matched the parallel Tor agent's `zcrowd` process: always filter by pid.)
- `strace` is not installed here, so no `connect()` list from the kernel.

## Open problems and hand-over

- `web/index.html` needs hosting (publishing is the lead's job). README and PRIVACY.md say nothing about a URL.
- Tor is merged (arti, inside the process; `--no-tor` goes direct): README section 7 and docs/PRIVACY.md T1..T7 describe
  it. The bulk 30-day sync of the Ironwood stage used `--no-tor` on purpose (the lead allowed it), so a 30-day sync over
  Tor is still not timed.
- The sample goes stale: after 2026-10-24 about 15:05 UTC `check` on it prints stale-data warnings (the README says so).
- `data/` (1.5 GB on disk, 67,977 raw transactions plus the events file, gitignored) from the 30-day sync is left in
  the working tree, and `target/debug/zcrowd` is a fresh build for the video agent.
- Verdict wording follows spec 41; the README never says "linked" or "deanonymised" about the sample: it says "exactly
  one earlier shield of about that amount in the 14 days before".
- Review hints: `crates/zcrowd-core/src/ingest.rs` (`link`, `resolve`), `source.rs` (`require_tls`, `Lwd::try_one`),
  `decode.rs` (`txid_of`, net-crossing logic), `crowd.rs` (bounds and weights).
