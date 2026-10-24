# zcrowd build notes (for the lead and the reviewers)

The design and privacy specs these notes followed are not in this repository; what they require is in `docs/PRIVACY.md` and checked by `tests/audit.sh`.
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
- S3 (scoring). Recorded: `check --unshield 1.0` NO MATCH; `1.23456789` NO MATCH plus the odd-amount note;
  `1.2497` crowd 157, weighted 114.5, GREEN; `1.2497 --to <an address that shielded 2026-10-19>` RED.
- S4 (HTML report). Recorded: `unshare -rn target/debug/zcrowd check|report` exit 0; report 3.4 KB without external
  references.
- S4b (web demo). Recorded: `PARITY OK: 160 JS-vs-Rust comparisons (38 sample amounts, 20 rejected inputs, synthetic edge
  suite); identical k, weighted, verdict, suggestions` (7 of 8 deliberate JS mutations caught; the survivor is
  behaviour-equivalent); `HEADLESS OK` (http and file://: 1 request at load, 0 while typing, 5 CSP violations recorded
  for the injected attempts, the static server log shows only `GET /index.html`, storage empty).
- S5 (integrity checks, stable output lines, audit, e2e, docs). Recorded: cross-check run once: one-day sync from `zec.rocks` and from `eu.zec.rocks` (range 3527732..3529028): byte-identical
  flows files (173 events). Mutation checks of the audit itself: a `fetch(` added to the page script -> P10 fails;
  the plain-text guard disabled -> P6 fails.

## Ironwood stage (findings S1-1, S1-2, S1-3, S2-4, S3-2, S3-4 of the independent review)

Cause: the first `sync` did not ask for the third shielded pool, Ironwood (transaction v6), and the decoder did not read
its value balance. Transactions through Ironwood were missing or showed another amount: an explorer cross-check found 4 of 20
sampled events wrong (`91c68e50...`, `c292feb1...`, `48ca0a26...`, `ce7e196a...`), the review's finding S1-1 a false NO MATCH.

Commits (all by the neutral identity, Sonnet 5.5 trailer; subjects shortened):

```
acfb52f Decode and fetch all three shielded pools (Ironwood), step 1
6acb46f Score crowds per shielded pool, add check --pool, step 3
27bb88f Apply rustfmt and fix one clippy lint in the pool-aware scoring code
79be5c6 report: score each unshield against entries of about its amount only
b3124d7 Re-sync the sample with all three shielded pools and test it against the explorer cross-check
2995ba7 Web demo: pick the pool, one line per pool, the worst pool decides
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
| the 20 explorer-checked events decoded as the explorers show | 16 of 20 | 20 of 20 (test `sample.rs`) |
| JS-vs-Rust comparisons | 160 | 760 |

Decisions:

- Event #7 of the explorer cross-check (`48ca0a26...`): the explorer shows the net crossing 12,272,904, the data holds the Ironwood
  shield 12,319,850. the decoder counts what arrives in the pool, and Orchard pays out 46,946 in the same transaction. This is
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

The reviewer's exact 40 (finding S1-1: their saved probe output for blocks 3528000..3528048, read only) were
replayed against the committed sample: all 40 are High Ironwood shields with the probe's amount, and `check --unshield
<shield - 15000>` (default pool) gives AMBER 19, RED 15, GREEN 6, NO MATCH 0 (their run on the Ironwood-blind sample:
NO MATCH 36, AMBER 3, RED 1). The probe itself was not run again; a test covers all 126 Ironwood shields of blocks
3528000..3528199. Of the 20 explorer-checked events, 19 equal the explorer in kind, pool and amount; #7 has the explorer's
direction and pool and the decoder's amount (see the first decision).

Evidence by finding (`cargo test --workspace` runs every test named here; the e2e steps and the parity run are noted):

| finding | where |
|---|---|
| S1-1 Ironwood data is fetched, no false NO MATCH | `decode_shield_into_ironwood_only`, `e11_ironwood_only_shield_is_counted`, `a_send_max_round_trip_through_an_ironwood_shield_is_never_a_false_no_match`, e2e round trip of `43bdcfaf...` |
| S1-2 decode reads all three balances, books must close | `decode_two_pool_unshield_e14` (a57f5cd4... = 134,225,000), `decode_three_pool_unshield_1`, `decode_three_pool_unshield_6`, `decode_two_pool_unshield_9`, `numbers_that_do_not_close_are_refused_not_clamped`, `the_twenty_explorer_checked_events_decode_as_the_explorers_show` |
| S1-3 one crowd per pool | `e12_other_pools_are_not_counted_and_any_is_the_worst_pool`, `worst_pool_order_and_ties`, `cli_check_pool_option_scores_one_pool_and_any_shows_the_worst`, e2e 125 ZEC (Sapling k=40, Ironwood NO MATCH), parity (every pool setting) |
| S2-4 `--shield X` is what arrives in the pool | `decode_shield_into_ironwood_while_orchard_pays_in_7`, `shield_mode_counts_entries_into_the_chosen_pool_only`; help text, prompt, README and web label say "arrives in the shielded pool" |
| S3-2 empty range is an error | `a_long_range_without_any_candidate_is_an_error` |
| S3-4 coinbase is `CompactTx.index == 0` | `candidate_kinds_cover_coinbase_and_moves`, `decode_coinbase_shield_into_ironwood` |
| E11, E12, E13, E14, E15 | `e11_ironwood_only_shield_is_counted`, `e12_other_pools_are_not_counted_and_any_is_the_worst_pool`, `e13_a_move_into_ironwood_is_a_candidate_for_an_ironwood_unshield` (and `decode_move_into_ironwood`), `decode_two_pool_unshield_e14`, `e15_a_shielded_coinbase_is_an_entry` (and `shielded_coinbase_entries_in_the_sample_are_counted`) |

Left for the next stage: S1-4, S2-1, S2-2, S2-3, S3-1, S3-3, S3-6 and the code items of S4 were done in the
review-fix stage below; S3-5, S3-7 and the other S4 items are text or web changes (written down for the docs and web
stages).

NOT VERIFIED in this stage: the five new fixtures were not compared with an explorer (the 20 explorer-checked events were);
the server's version or pool support is not read, so a server that leaves out only one pool was not detected (the
review-fix stage added a check for that); Firefox and Safari; a 30-day sync over Tor (timed in the review-fix stage).

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

## Review-fix stage (the remaining code findings of the review and the plan's must-have items)

The independent review of the build at 8914192 also had findings that the Ironwood stage did not take ("Left for the
next stage" above), and the plan added must-have items for the CLI (M2, M3, M5, M6, Q9) and two checks on what a
lightwalletd server sends. This stage fixed all of them that live in Rust code, tests and the shell tests; each change was
broken on purpose once and the test named here had to fail (one exception, noted below). What needs words in README.md
and docs/PRIVACY.md was written down for the docs stage, not edited here.

| finding | what changed | tests |
|---|---|---|
| S1-4 the id given to `verify` sits in argv | `verify -` reads it privately; an id in the arguments prints a note; the advice after MATCH says to spot-check random events and to open the explorer over Tor | `verify_reads_the_id_from_stdin_notes_one_in_the_arguments_and_advises_spot_checks` |
| S2-1 the sandbox proof could not fail | a Linux test: `check`, `report` and `verify` must each be in their own network namespace before they read their data file | `check_report_and_verify_each_leave_the_network_before_they_read_anything` |
| S2-2 the amount was echoed back | the header, amount parse errors and address errors never repeat what was typed | `the_amount_typed_is_never_printed_back`, `parse_errors_never_repeat_the_text`, `cli_check_shield_mode_and_address_reuse` |
| S2-3 `--json` lacked what the text has | data window, its age, short-window flag, the not-covered footer, per-pool lines and the notes | `json_carries_the_data_window_its_age_the_footer_and_the_notes` |
| S3-1 a NO MATCH or RED ignores new shields | both always say `shields after <time> UTC (last synced block) are not counted` | `no_match_and_red_say_that_shields_after_the_last_block_are_not_counted` |
| S3-3 skipped transactions were invisible | the meta keeps a count per reason; `check` prints one line, and a warning when a reason could hide a shield or unshield | `check_says_what_the_sync_skipped_and_whether_it_matters`, `skipped_counts_add_up_and_unrecorded_stays_unrecorded` |
| S3-6 no bounds on numbers | `--lookback-days` 1..=3650, `--days` 1..=3650, `--tolerance-zat` and `--fee-allowance-zat` at most 1 ZEC; `u64::MAX` neither panics nor wraps | `numeric_options_have_limits_and_huge_values_are_refused_not_crashed`, `huge_parameters_neither_panic_nor_overflow`, `the_margin_is_five_percent_with_a_floor_and_never_overflows` |
| S4-1 6 h of silent slack in the short-window warning | gone; `sync` fetches 5% more blocks than asked (at least 144) so the same lookback is not found a hair short | `a_window_shorter_than_the_lookback_is_a_short_window_without_slack`, `a_span_an_hour_short_of_30_days_does_not_read_as_30_days` |
| S4-2 / M3 a crash between the two files | temp files, the events first and the meta last, the old meta removed before; `flows.lock` stops two syncs on one output | `a_save_that_stops_between_the_two_renames_leaves_no_window_to_trust`, `a_save_that_fails_before_the_events_are_replaced_keeps_the_old_pair`, `a_second_sync_on_the_same_output_is_refused_while_the_first_holds_the_lock` |
| S4-3 a corrupt cached transaction stopped the sync | it is fetched again and overwritten | `a_corrupt_cache_file_is_fetched_again_and_overwritten` |
| S4-4 `TEX1...` in capitals | accepted (ZIP 320 allows it); base58 and mixed case stay refused | `decode_tex_in_capitals_is_accepted_but_base58_and_mixed_case_are_not` |
| S4-8 a vacuous assert | the bad address is typed first and must not come back in the error | `cli_check_shield_mode_and_address_reuse` |
| S4-12 `now` was the last block's time | the window's times are the earliest and latest block time seen | `the_windows_times_are_the_extremes_of_the_block_times_not_those_of_its_edge_blocks` |
| M2 data format version | the meta has `version: 2`; `check`, `report` and `verify` refuse any other (or a meta that does not parse) | `a_meta_without_this_version_is_refused`, `check_and_report_refuse_data_without_this_format_version`, `sync_replaces_data_of_another_version_and_stores_the_format_version_and_skipped_counts` |
| M5 `--to` with a Unified Address | its transparent receiver is checked; a note says a wallet normally pays the shielded one; a UA without a transparent receiver is refused with a reason | `a_unified_address_is_checked_through_its_transparent_receiver` (core and CLI, the CLI one also proves it is never printed back), `a_unified_address_without_a_transparent_receiver_says_it_does_not_leave_the_shielded_pools` |
| M6 clock sanity | `check` warns when the clock is more than 3 hours behind or a year ahead of the newest block | `the_clock_is_wrong_beyond_three_hours_behind_or_a_year_ahead_of_the_newest_block`, `a_clock_far_off_the_chain_is_reported` |
| a server that leaves one pool out | from NU6.3 on, more than 1000 blocks with data in some pool and none in another refuses the range; the consensus branch id of the tip must be the one expected at its height | `a_server_that_leaves_a_pool_out_is_refused_after_the_activation_of_nu6_3`, `blocks_before_the_activation_of_nu6_3_do_not_count_for_the_pool_check`, `the_branch_id_of_the_tip_must_be_the_one_expected_at_that_height` |
| Q9 a first Tor start and a long sync looked like a hang | stderr says `starting Tor; the first run can take a minute`, then the block range, blocks done with a percent and the transactions fetched (heights and counts only) | `progress_prints_about_twenty_lines_for_any_size_and_ends_at_one_hundred`, `sync_reports_its_progress_on_stderr_with_heights_and_counts_only`, `tor_failure_is_loud_and_never_goes_direct` |

The one change a test does not pin: the `TEX1` prefix guard in `decode::destination` (a base58 string lowercased fails its
checksum anyway, so the guard is belt and braces). `tests/audit.sh` P3 now lists `model.rs` among the files that write
(called by `sync` only) and P11 accepts the project's own GitHub remote and no other.

The live leak check (`tests/leakcheck/run.sh`, audit T3/T4/T6) synced `--days 0`, which the new limit refuses. It now
syncs one day, the shortest range. To keep the Tor leg to a few minutes it starts from the direct leg's transaction
cache minus 20 files, asserts that it fetched at least one transaction, and still asks for the tip, every block and 20
transactions through Tor. The byte-identity of the two outputs therefore proves the blocks and only those 20
transactions, not all of them.

Decisions worth a reviewer's eye:
- M3 is read as: after a crash `check` sees either the old matching pair of files or an explicit refusal ("no coverage
  file ... run `zcrowd sync` again"), never a window that claims more blocks than the events hold. It is not read as
  "the interrupted sync can be resumed".
- A Unified Address given to `--to` is judged by its transparent receiver only, because that is what an unshield to it
  would use; ZIP 316 says a wallet prefers the shielded receiver, and the output says so. Uppercase `U1...` is still refused.
- Orchard after NU6.3 (3,428,143): no rule found in the Orchard and zcash_primitives crates or in ZIP 2005 stops value
  from entering Orchard, but the builder can no longer pay another person's Orchard address (cross-address output is
  off in the V3 Orchard pool: orchard-0.16.0 `src/lib.rs:196-198`, `src/builder.rs:1004-1026` and `1374-1379`;
  zcash_primitives-0.31.0 `src/transaction/builder.rs:407-411` switches off only the coinbase builder). ZIP 2005
  (status Proposed) says wallets SHOULD move funds to Ironwood. The sample's 0 shields into Orchard in 30 days fits
  that, but the reason is an inference; the consensus code of zebrad and zcashd was not read. `check --pool orchard`
  needs no special message and has none.
- The pool check is a shortcut with two limits. It fires only when more than 1000 blocks hold data in some pool, so a
  range of about a day or less is not protected (the 1-day live run had 150 Sapling, 64 Orchard and 783 Ironwood
  blocks: at most 997 with any, which is below it). And Orchard activity is falling, so in the future even a long range
  might hold no Orchard block and be refused; the comment in `ingest.rs` says when to revisit it.

### Measured sync times (release build, 2026-10-24, server `zec.rocks`)

The release binary (`cargo build --release -p zcrowd`, 1 min 13 s) synced about one day (1297 blocks, 2134 to 2188
candidate transactions) four times, each into an empty output directory. The machine was shared with other work (load
average 2 to 12 on 20 cores), so these show what a run looks like, not a clean benchmark. "Ready" is the time from
starting the program to the connection being up (for Tor: the Tor start, with its state directory new or kept).

| run | ready | 1297 blocks | transactions | total |
|---|---|---|---|---|
| Tor, new state directory | 22 s | 101 s | 2188 in 743 s (2.9 per second) | 14 min 26 s |
| Tor, state directory kept from the run above | 5 s | 132 s | 2159 in 434 s (5.0 per second) | 9 min 31 s |
| Tor, new state directory (run later, beside the 30-day run) | 34 s | 18 s | 2134 in 204 s (10.4 per second) | 4 min 17 s |
| `--no-tor` | 1 s | 2.6 s | 2134 in 60 s (36 per second) | 63 s |

A 30-day sync over Tor (`--days 30`, an empty output directory, the Tor state directory of the runs above) took
2 h 25 min (18:34 to 20:59 UTC) in two processes of the same command. The first read the 36,289 blocks in 777 s, fetched
transactions at 10 to 13 per second for about 20 minutes, then slowed down (4.7, 2.5, then 0.9 to 1.6 per second) and was
stopped by me with `kill` after 70 min 30 s with 19,159 of 75,522 transactions cached. A fresh process that synced one day
beside it ran at 10.4 per second, so the server and Tor as a whole were not slow: the slowdown stayed with that process.
The second process (the same command 5 s later) read the blocks again (blocks are not cached) in 477 s, took 18,922
transactions from the cache and fetched the other 56,447 in 66 min (14.2 per second, 13.7 to 16.6 in every 10-minute
window), and ended with exit code 0 after 74 min; its integrity line: 75,369 of 75,369 candidates recomputed, 0 did not
match. On the 34,705 blocks that it and the direct 30-day sync of the Ironwood stage both cover, the 67,965 events are
identical line for line. One process without the slow phase would have taken about 1.6 to 2.1 hours (a projection from
the two rates, NOT VERIFIED as one run). The numbers are from one occasion.

Over Tor the speed varies a lot, between runs and inside one run: one-day runs went at 2.9, 5.0 and 10.4 transactions
per second, the 30-day processes at 10 to 13, then 1.3, then 14 to 17; 2000 blocks took 16 to 85 s per request in the first
30-day process (median 33 s) and about 26 s in the second. What sets the pace was not tested. Facts in the code that fit a
cause that stays with one process: all requests of a run share one HTTP/2 connection (one Tor stream, one circuit), a
request may wait 60 s per attempt (5 attempts), and `buffered(8)` in `resolve()` returns results in order, so one slow
request holds back the seven behind it. Stopping the slow process and running the same command again recovered the
pace (the second process above). The README's earlier "a one-day sync took 20 to 40 s over Tor" (nine runs, debug
build) does not match these runs; the ranges and candidate counts of those runs are not recorded here (a day holds
about 2100 candidates now).

### Final run (review-fix stage, last code commit a29b5d3)

Real output of `PLAYWRIGHT_CORE=<playwright-core dir> LIVE=1 bash tests/e2e.sh` (it runs `tests/audit.sh` with the same
`LIVE=1`) on 2026-10-24, all 32 lines, nothing removed or condensed; exit code 0, 629 s in all (network: the live
one-day sync and the Tor leak check). It ran in a fresh clone of that commit with one clone-only commit on top: the web
fix that belongs to the web stage (`version: 2` in the synthetic meta of `web/test/parity.mjs`, and `web/index.html`
rebuilt). Without it two checks fail on this branch: the parity test (the Rust side now refuses a meta without a
version) and P10 (the page embeds the sample's meta, which gained `version` and `skipped`, so it must be rebuilt).

```
ok    cargo fmt --check
ok    cargo clippy --all-targets -D warnings
ok    cargo test: 96 tests passed, 0 failed
ok    offline sync from fixtures: 3 events, range ends at tip-10 (3522630), 3 of 3 txids recomputed, 0 mismatches
ok    check: one verdict line and one crowd line each time, a line per pool; 1.0 GREEN (ironwood k=162); 1.2497 k=157 weighted=114.5 GREEN (text and --json); --pool; 125 ZEC Sapling k=40 vs Ironwood NO MATCH; the Ironwood-only round trip is found; 3.73 k=1 RED; --to a shield source address RED (text and --json overall verdict)
ok    report on the committed sample: 17827 unshields in the last 14 days = NO MATCH 3642 (20.4%), RED 1405 (7.9%), as the README says
ok    verify of a fixture transaction: MATCH (decoded from the cached raw bytes, offline)
ok    example five_lines prints: crowd 592 -> GREEN
ok    LIVE one-day sync: transaction id recomputed from the raw bytes for 2138 of 2138 candidates, 0 did not match (dropped); block hash chain checked over 1297 blocks
ok    build
ok    P1 sync requests depend only on the block range (test sync_requests_depend_only_on_range)
ok    P2 check, report and verify run with the network removed from outside (unshare -rn)
ok    P2 check.rs, report.rs and verify.rs import no network code (no source, tonic, tokio, std::net, Lwd, sockets)
ok    P2 the process cuts its own network: a loopback connection works before and fails after (test sandbox_blocks_connections)
ok    P3 only ingest.rs (raw cache), model.rs (the flows files, called by sync only), report.rs (report), sync.rs (lock file) and source.rs (Tor state directory, sync only) write files; check.rs and verify.rs cannot
ok    P3 random address and amount typed via stdin and arguments: not in any file, no file created, neither echoed in the output (text, --json, --shield, arguments)
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
ok    P11 no home path, mail address or personal string in tracked or untracked files; 34 commits, one neutral identity; no remote but the project's own; data/ and target/ ignored
ok    T1/T2/T5/T7 empty network namespace: the default sync fails loudly (names --no-tor), looks up no server name, resolves no name locally, writes no output; --no-tor is the control the shim sees; also flag conflicts, plain-text URL refused before Tor starts, --end-height reorg margin (crates/zcrowd/tests/tor.rs)
ok    T3/T4/T6 lookups (any name)=0, lookups of the server name=0, connects to server IPs=0, port-53 sends=0, distinct connect destinations (Tor relays)=8; through Tor: the tip, every block and 20 transactions; 6 files, all arti directory data, guard state or locks; 0 mention the server name
AUDIT OK
E2E OK
```

## Deviations from the first design (and why)

1. `--unshield 1.23456789` prints NO MATCH plus the odd-amount note, not RED (the first design said RED; the privacy spec made odd amounts a
   note and k = 0 a NO MATCH).
2. `verify` is fully offline from the raw cache instead of fetching the transaction online (the first design).
3. Unshield `zat` is the net pool decrease (fee included). `fee_zat` is stored only when the transaction has no
   transparent inputs (otherwise it cannot be known exactly). The scorer subtracts a known fee.
4. `now` for scoring is the data's newest block time (coverage `last_t`), not the wall clock; the wall clock only drives
   the stale-data warnings. This keeps results reproducible on the committed sample.
5. Coinbase-to-shielded transactions are counted as shields and flagged `coinbase` (detected as the first transaction of
   a block, `CompactTx.index == 0`). Low confidence ("multi-pool", pool Mixed) is used for transactions that cross or move
   value in more than one shielded pool without being a plain shield, unshield or move. Transactions that
   cannot be decoded or whose books do not close are counted by reason in the `sync` output, never silently dropped
   (before the Ironwood stage the first sentence said no coinbase occurred and Low was only for opposite-sign
   transactions: that was the Ironwood-blind sample).
6. Suggestions offer only amounts that shields actually used; "split into chunks" advice was skipped (chunks sent at the same time or to one
   address would link to each other).
7. `--json` omits the typed amount and address. `--amount-stdin` of the first design is replaced by `--unshield -` / no value
   (so the amount stays out of shell history).
8. Coverage is stored in a sidecar `<data>.meta.json`; if a new sync leaves a gap to the old range, the old data is
   dropped (a gap would make NO MATCH untrue).
9. The offline fixtures are 5 block summaries (heights 3520931, 3520943, 3520949, 3522616, 3522640) and 14 raw
   transactions in `fixtures/raw/` (4 sit behind those blocks; the other 10 are decode tests, among them 5 added in the
   Ironwood stage). The last block is padding so that tip - 10 keeps the others. The hidden `fixture` dev command that
   fetched them was used once and deleted. The sample (`flows-sample.ndjson`) contains public transparent addresses.
10. Web demo is a JS port, not WASM, so `'wasm-unsafe-eval'` is dropped from the CSP. The data is inlined (the privacy spec
    preferred it to a "fetch a data file", which needs a network permission the page must not have). It has no `--to` support.
11. The audit's secret-word grep (`mnemonic|seed|ufvk|viewing|address` in `crates/`) is narrowed to
    `mnemonic|seed|ufvk|viewing`, because zcrowd stores and accepts addresses on purpose.
12. The report gained a table of how the unshields of the last 14 days score (the number in the README).
13. E10 hand check: the three fixture transactions were compared with the Blockchair API on 2026-10-24 (in the test
    comments).
14. `docs/demo-script.md` was not written: the video belongs to S6.
15. New, beyond the spec (found while testing): tonic silently sends plain-text HTTP/2 for an `http://` URL even with a
    TLS config. `require_tls` (in `crates/zcrowd-core/src/source.rs`) refuses `http://` unless the host is `localhost`
    or a loopback IP. Test `plain_text_only_to_this_machine`; the audit runs the refusal with the network removed.
16. The server is not trusted: transaction ids are recomputed (ZIP-244 / legacy) and mismatches dropped and counted; the block hash
    chain is checked only on the network path (`BlockSource::strict_chain`; the offline fixtures are single blocks).
    `BlockSummary` gained optional `hash` / `prev_hash` fields (old fixtures still load). `sync` prints an `integrity:`
    line.
17. Stable output: the `crowd:` line is `crowd: k=<n> weighted=<w>` followed by an explanation in parentheses. The web
    parity test uses `check --json` as its stable format; the text contract is asserted in `crates/zcrowd/tests/cli.rs`
    and `tests/e2e.sh`.
18. The audit numbers claims P1..P11 (docs/PRIVACY.md), adds "no file is created by
    `check`" (stamp-file test), a leak-detector canary, and runs the plain-text refusal under `unshare -rn`.
19. Commit trailers: S1..S3 carry the Opus 5.5 co-author trailer, S4 onward the Sonnet 5.5 one (instruction changed
    between stages). Author and committer are the neutral identity in every commit; no remote existed until the lead
added the project's own private GitHub remote `origin`. The agents never push, and audit P11 now accepts that one URL and
no other remote.
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

- `sync` over Tor on macOS and Windows, on networks that block Tor, and from servers other than `zec.rocks` (the first
  connection to another host through a freshly started Tor timed out at the exit once); the cause of one hang in an
  early Tor sync. macOS, Windows. Firefox and Safari for the web demo.
- Release-build behaviour: the release binary was built and ran the timed syncs of the review-fix stage; the tests and
  the e2e run use the debug profile only.
- P2SH input-address detection beyond the t3 sources seen in the sample. Whether a lightwalletd server offers all three
  pools: since the review-fix stage `sync` refuses a range in which, from NU6.3 on, more than 1000 blocks hold data in some
  pool and none holds data in another. A range of about a day or less is below that and not protected, and a server
  that leaves a pool out for only part of the range is not noticed.
- The consensus side of the Orchard finding above (zebrad and zcashd were not read). The oldest Rust that builds it:
  the dependencies declare 1.90 (`typed-index-collections`, through arti); it was built here with 1.97.1 only. Whether
  the sandbox tests and audit P2 run on a CI runner: Ubuntu 24.04 can set `kernel.apparmor_restrict_unprivileged_userns=1`,
  which breaks `unshare` of a user namespace (no CI run has happened).
- A lightwalletd server that lies consistently (compact blocks carry no proof of work). Independence of the default
  servers: `na.zec.rocks` and `eu.zec.rocks` share the domain of `zec.rocks` and are probably one operator (their output
  was byte-identical once). Cold-sync time of 30 days without Tor (the measured 2,048 s had 6,227 of 67,977 transactions cached; the Tor runs are in
  "Measured sync times"). The link between two sync runs (deviation 23).
- Packet-level behaviour of `sync`: a live one-day sync was sampled with `ss` (34 polls, filtered to the process id).
  Peers seen: the chosen server on 443 and the DNS resolver on UDP 53. Sampling can miss short-lived sockets. (A first
  attempt filtered by process name also matched the parallel Tor agent's `zcrowd` process: always filter by pid.)
- `strace` is not installed here, so no `connect()` list from the kernel.

## Open problems and hand-over

- `web/index.html` needs hosting (publishing is the lead's job). README and PRIVACY.md say nothing about a URL.
- Tor is merged (arti, inside the process; `--no-tor` goes direct): README section 7 and docs/PRIVACY.md T1..T7 describe
  it. The bulk 30-day sync of the Ironwood stage used `--no-tor` on purpose (the lead allowed it); the review-fix stage
  timed Tor syncs of one and of 30 days (see "Measured sync times").
- A Tor sync can end on a stream error in the middle of the block crawl (seen once in several dozen requests: `Error:
  block stream` / `Stream not connected`). `blocks()` in `source.rs` retries only the call that opens the stream and blocks
  are not cached, so a re-run crawls them again (the transaction cache is kept). Not changed.
- A long Tor sync can slow down for good (30 days: 10 to 13 per second for 20 minutes, then 1.3 per second for 35
  minutes, while a fresh process beside it ran at 10.4). The percent progress lines went quiet for those 35 minutes.
  Stopping it and running the same command again fixed it once. Not changed: several connections, unordered buffering
  in `resolve()` (`ingest.rs:221`, `MAX_CONCURRENCY` is 8), a time-based progress line and a retry of a whole block range
  are open design choices, each of which needs a test against the server's load limits.
- The sample goes stale: after 2026-10-24 about 15:05 UTC `check` on it prints stale-data warnings (the README says so).
- `data/` (1.5 GB on disk, 67,977 raw transactions plus the events file, gitignored) from the 30-day sync is left in
  the working tree, and `target/debug/zcrowd` is a fresh build for the video agent.
- The README never says "linked" or "deanonymised" about the sample: it says "exactly
  one earlier shield of about that amount in the 14 days before".
- Review hints: `crates/zcrowd-core/src/ingest.rs` (`link`, `resolve`), `source.rs` (`require_tls`, `Lwd::try_one`),
  `decode.rs` (`txid_of`, net-crossing logic), `crowd.rs` (bounds and weights).
