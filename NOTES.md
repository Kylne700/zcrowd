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
- S5 run (A9, A10, audit, e2e, docs):

```
ok    cargo fmt --check
ok    cargo clippy --all-targets -D warnings
ok    cargo test: 30 tests passed, 0 failed
ok    offline sync from fixtures: 3 events, range ends at tip-10 (3522630), 3 of 3 txids recomputed, 0 mismatches
ok    report on the committed sample: 1440 unshields in the last 14 days = NO MATCH 828 (57.5%), RED 253 (17.6%)
ok    LIVE one-day sync: transaction id recomputed from the raw bytes for 173 of 173 candidates, 0 did not match (dropped); block hash chain checked over 1297 blocks
ok    P1 ... P11 (see docs/PRIVACY.md)
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

## NOT VERIFIED

- Tor and `torsocks` with `sync` (S7, not built here). macOS, Windows. Firefox and Safari for the web demo.
- Release-build behaviour and speed (only the debug profile was built).
- Coinbase-to-shielded and cross-pool decode paths on real transactions; P2SH input-address detection beyond the 16 t3
  sources seen in the sample.
- A lightwalletd server that lies consistently (compact blocks carry no proof of work). Independence of `zec.rocks` and
  `eu.zec.rocks` (their output was byte-identical once).
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
