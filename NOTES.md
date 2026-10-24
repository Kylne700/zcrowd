# zcrowd build notes (for the lead and the reviewers)

Specs followed: `research/39-zecathon-design.md`, then `research/41-zcrowd-privacy-spec.md` (binding, overrides 39).
Everything below was run on this machine on 2026-10-24. Lines marked "recorded" were printed during the named stage and
not re-run later; lines marked "S5 run" come from the final `bash tests/e2e.sh` (LIVE=1, PLAYWRIGHT_CORE set).

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
- S5 run (A9, A10, audit, e2e, docs; re-run after the review follow-up commit). Real output of
  `PLAYWRIGHT_CORE=<playwright-core dir> LIVE=1 bash tests/e2e.sh`, all 30 lines, nothing removed or condensed:

```
ok    cargo fmt --check
ok    cargo clippy --all-targets -D warnings
ok    cargo test: 30 tests passed, 0 failed
ok    offline sync from fixtures: 3 events, range ends at tip-10 (3522630), 3 of 3 txids recomputed, 0 mismatches
ok    check: one verdict line and one crowd line each time; 1.0 NO MATCH; 1.2497 k=157 weighted=114.5 GREEN (text and --json); 3.73 k=1 RED; --to a shield source address RED (text and --json overall verdict)
ok    report on the committed sample: 1440 unshields in the last 14 days = NO MATCH 828 (57.5%), RED 253 (17.6%), as the README says
ok    verify of a fixture transaction: MATCH (decoded from the cached raw bytes, offline)
ok    example five_lines prints: crowd 157 -> GREEN
ok    LIVE one-day sync: transaction id recomputed from the raw bytes for 173 of 173 candidates, 0 did not match (dropped); block hash chain checked over 1297 blocks
ok    build
ok    P1 sync requests depend only on the block range (test sync_requests_depend_only_on_range)
ok    P2 check, report and verify run with the network removed from outside (unshare -rn)
ok    P2 check.rs, report.rs and verify.rs import no network code (no source, tonic, tokio, std::net, Lwd, sockets)
ok    P2 the process cuts its own network: a loopback connection works before and fails after (test sandbox_blocks_connections)
ok    P3 only ingest.rs (raw cache), report.rs (report) and sync.rs (flows) write files; check.rs and verify.rs cannot
ok    P3 random address and amount typed via stdin and arguments: not in any file, no file created, address not echoed, not in --json
ok    P4 no mnemonic, seed, viewing-key or ufvk anywhere in crates/
ok    P5 336 distinct crates in cargo tree, none of reqwest, sentry, posthog, segment, openssl, native-tls, analytics, telemetry, update; Cargo.lock tracked
ok    P6 sync --server http://... is refused before any connection (run with the network removed)
ok    P6 a local TLS server with a self-signed certificate is rejected (invalid peer certificate: UnknownIssuer)
ok    P7 flows file keys, cache file names and the directory listing contain only public chain fields
ok    P8 wrong txid dropped and counted, broken prevHash and gaps are errors (tests wrong_or_broken_transactions_are_dropped_counted_and_never_cached, broken_block_chain_is_an_error, block_chain_is_checked_across_request_chunks)
ok    P9 report HTML has no src, href, url(), http(s), @import or script
ok    P10 web/index.html: CSP with connect-src, form-action, base-uri none; no external reference; no network, storage or navigation API in its script; no <form>
ok    P10 PARITY OK: 160 JS-vs-Rust comparisons (38 sample amounts, 20 rejected inputs, synthetic edge suite); identical k, weighted, verdict, suggestions
ok    P10 web/index.html equals what web/build.mjs builds from the template and the committed sample
ok    P10 headless Chromium: zero requests while typing, CSP blocks injected fetch, image, beacon, WebSocket and form (http:// and file://)
ok    P11 no home path, mail address or personal string in tracked or untracked files; 6 commits, one neutral identity; no remote; data/ and target/ ignored
AUDIT OK
E2E OK
```

  Cross-check run once: one-day sync from `zec.rocks` and from `eu.zec.rocks` (range 3527732..3529028): byte-identical
  flows files (173 events). Mutation checks of the audit itself: a `fetch(` added to the page script -> P10 fails;
  the plain-text guard disabled -> P6 fails.

## Deviations from the specs (and why)

1. `--unshield 1.23456789` prints NO MATCH plus the odd-amount note, not RED (spec 39 check; spec 41 A4 made odd amounts a
   note and A2 made k = 0 a NO MATCH).
2. `verify` is fully offline from the raw cache (spec 41 C2) instead of fetching the transaction online (spec 39).
3. Unshield `zat` is the net pool decrease (fee included). `fee_zat` is stored only when the transaction has no
   transparent inputs (otherwise it cannot be known exactly). The scorer subtracts a known fee.
4. `now` for scoring is the data's newest block time (coverage `last_t`), not the wall clock; the wall clock only drives
   the stale-data warnings. This keeps results reproducible on the committed sample.
5. Coinbase-to-shielded transactions are counted as shields and flagged `coinbase`; none occurred in the 30 days of data,
   so that path has only synthetic tests. Low confidence is used only for opposite-sign cross-pool transactions (none
   seen). Undecodable transactions are counted in the `sync` output, never silently dropped.
6. Suggestions offer only amounts that shields actually used; "split into chunks" advice was skipped (spec 41 A7 warns
   against it).
7. `--json` omits the typed amount and address. `--amount-stdin` (spec 39) is replaced by `--unshield -` / no value
   (spec 41 C4).
8. Coverage is stored in a sidecar `<data>.meta.json`; if a new sync leaves a gap to the old range, the old data is
   dropped (a gap would make NO MATCH untrue).
9. The offline fixtures are 5 block summaries (heights 3520931, 3520943, 3520949, 3522616, 3522640) and 4 raw
   transactions. The last block is padding so that tip - 10 keeps the others. The hidden `fixture` dev command that
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
    shield data only (no unshields), so it cannot show a pair either.
22. README wording: "about 2.5 minutes cold" was removed (never measured; the measured run was 140 s with 1,406 of
    6,227 transactions already cached, 4,821 fetched). `sync` uses the first default server that answers; each server it
    tries sees your IP (README section 7, docs/PRIVACY.md), it is not "one server".
23. Limit added (README section 8, docs/PRIVACY.md): the block hash chain is checked inside one `sync` run. When a later
    run extends older data, the link between the two runs is not checked (block hashes are not stored), so a different
    server on a later run is not noticed.

## NOT VERIFIED

- Tor and `torsocks` with `sync` (S7, not built here). macOS, Windows. Firefox and Safari for the web demo.
- Release-build behaviour and speed (only the debug profile was built).
- Coinbase-to-shielded and cross-pool decode paths on real transactions; P2SH input-address detection beyond the 16 t3
  sources seen in the sample.
- A lightwalletd server that lies consistently (compact blocks carry no proof of work). Independence of the default
  servers: `na.zec.rocks` and `eu.zec.rocks` share the domain of `zec.rocks` and are probably one operator (their output
  was byte-identical once). Cold-sync time (never measured). The link between two sync runs (deviation 23).
- Packet-level behaviour of `sync`: a live one-day sync was sampled with `ss` (34 polls, filtered to the process id).
  Peers seen: the chosen server on 443 and the DNS resolver on UDP 53. Sampling can miss short-lived sockets. (A first
  attempt filtered by process name also matched the parallel Tor agent's `zcrowd` process: always filter by pid.)
- `strace` is not installed here, so no `connect()` list from the kernel.

## Open problems and hand-over

- `web/index.html` needs hosting (publishing is the lead's job). README and PRIVACY.md say nothing about a URL.
- Tor merge (branch `tor` in the sibling worktree, not touched): client creation stays in `Lwd::try_one`; `require_tls`
  is one call at its top. It would refuse an `http://...onion` URL, so the Tor change may need to adapt it. README
  section 7 and docs/PRIVACY.md say "built-in Tor support is not part of this version" and "torsocks NOT VERIFIED": update
  them when Tor is merged.
- The sample goes stale: after 2026-10-24 about 15:05 UTC `check` on it prints stale-data warnings (the README says so).
- `data/` (95 MB on disk, gitignored) from the 30-day sync is left in the working tree, and `target/debug/zcrowd` is a
  fresh build for the video agent.
- Verdict wording follows spec 41; the README never says "linked" or "deanonymised" about the sample: it says "exactly
  one earlier shield of about that amount in the 14 days before".
- Review hints: `crates/zcrowd-core/src/ingest.rs` (`link`, `resolve`), `source.rs` (`require_tls`, `Lwd::try_one`),
  `decode.rs` (`txid_of`, net-crossing logic), `crowd.rs` (bounds and weights).
