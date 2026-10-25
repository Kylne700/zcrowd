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
| a server that leaves one pool out | `sync` compares the growth of each pool with the server's note commitment tree sizes at both ends of the range (any range, since the sync stage S9t); a server that gives no tree states falls back to: from NU6.3 on, more than 1000 blocks with data in some pool and none in another refuses the range; the consensus branch id of the tip must be the one expected at its height | `a_server_that_leaves_ironwood_out_of_its_blocks_is_refused_for_a_one_day_range`, `one_missing_commitment_is_enough_to_refuse_a_range`, `a_normal_one_day_range_agrees_with_the_tree_states_of_the_server`, `without_tree_states_a_pool_missing_from_a_one_day_range_is_refused`, `a_server_that_leaves_a_pool_out_is_refused_after_the_activation_of_nu6_3` (fallback), `blocks_before_the_activation_of_nu6_3_do_not_count_for_the_pool_check`, `the_branch_id_of_the_tip_must_be_the_one_expected_at_that_height` |
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
  would use; ZIP 316 says a wallet prefers the shielded receiver, and the output says so. A Unified Address in capitals
  is accepted since the S9c stage (below); mixed case is refused.
- Orchard after NU6.3 (3,428,143): no rule found in the Orchard and zcash_primitives crates or in ZIP 2005 stops value
  from entering Orchard, but the builder can no longer pay another person's Orchard address (cross-address output is
  off in the V3 Orchard pool: orchard-0.16.0 `src/lib.rs:196-198`, `src/builder.rs:1004-1026` and `1374-1379`;
  zcash_primitives-0.31.0 `src/transaction/builder.rs:407-411` switches off only the coinbase builder). ZIP 2005
  (status Proposed) says wallets SHOULD move funds to Ironwood. The sample's 0 shields into Orchard in 30 days fits
  that, but the reason is an inference; the consensus code of zebrad and zcashd was not read. `check --pool orchard`
  needs no special message and has none. SUPERSEDED in the S9d stage: ZIP 258 (Status: Final, read 2026-10-24) has the
  rule ("No new value may enter the Orchard pool": from NU6.3 activation on, for every transaction of every version, the
  encoded `valueBalanceOrchard` is nonnegative; value may still leave Orchard). So `--shield --pool orchard` is refused and
  an unshield out of Orchard gets a message when Orchard has no entry (S9d section, F5 and F6).
- The pool check compares each pool with the tree states of the server at both ends of the range (exact, any range; the
  sync stage S9t). Only a server without tree states falls back to the shortcut: a pool with no data in more than 1000
  blocks from NU6.3 on is refused; that rule does not protect a range of 1000 blocks or fewer, and Orchard activity is
  falling (the comment in `ingest.rs` says when to revisit it). Before the sync stage this check was only the shortcut
  (it fired only when more than 1000 blocks held data in some pool, so a range of about a day was not protected: the
  1-day live run had 150 Sapling, 64 Orchard and 783 Ironwood blocks, at most 997 with any).

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
30-day process (median 33 s) and about 26 s in the second. The README's earlier "a one-day sync took 20 to 40 s over Tor"
(nine runs, debug build) does not match these runs; the ranges and candidate counts of those runs are not recorded here (a
day holds about 2100 candidates now). The sync stage (branch `sync`, S9t) measured what sets the pace:

What sets the pace (sync stage, release builds, 2026-10-24, one day 3527704..3529000: 1297 blocks, 2298 candidate
transactions, 55.8 MB of raw transactions; a new Tor state directory for every run). The rate is the number of requests on
the wire divided by the time one answer takes, and that time is the circuit's: the mean answer took 0.49 to 1.36 s in the
eight runs where every request was timed, and the same range ran at 4.6 to 16.3 transactions per second on one circuit
(156 s to 578 s of sync time). Inside one run the first ~1400 candidates are small transactions (about 0.4 s each, 18 per
second, 7.5 of the 8 requests in flight all the time); the last ~900 hold about 170 transactions of 100 KB and more (median
205 KB, largest 542 KB), their latency rises to 2 s median and 5.3 s at most, and the rate falls to 8 to 10 per second. That
slowdown comes at the same candidate in every run, so on one day it is the size of the transactions, not time.
Four connections on four circuits (at most 8 requests in flight in all): a window of eight that kept its order waited for
its slowest request and held only 6.0 to 6.7 requests on the wire (17 to 25 percent of the allowed requests idle). Fetching
without the order (`resolve` in `ingest.rs`; the events are put back in candidate order) holds 7.8 to 8.0. Four circuits
are not faster than one on average: three pairs started together ran at 16.3 / 14.0, 5.9 / 10.8 and 11.3 / 9.1 transactions
per second (one circuit / four), means 11.2 and 11.3. The slowest of the three four-circuit runs was 9.1, of the
three one-circuit runs 5.9 (that one had an answer that took 29 s); three pairs are not a test. Sync time of the product builds over Tor: 578 s and 156 s
before, 330 s and 260 s with four connections and the ordered window, 247 s and 170 s with the unordered fetch. The spread
of one build is larger than the difference between builds, so no gain on a one-day range is claimed. The events of all
these Tor runs and of the direct run are identical (same SHA-256), so the Tor runs equal the `--no-tor` run line for line.
A 16-day Tor sync (37,986 candidates, 416 MB of raw transactions; build before the unordered fetch) ran in one process in
73 min 20 s, 8.6 transactions per second on average and between 7.6 and 11.5 in every window of 10 minutes; its events equal
the 30-day file above on the same heights, line for line, and a `--no-tor` sync of the same range on the transactions that
run saved wrote the same file (same SHA-256). A 30-day sync (75,606 candidates) at that rate takes about 2 h 25 min.

The same 30-day command was run once more in S9e, over Tor with a release build (`zcrowd sync --days 30`, a Tor state directory of
its own) and with the 75,606 transactions of the 30-day run above copied into its transaction directory first: 236 s (2026-10-25
00:21:11 to 00:25:07 UTC), exit code 0. The 36,289 blocks took 178 s (203.8 per second; 777 s and 477 s in the two processes above),
then the 74,655 candidates 35 s: 74,409 from the cache (each re-checked against its txid) and 246 fetched, at 16.7 per second over
the 14.7 s they took (13.1 and 11.5 per second in the two 10 s windows; the run is too short for windows of 10 minutes). It is the
time of a warm cache, not of a cold sync, and a cold 30-day sync of the final code was not repeated (NOT VERIFIED). The output of that
run is in "Merge, F8 h and data refresh stage (S9e)".

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

## Merge and portability stage (S9c)

Master is the review-fix stage's master (eaa45fd) with branch `ci` merged in (50fd22f, no textual conflict) and the items below,
one commit each. No data was refreshed: `fixtures/flows-sample.ndjson` and its meta are as before, so every sample number below
is a count, not a rewrite.

| Item | Commit | Test or proof (fails or differs without it) |
|---|---|---|
| CI keeps its `git remote remove origin` step; audit P11 accepts the project's own repository as https (with or without `.git`) or SSH | 5444085, and the P11 commit after be6102d | six accepted and nine refused spellings piped through the P11 `grep -vxE` (below); no CI run exists |
| SQLite built in: `libsqlite3-sys = { version = "0.37", features = ["bundled"] }` in `zcrowd-core`; the vcpkg steps of ci.yml and release.yml are gone | 149f772 | `cargo tree -e features -i libsqlite3-sys` shows `feature "bundled"` under `zcrowd-core`; `ldd` of the binary built by `.github/scripts/release.sh` lists glibc, libgcc_s and libm only (the debug binary built before lists `libsqlite3.so.0`); Cargo.lock gains two lines and no package |
| B1: `input_address` made up a P2SH `src` for a coinbase scriptSig | aa555f0, 8a991b4 | `a_coinbase_input_has_no_address_even_when_its_text_looks_like_a_redeem_script` (a real coinbase shield, height 3496848); it failed before with `["t3L15jegnpAVndkDzYKEvGKzAPXpaKkWHS8"]` |
| Windows default for `--tor-dir`: `%LOCALAPPDATA%\zcrowd\tor` | 0863df2 | `the_tor_dir_follows_the_system_and_is_never_a_relative_path` (with the Windows branch switched off it fails) |
| `--to` takes a Unified Address in capitals | da4df0f | `a_unified_address_in_capitals_is_accepted_but_mixed_case_is_not`; the CLI test of Unified Addresses also feeds capitals and a mixed-case string and checks that no form of the address is printed back |
| `rust-version = "1.90"` | be6102d | `cargo +1.90.0 check --workspace --all-targets --locked --offline`: exit 0, no warning |
| Web: `version: 2` in the synthetic meta of `parity.mjs`, the 6 h slack out of the page's short-window warning, `web/index.html` rebuilt | af400f9 | `node web/test/parity.mjs` failed on the merged tree; now `PARITY OK: 808` |

- **B1.** `event_from_tx` is the only place that sets `src`. A coinbase scriptSig is free text the miner wrote; its last push
  was read as a P2SH redeem script and hashed into a t3 address, so a coinbase shield got a made-up `src`. That address belongs
  to nobody (it is the hash of the text, e.g. of a pool tag): the damage was wrong data in `src` and a `--to` check that would
  have called that invented address "shielded from", not a wrong verdict for a real wallet (the message of aa555f0 says a real
  address could be flagged: too strong). The fix asks `TransparentBundle::is_coinbase()` and skips `input_address`.
  `core` test `decode_coinbase_shield_into_ironwood` also asserts an empty `src`. The new fixture `fixtures/raw/8db5109f...hex` is
  the same transaction an independent data check compared with two public explorers on 2026-10-24: kind, pool, amount,
  height, time, counts and confidence agreed; both explorers show no input address for a coinbase.
- **What changes in the committed sample** (all 67,965 lines re-decoded from their raw transactions with a throwaway example,
  deleted after): on the old decoder 0 lines differ (the control: the method reproduces the file); on the fixed decoder 1,466
  differ, all coinbase shields, `src` only, `src` becomes empty, nothing else moves. The sample has 1,564 coinbase shields:
  1,466 carried a made-up `src` (`t3L15jeg...` 1,413 times, `t3bKHRKv...` 52, `t3bvkZeE...` once), 98 had none. Non-coinbase:
  27,086 shields, 45 of them with an input read as P2SH (40,045 inputs), unchanged; none of their last pushes looks like a DER
  signature, so the other possible misfire of that rule is not visible here (still NOT VERIFIED below).
- Until the sample is refreshed, `verify` of a coinbase shield of it prints the recomputed event and
  `Error: DIFF: recomputed event differs from the cached line` (exit 1). `sync` replaces the events inside the new range and keeps
  older ones as they are, so the refresh must cover the old range or write a fresh `--out`. `FORMAT_VERSION` stays 2: files
  stay readable, one field of one kind of event was corrected. `rg -c '"coinbase":true.*"src"' fixtures/flows-sample.ndjson`
  prints 1466 now and must print nothing after the refresh (it does since S9e). `web/index.html` embeds the sample: rebuild it with `node web/build.mjs` then.
- **SQLite.** The crate that pulls it is `zcrowd-core` (`zcash_client_backend` with `tor` -> arti -> `tor-dirmgr` -> `rusqlite 0.39`
  -> `libsqlite3-sys 0.37`); features unify over the workspace. Arti's directory cache is a SQLite file (`arti-cache/dir.sqlite3`),
  so a real Tor sync is the runtime test: `LIVE=1 bash tests/audit.sh` below ran it with the new binary. A C compiler is not a
  new requirement (`ring`, `secp256k1-sys`, `zstd-sys` and `liblzma-sys` already depend on `cc`). `release.sh` no longer allows `libsqlite3.so.0`.
- **Windows tor directory.** `tor_dir_in(windows, LOCALAPPDATA, XDG_CACHE_HOME, HOME)` in `main.rs` takes the environment as
  arguments, so the test runs on every system (its paths come from `std::env::temp_dir()`, because whether a path is absolute
  depends on the system). Windows: `%LOCALAPPDATA%\zcrowd\tor` and nothing else; elsewhere `$XDG_CACHE_HOME/zcrowd/tor`, else
  `~/.cache/zcrowd/tor`; a relative value is ignored, and with nothing absolute `sync` stops: `no cache directory (... not set, or not an
  absolute path); pass --tor-dir`. A real run on Windows: NOT VERIFIED.
- **Capitals.** Bech32m (BIP 350) allows all-lowercase or all-uppercase. `destination` lowercases a string that has no lowercase
  letter, which is safe because a Base58 t1/t3 address always has its lowercase `t`; the Rust crate decodes lowercase only.
  Accepted: `t1`, `t3`, `tex1`, `u1`, `tu1`, with `tex1` and Unified Addresses in lowercase or in capitals; mixed case and capitals-only t1/t3 are refused.
- **Rust 1.90.** The highest `rust-version` any of the 624 packages declares is 1.90.0 (`typed-index-collections 3.5.0`, through
  arti); the next is 1.89. Only `check` ran with 1.90.0 (rustc 1.90.0 (1159e78c4 2025-09-14)); the tests ran with 1.97.1.
- **Why the CI step that drops `origin` stays.** With no remote P11 passes whatever a runner calls its origin, and that job needs no
  remote. P11 itself accepts the project's own repository, `Kylne700/zcrowd`, as https with or without `.git` and as SSH (scp-style
  or `ssh://`), so a reviewer's correct clone passes; a fork, another name or a URL with a token in it is refused. Checked by
  piping six accepted and nine refused spellings through the same `grep -vxE` (all six pass, all nine come out), not by a clone.

### Final run (S9c)

`PLAYWRIGHT_CORE=<playwright-core dir> bash tests/e2e.sh` on the merged tree (HEAD af400f9): exit 0, 127 s on a busy machine, no
`LIVE=1`. Of its 28 `ok` lines 25 are the same as in the final run of the review-fix stage above (compared with `diff`); the
three that differ:

```
ok    cargo test: 99 tests passed, 0 failed
ok    P10 PARITY OK: 808 JS-vs-Rust comparisons (128 sample amounts, 20 rejected inputs, synthetic edge suite); identical pool, k, weighted, verdict, pools, suggestions and ordered match txids (8 sample and 39 synthetic comparisons with a time tie)
ok    P11 no home path, mail address or personal string in tracked or untracked files; 52 commits, one neutral identity; no remote but the project's own; data/ and target/ ignored
```

It prints `SKIP  T3/T4/T6 live Tor leak check ... (NOT VERIFIED)` and `AUDIT OK (1 check(s) skipped and NOT VERIFIED ...)` because
`LIVE=1` was not set; the e2e `LIVE one-day sync` line was not run in this stage. `LIVE=1 bash tests/audit.sh` (own temporary
directory, Tor, 243 s, `AUDIT OK`, no SKIP) printed the live line:

```
ok    T3/T4/T6 lookups (any name)=0, lookups of the server name=0, connects to server IPs=0, port-53 sends=0, distinct connect destinations (Tor relays)=8; through Tor: the tip, every block and 20 transactions; 6 files, all arti directory data, guard state or locks; 0 mention the server name
```

Also run: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` (exit 0); `node web/test/parity.mjs` (PARITY OK 808);
`BROWSERS=chromium,firefox,webkit node web/test/headless.mjs`: `HEADLESS OK (chromium 154.0.8037.92, firefox 157.0, webkit 27.2; the
server saw 3 page loads and nothing else)`; the privacy scan of the repository over all branches: 0 personal-string hits, 0 home paths,
0 key-like patterns and 0 mail addresses in files, 0 personal-string hits and 0 home paths in history (the only mail addresses in
history are the neutral author address and the `noreply` trailer).

## Sandbox merge and review-fix stage (S9d)

Master is S9c's master (434e3d4) with branch `sandbox` merged (69a8f08, no conflict) and the items below, one commit each.
No data was refreshed and `source.rs` and `ingest.rs` are untouched (the `sync` branch is merged and the data refreshed after
this stage), so every sample number below is a count on the unchanged sample.

- **The sandbox holds without user namespaces (S9s).** `enter()` installs every layer the OS has and names them in one note: on
  Linux the empty network namespace as before, then always a seccomp filter (no_new_privs, all threads) that refuses `socket`
  (every family), `socketpair` and io_uring with EPERM, and every call of another ABI (i386, x32); on macOS the no-network sandbox
  profile; on Windows none, and the note says the network is NOT cut. Tests: `sandbox::tests::*` (child processes, through
  `enter()` and the filter alone), `check_report_and_verify_each_install_the_seccomp_filter_before_they_read_anything`;
  `sandbox_blocks_connections` now runs on macOS too. The two new P2 lines of the audit:
  `ok    P2 the seccomp filter refuses TCP, UDP, Unix sockets, socketpair, io_uring and other ABIs, through enter() and alone (tests sandbox::tests::*)`
  `ok    P2 check, report and verify each install the seccomp filter before they read their data (test check_report_and_verify_each_install_the_seccomp_filter_before_they_read_anything)`
  The two earlier transcripts of the audit above show the P2 lines of their day. NOT VERIFIED: the macOS run, a real Ubuntu
  24.04 (simulated with `max_user_namespaces=0`).
- **e2e reads `--json` from stdout only.** Where a layer of the sandbox is missing (a host without user namespaces) the note is
  on stderr; `tests/e2e.sh` used to parse `check --json 2>&1` and would have failed on that note. Now stderr goes to its own file,
  and the address-echo test of `--to --json` looks in both files.
- **CI.** New job `sandbox` (ubuntu-24.04 without the sysctl switch: the seccomp tests, the note on a terminal, and the namespace
  layer shown, not asserted) and the macOS step "The macOS sandbox as a user sees it" in the `test` job, both as in the S9s
  handoff with one change: `run: "cargo test --locked -p zcrowd --bin zcrowd sandbox::tests::"` is quoted, because the plain
  scalar ending in `::` is not valid YAML (PyYAML: `mapping values are not allowed here`) and GitHub would have refused the
  whole workflow. NOT VERIFIED: no CI run exists (the lead pushes to the private origin later); `actionlint` is not installed,
  the file was parsed with PyYAML only (jobs: test, sandbox, e2e, browsers, deny).

The code findings of the second independent review (G2), each with a test that failed on the code before its fix
(the page gets the same logic in `web/index.template.html`, `web/index.html` is rebuilt, and `web/test/parity.mjs` compares
it with the Rust output wherever the case is reachable):

- **F5, `--shield` weighed Orchard (becca33).** ZIP 258 (Final): from NU6.3 on no new value may enter Orchard, so Orchard had
  k=0 for every shield amount and the default `--pool any` (the worst of three pools) turned every `--shield` answer RED.
  In shield mode `any` is now the worse of Sapling and Ironwood (`score_pools`, `pools_for`): header `pool any (worst of
  sapling, ironwood)`, crowd line `... , the worst of the two`, two entries in the JSON `pools`; `--shield --pool orchard`
  is refused (`--shield cannot go into Orchard: from NU6.3 on no value may enter that pool (ZIP 258). Use --pool sapling or
  --pool ironwood, or leave --pool out`, exit 1; the page refuses it too: `A shield cannot go into Orchard: ... Choose
  Sapling, Ironwood or Not sure.`). Unshield mode is unchanged. Sample: `--shield` 0.01, 0.1 and 5 give AMBER (RED before).
  Tests: `shield_mode_weighs_only_the_pools_a_shield_can_go_into` (cli; failed before), core `shield_mode_counts_entries_into_the_chosen_pool_only_and_never_scores_orchard`,
  parity (the refusal on both sides, the two-pool `pools` of a synthetic sample).
- **F1, the line "shields after ... are not counted" was missing under `any` (081e188).** It was printed when the overall
  (worst) verdict was NO MATCH or RED; but GREEN outranks NO MATCH, so with one pool GREEN and another NO MATCH or RED the
  caveat of the pool that rests on a missing entry never showed. Now it is printed when any pool shown rests on it, except
  Orchard (later blocks cannot add to it). Test `under_any_the_missing_shields_line_follows_every_pool_shown_except_orchard`
  (failed before); page: `restsOnMissing`, compared on every parity comparison, text asserted in `headless.mjs`. S9g adds a
  third case (an address that is still open) and gives the complete rule: see N1 in the code freeze stage below.
- **F2, suggestions under `any` were scored in one pool (7193227).** `check --unshield 1.23456789` offered `1.25000000 ZEC ->
  crowd 723 (weighted 539.5, GREEN)` while checking 1.25 gave crowd 96 (weighted 70.0): the suggestion was weighed in Ironwood
  (the pool that was worst for the typed amount), the answer for 1.25 comes from Sapling (the worst pool for that amount). `suggest::popular` now takes the pools the check weighs and judges
  each candidate by their worst, as `check` does (an amount-reach pre-filter keeps `check` of the sample at 0.63 s in the debug build, measured once on a busy machine). Now
  the first suggestions are `1.24985000 ZEC -> crowd 182 (weighted 131.5, GREEN)`, `1.24980000 -> 172 (124.5)`, `1.24975000 -> 167
  (121.5)`, `1.24995000 -> 108 (78.0)`, `1.25000000 -> 96 (70.0)`. Tests: `under_any_a_suggestion_is_scored_the_way_the_check_scores_that_amount`
  (cli; every suggestion equals the check of that amount; failed before), `a_suggestion_is_judged_by_the_worst_of_the_pools_the_check_weighs`
  (suggest.rs), parity (`sample: unshield 1.23456789 {"pool":"any"}` failed before; a loop compares every suggestion with the check of its
  own amount), headless (the first suggestion of the demo amount failed before).
- **F6, an Orchard holder got NO MATCH and no hint why (0e7ae89).** Nothing can enter Orchard since NU6.3, so a note spent out
  of Orchard now was shielded before any window: when the data holds no Orchard entry (a High-confidence shield or move, any
  amount, inside the lookback) `check --unshield` with `--pool any` or `--pool orchard` adds a `note:` line after the verdict
  (and in the JSON `notes`) that says so and says how to reach further back (`sync --days N`, `--lookback-days N`); the
  report adds a sentence under its table (`N of the NO MATCH unshields left Orchard. ...`); the page has the note in its own
  words. A note, not a verdict: the verdict stays NO MATCH (the same arithmetic, honestly labelled). `shortcut:` it is not keyed
  on height, so data from before NU6.3 without an Orchard entry gets the line too. Tests: `an_empty_orchard_crowd_says_why_and_how_to_reach_further_back`
  and `report_counts_the_no_match_unshields_out_of_an_orchard_nothing_can_enter` (both failed before), core
  `has_entries_looks_at_the_window_the_kind_and_the_confidence_not_the_amount`, parity (`orchardEmpty`, compared with the CLI
  note on every comparison, plus a second synthetic file whose only Orchard entry is 20 days old: note at a lookback of 19 days,
  none at 20), headless (the note texts).
- **F7, the cli tests failed at random on a busy machine (a2cea35).** `run_env` wrote the child's stdin with `unwrap()`; a
  child that refuses its arguments exits before it reads, and the write fails with a broken pipe. `feed()` now accepts that one
  error kind and fails on any other. Test `input_for_a_child_that_already_refused_its_arguments_is_not_a_test_failure` (failed
  before with `Os { code: 32, kind: BrokenPipe ... }`); `cargo test` ran twice in the final run below.
- **F8 one-liners.** (a) The `LIVE=1` sync of `tests/e2e.sh` has its own `--tor-dir "$T/tor"` and no longer shares the default
  arti state directory (the commit message d2bd850 says "like the sandboxed runs": wrong, no other e2e run uses `--tor-dir`;
  `bash -n` only, `LIVE=1` was not run: NOT VERIFIED). (d) A damaged line in an events file was reported with serde's position
  inside the line (`at line 1 column 17`); `read_ndjson` now names the line of the file (`line 4, column 17: EOF while parsing a
  value`); test `a_damaged_event_line_is_reported_with_its_line_number_in_the_file` (failed before). The other five: (b) clap echoes
  an argv value in its errors, (c) the bad-certificate checks by hand, (e) `verify -` prints the txid it read, (f) the seccomp list has no
  ptrace/`process_vm_writev`/`pidfd_getfd`, (i) the README examples: documentation or a known residue, in the handoff
  `S9d-docs.md`, no code change. (g) `input_address` still guesses a p2sh address for a non-coinbase bare-multisig spend
  (decode.rs, S9c's file; 0 cases in the sample): not changed. **(h) NOT DONE:** no note when a Low-confidence entry falls in
  the amount window (the review says "fix if wanted"; all 140 Low events of the sample are unshields, so no entry is lost there;
  it would need a note in the text, the JSON, the report and the page, each with a parity case).
- **The P3 flake of the audit (9048c4d; the commit message labels it "F8 / G2 flake": it is item 6 of the S9d brief, not F8).** The pre-scan
  for the random typed values read the ignored `data/` directory (1.6 GB), where a random 9-digit value occurs by chance in
  about 2% of runs (`FAIL P3 the random test values were already in the tree (run again)`). The scan now asks `git grep
  --no-index --exclude-standard` (what git shows of the working tree: tracked and untracked files that are not ignored; the
  post-scan too, and the `find -newer` check still catches any file written anywhere but `target/` and `.git`). A self-check
  in the audit builds a toy directory with `.gitignore`, plants the values in an ignored file (must not be found) and in a plain
  file (must be found). Repro: a file `data/.s9d-p3-plant-<pid>` holding the typed values was found by the old expression (the
  audit would have died) and not by the new one; with `--exclude-standard` removed the self-check fails with `FAIL  P3 the
  repository scan reads ignored files ...`. What is left: a chance of about 2e-4 that a value occurs in a tracked file (still
  "run again").
- **S4-1 (re-run table "partly").** The 6 h slack of the page's short-window warning was already gone (af400f9); now the
  condition is one pure function `shortWindow(p)` and `parity.mjs` compares it with the CLI's `short_window` on every
  comparison (a synthetic sample that covers 29 days 23 h is short for a lookback of 30 days, not of 29). With the old
  slack put back parity fails: `orchard-old: short-window warning, unshield 1.0 {"pool":"any"}`.
- **S4-9 (sandbox allowed AF_UNIX): fixed by the merged seccomp filter** (every socket family, socketpair, io_uring, other ABIs). Left:
  the filter does not name `ptrace`, `process_vm_writev` or `pidfd_getfd`, and an inherited descriptor stays usable. Theoretical:
  `check`, `report` and `verify` inherit only stdio from the shell, and Yama `ptrace_scope` 1 (the Ubuntu default) limits
  tracing to descendants. Not changed, said in the docs.
- **F3, F4 (documentation only; a comment in `check.rs` cites F3).** The `check:` header no longer prints the typed amount, but
  the match lines print each match's own amount to the zatoshi and `--json` carries the matches' txids and the suggested amounts.
  An unshield's matches lie within the fee allowance above the typed amount, so the output is as sensitive as the amount: P3 of
  docs/PRIVACY.md must say so (F3; `the_amount_typed_is_never_printed_back` covers the header, the errors and the warning, not
  the match lines). The README examples still show the typed amount in the header (F4) and are regenerated from real output.
- **Docs.** README.md and docs/ belong to the next stage (S11); every change of an output line and every claim with its
  check is in `scratch/zecathon/handoff/S9d-docs.md` (the texts for F3 and F4 are in its section 3; S3-5: shield sources are
  not authenticated by the txid), as the brief says.

The sample and the number in the examples above are those of the unchanged sample; the stale coinbase `src` lines of the sample
(1,466) are still in it, `verify` on them showed DIFF until the data refresh (S9e: done, the refreshed sample has none).

### Final run (S9d)

On the tree after the last change to code, tests or the page (b602a2c; the commits after it change NOTES.md only), debug
profile, load average 1 to 3, `PLAYWRIGHT_CORE=<playwright-core dir>`, no `LIVE=1`. Every command ended with exit 0:

- `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`.
- `cargo test`, twice in a row (F7 was a race): 112 tests passed, 0 failed, both times (13 s and 9 s; 99 tests in the S9c run).
- `bash tests/e2e.sh`: `E2E OK`, 111 s, 30 `ok` lines and its own `skip  LIVE=1 not set`; inside it `AUDIT OK (1 check(s)
  skipped and NOT VERIFIED ...)`, the one skipped check being T3/T4/T6 (the live Tor leak check). The `ok` lines that differ
  from the S9c run above: `cargo test: 112 tests passed, 0 failed`, `P10 PARITY OK: 1008 JS-vs-Rust comparisons ...`, `P11 ...
  70 commits, one neutral identity ...` and the two P2 lines of the sandbox section.
- `bash tests/audit.sh` alone: `AUDIT OK`, 93 s, 22 `ok` lines, the same one SKIP.
- `node web/test/parity.mjs`, 68 s: `PARITY OK: 1008 JS-vs-Rust comparisons (128 sample amounts, 20 rejected inputs, synthetic
  edge suite); identical pool, k, weighted, verdict, pools, suggestions and ordered match txids (9 sample and 39 synthetic
  comparisons with a time tie)`.
- `BROWSERS=chromium,firefox,webkit node web/test/headless.mjs`, 15 s: `HEADLESS OK (chromium 154.0.8037.92, firefox 157.0,
  webkit 27.2; the server saw 3 page loads and nothing else)`.
- The privacy scan of the repository over all branches (counts): 0 tree files and 0 history lines with a personal-string hit,
  0 mail addresses in files, 0 home paths in files and history, 0 key-like files, 0 hits in the HEAD message; one author and
  committer (the neutral address; 82 commits over all branches); the only mail domains in history are that address's and
  the `noreply` trailer's.

Not run: `LIVE=1 bash tests/audit.sh` (T3/T4/T6) and the `LIVE=1` sync of `tests/e2e.sh`. `sync` itself is untouched by this
stage (`source.rs`, `ingest.rs` and `sync.rs` are as in S9c, and `sync` does not enter the sandbox), so the live audit of
S9c still describes it; what this stage changed on that path is the `--tor-dir` argument of the e2e line (NOT VERIFIED).

## Merge, F8 h and data refresh stage (S9e)

Master is S9d's master (13374bd) with branch `sync` (S9t) merged (a31810f) and the items below, one commit each. This stage
refreshed the committed sample, so every sample number in the stages above is a number of the old sample (blocks
3494295..3528999, 67,965 events); the table below puts the old and the new side by side.

- **Merge (a31810f, f90836d).** `git merge-tree` showed no conflict (the branch touches `source.rs`, `ingest.rs`, `sync.rs`,
  their tests, one line of `cli.rs` and one of `crates/zcrowd/Cargo.toml`), `git merge --no-ff sync` made a31810f, and S9t's
  NOTES text (what sets the Tor pace, the tree-state pool check, the new open problems) is folded in (f90836d). After the
  merge `zcrowd sync --help` still describes `--tor-dir` as Tor's state directory, which the LIVE sync of `tests/e2e.sh`
  and `tests/leakcheck/run.sh` rely on, and both workflow files parse (PyYAML `safe_load`).
- **Windows (ee98457).** `crates/zcrowd/tests/stop.rs` sends SIGINT and SIGTERM with `libc::kill`, which the libc crate does
  not have on Windows, and the CI matrix builds on `windows-latest`: the file is now `#![cfg(unix)]`. The non-Unix
  `stop_signal` in `sync.rs` (`tokio::signal::ctrl_c`; the `signal` feature is on for every target) has nothing in it that
  cannot compile there, and no test sends it a Ctrl-C. The Windows build itself is NOT VERIFIED (no Windows target and no
  mingw here): the Windows CI job is the check.
- **F8 h (578edd2).** A shield or a move into more than one pool at once is low confidence and an entry of no pool, so no
  crowd counts it; when some lie in the lookback and amount windows of a check, a NO MATCH or a small crowd can be wrong
  by that many, and the check kept quiet. `uncounted` in `crowd.rs` counts them (Shield and Move only, in any pool, with
  the same `in_windows` that `in_scope` uses, so the two cannot drift apart) and `check` ends its notes with one line:
  `note: low-confidence shields or moves of about this amount: N. A transaction that goes into more than one pool at once
  is not counted in any crowd, so a NO MATCH or a small crowd can be wrong here` (text: after `verdict:`; `--json`: the
  last of `notes`; the page prints the same sentence with a full stop). The page gets the count from a new `DATA.low`
  (`[time, zatoshi]` of each such event, no txid; `pageLow` in `web/build.mjs`). Tests: core
  `low_confidence_entries_in_both_windows_are_counted_apart_and_never_in_a_crowd`, cli
  `low_confidence_shields_in_the_window_are_told_and_never_counted` (it failed against the old `check.rs` with
  `left (None, 3) right (Some(3), 3)`), and `web/test/parity.mjs` compares the page's count with the CLI note in every one of
  its comparisons, with two synthetic low-confidence events added (a page that ignores the lookback or the mode of the
  check fails it; both were tried). All 152 low-confidence events of the new sample are unshields (the old sample's 140
  were too), so the note never shows on the sample; it is covered by those tests, not by the sample.
- **Refresh (046cc27).** One 30-day sync over Tor (`--tor-dir` of its own, the default four connections, server `zec.rocks`;
  release build of f90836d, and only `crowd.rs`, `check.rs` and tests changed after it), with the transaction cache of
  the 30-day run of the review-fix stage (75,606 files, see "Measured sync times") copied into the output directory first,
  so that only new transactions had to be fetched. It ran once, ended with exit code 0 and took **236 s** (2026-10-25
  00:21:11 to 00:25:07 UTC). Its stdout:

```
blocks=36289 txs=284182 shield_like=30830 (coinbase 1635) unshield_like=40307 move_like=3518 range=3493237..3529525 secs=178.1
pools: of the 36289 blocks from block 3428143 (NU6.3) on, 5803 hold Sapling data, 2659 Orchard, 23266 Ironwood
tree check: the server's tree states (blocks 3493236 and 3529525) grew by Sapling 16169, Orchard 11398, Ironwood 325664 note commitments, exactly what its blocks hold
events=74643 shields=30783 unshields=40350 moves=3510 fetched_txs=246 cached_txs=74409 secs=213.3
excluded: no_net_crossing=4 no_transparent_part=8 multi_pool(kept, low confidence, not counted in crowds)=152
integrity: transaction id recomputed from the raw bytes for 74655 of 74655 candidates, 0 did not match (dropped); block hash chain checked over 36289 blocks
wrote /tmp/s9e/refresh/flows.ndjson (raw transactions cached in /tmp/s9e/refresh/tx)
tip: sync on a schedule, not right before you transact. The server learns nothing about your wallet, and over Tor (the default) it does not see your IP address either; with --no-tor it does.
```

  The time is that of a warm cache: 74,409 of the 74,655 candidates were read from `tx/` (each file is re-checked against
  its txid when read) and 246 were fetched, 16.7 per second over the 14.7 s they took (13.1 and 11.5 per second in the two
  10 s windows; the run was too short for windows of 10 minutes). The block crawl took 178 s (203.8 blocks per second; the
  same 36,289 blocks took 777 s and 477 s in the two processes of the review-fix stage), the events phase 35 s, Tor's start
  and the write about 23 s. A cold 30-day sync was not repeated (the cold numbers are in "Measured sync times"). Debug
  builds and tests ran beside it under `nice -n 19`. The range is 36,289 blocks, not 34,560: `margin()` adds 5% of the days
  (1,728 blocks) to `--days 30`
  (at least 144), so the span is 31.65 days (the old sample had the 144-block margin of the first sync, 30.27 days), and
  `check --lookback-days 32` now warns (`the data covers 31.65 days`), 31 does not.
  - Checks of the new file: a throwaway release example decoded all 74,643 events again from their cached raw
    transactions with the tip's decoder: `events=74643 identical=74643 (of them low-confidence 152, coinbase 1635) diff=0
    now-skipped=0 no-raw=0` (deleted, not committed). `zcrowd verify <txid> --data <flows> --raw-dir <tx>` printed `MATCH` for a
    shielded coinbase, a low-confidence event and the newest cached transaction. `rg -c '"coinbase":true.*"src"'
    fixtures/flows-sample.ndjson` prints nothing now (it printed 1466; none of the 1,635 coinbase shields has a `src`).
  - Changed with the new sample: `fixtures/flows-sample.ndjson`, its meta, `web/index.html` (`node web/build.mjs`), the
    literals of `tests/e2e.sh` (1.0 ZEC: Ironwood 160 entries, weighted 111.0; 1.2497: Ironwood 591, weighted 438.0, Sapling
    157 / 114.5 as before; 125 ZEC: Sapling 32 / 24.0; the report rows RED 1412 8.0% and NO MATCH 3695 20.8% of 17739; the
    `five_lines` example `crowd 591 -> GREEN`), of `web/test/headless.mjs` (the same Ironwood numbers; the 1.24985 shield
    now has 87 in Sapling and 9 in Ironwood) and the cli test `a_window_shorter_than_the_lookback_is_a_short_window_without_slack`
    (it assumed the old 30.27 days; a lookback of 31 days no longer warns). `3.73` (k=1, RED) and `0.27952119` (k=3, AMBER) are
    unchanged. `parity.mjs` now finds 3 sample comparisons with a time tie (S9d: 9). The `tests/e2e.sh` message that says
    "as the README says" is true after the README is updated with the numbers below (the docs stage).
  - The stale coinbase `src` (S9c) is gone: the sample was decoded by the fixed decoder, so `verify` of a coinbase shield
    of the sample no longer prints DIFF.

| | old sample (until S9d) | new sample (046cc27) |
|---|---|---|
| blocks, span | 3494295..3528999 (34,705), 30.27 days | 3493237..3529525 (36,289), 31.65 days |
| first and newest block (UTC) | 2026-09-24 06:39 and 2026-10-24 13:05 | 2026-09-23 08:32 and 2026-10-25 00:05 |
| events: shields, unshields, moves | 67,965: 28,650, 36,492, 2,823 | 74,643: 30,783, 40,350, 3,510 |
| low confidence (all unshields) | 140 | 152 |
| shielded coinbase | 1,564 (1,466 with a made-up `src`) | 1,635 (none with a `src`) |
| Orchard events (all unshields) | 443 | 500 |
| entries (High shields and moves) | 31,473 | 34,293: Ironwood 29,800, Sapling 4,493, Orchard 0 |
| unshields of the last 14 days | 17,827 | 17,739 |
| NO MATCH, RED, AMBER, GREEN | 3,642 (20.4%), 1,405 (7.9%), 2,561 (14.4%), 10,219 (57.3%) | 3,695 (20.8%), 1,412 (8.0%), 2,541 (14.3%), 10,091 (56.9%) |
| 1.0 ZEC | Ironwood 162 | Ironwood 160 (weighted 111.0) |
| 1.2497 ZEC | Ironwood 592 (437.0), Sapling 157 | Ironwood 591 (438.0), Sapling 157 |
| 125 ZEC | Sapling 40 | Sapling 32 |
| `--shield 1.24985` | Sapling 88 (64.5), Ironwood 8 (6.0) | Sapling 87 (63.0), Ironwood 9 (6.5) |
| events file, `web/index.html` | 17,770,521 and 771,319 bytes | 19,396,366 and 835,793 bytes |

### Final run (S9e)

The code is that of 046cc27 (F8 h is 578edd2). The one commit after it, 07043d0, edits a comment in
`crates/zcrowd-core/tests/sample.rs`; after it `cargo fmt --check`, a cold `cargo clippy` and `cargo test` (twice) ran again, and
the commits after that change NOTES.md only. Debug profile, load average 1 to 2, `PLAYWRIGHT_CORE=<playwright-core dir>`, one
command after the other, 2026-10-25. Every command ended with exit 0:

- `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`. The first clippy result was cached from an earlier
  run on the same sources, so after 07043d0 clippy ran twice more: once with all sources touched, once after
  `cargo clean -p zcrowd -p zcrowd-core`, so that nothing was cached: both crates checked, no warning.
- `cargo test`, twice in a row: 139 tests passed, 0 failed, every time (15 s and 16 s; 17 s and 16 s after 07043d0; 112 tests in
  the S9d run).
- `cargo +1.90.0 check --workspace --all-targets --locked` (the `rust-version`, offline): 23 s.
- `bash tests/e2e.sh`: `E2E OK`, 123 s, 30 `ok` lines and its own `skip  LIVE=1 not set`; inside it `AUDIT OK (1 check(s)
  skipped and NOT VERIFIED ...)`, the one skipped check being T3/T4/T6 (the live Tor leak check).
- `bash tests/audit.sh` alone: `AUDIT OK`, 99 s, 22 `ok` lines, the same one SKIP.
- `LIVE=1 bash tests/audit.sh` alone: `AUDIT OK`, 250 s, 23 `ok` lines, no SKIP (161 s at d5d3939, above, when the audit had 20
  check lines; it has 22 now). Its T3/T4/T6 line: lookups of any name 0, of the server name 0, connects to server IPs 0, port-53
  sends 0, 8 distinct connect destinations (Tor relays); through Tor the tip, every block and 20 transactions; 6 files, all arti
  directory data, guard state or locks, none mentions the server name.
- `LIVE=1 bash tests/e2e.sh`: `E2E OK`, 435 s, all 34 lines below.
- `node web/test/parity.mjs`, 70 s: `PARITY OK: 1008 JS-vs-Rust comparisons (128 sample amounts, 20 rejected inputs, synthetic
  edge suite); identical pool, k, weighted, verdict, pools, suggestions and ordered match txids (3 sample and 39 synthetic
  comparisons with a time tie)`.
- `BROWSERS=chromium,firefox,webkit node web/test/headless.mjs`, 15 s: `HEADLESS OK (chromium 154.0.8037.92, firefox 157.0,
  webkit 27.2; the server saw 3 page loads and nothing else)`.
- Both workflow files (`ci.yml`, `release.yml`) parse (PyYAML `safe_load`).
- After 2026-10-25 02:05:25 UTC, when the sample is more than 2 h old and every `check` on it prints the stale-data warning
  (``warning: the newest block in this data is 2.0 h old; shields since 2026-10-25 00:05 UTC are not counted (run `zcrowd sync`)``),
  `cargo test` (139 passed), `bash tests/e2e.sh` (`E2E OK`, 123 s), `node web/test/parity.mjs` (`PARITY OK`, same line) and
  `node web/test/headless.mjs` (`HEADLESS OK`, same line) ran again on the NOTES commit before this one (26e4132) and passed;
  the e2e output differs from the earlier run in the P11 commit count only. So nothing in the tests depends on the sample being
  fresh.
- The privacy scan of the repository over all branches (counts, run on 07043d0): 0 tree files and 0 history lines with a
  personal-string hit, 0 mail addresses in files, 0 home paths in files and history, 0 key-like files, 0 hits in the HEAD
  message; one author and committer (the neutral address; 90 commits over all branches); the only mail domains in history are
  that address's and the `noreply` trailer's.

Real output of `PLAYWRIGHT_CORE=<playwright-core dir> LIVE=1 bash tests/e2e.sh` (it runs `tests/audit.sh` with the same `LIVE=1`)
on the code of 046cc27, all 34 lines, nothing removed or condensed; exit code 0, 435 s in all (network: the live one-day sync and
the Tor leak check):

```
ok    cargo fmt --check
ok    cargo clippy --all-targets -D warnings
ok    cargo test: 139 tests passed, 0 failed
ok    offline sync from fixtures: 3 events, range ends at tip-10 (3522630), 3 of 3 txids recomputed, 0 mismatches
ok    check: one verdict line and one crowd line each time, a line per pool; 1.0 GREEN (ironwood k=160); 1.2497 k=157 weighted=114.5 GREEN (text and --json); --pool; 125 ZEC Sapling k=32 vs Ironwood NO MATCH; the Ironwood-only round trip is found; 3.73 k=1 RED; --to a shield source address RED (text and --json overall verdict)
ok    report on the committed sample: 17739 unshields in the last 14 days = NO MATCH 3695 (20.8%), RED 1412 (8.0%), as the README says
ok    verify of a fixture transaction: MATCH (decoded from the cached raw bytes, offline)
ok    example five_lines prints: crowd 591 -> GREEN
ok    LIVE one-day sync: transaction id recomputed from the raw bytes for 2087 of 2087 candidates, 0 did not match (dropped); block hash chain checked over 1297 blocks
ok    build
ok    P1 sync requests depend only on the block range (test sync_requests_depend_only_on_range)
ok    P2 check, report and verify run with the network removed from outside (unshare -rn)
ok    P2 check.rs, report.rs and verify.rs import no network code (no source, tonic, tokio, std::net, Lwd, sockets)
ok    P2 the process cuts its own network: a loopback connection works before and fails after (test sandbox_blocks_connections)
ok    P2 the seccomp filter refuses TCP, UDP, Unix sockets, socketpair, io_uring and other ABIs, through enter() and alone (tests sandbox::tests::*)
ok    P2 check, report and verify each install the seccomp filter before they read their data (test check_report_and_verify_each_install_the_seccomp_filter_before_they_read_anything)
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
ok    P10 PARITY OK: 1008 JS-vs-Rust comparisons (128 sample amounts, 20 rejected inputs, synthetic edge suite); identical pool, k, weighted, verdict, pools, suggestions and ordered match txids (3 sample and 39 synthetic comparisons with a time tie)
ok    P10 web/index.html equals what web/build.mjs builds from the template and the committed sample
ok    P10 headless Chromium: zero requests while typing, CSP blocks injected fetch, image, beacon, WebSocket and form (http:// and file://)
ok    P11 no home path, mail address or personal string in tracked or untracked files; 84 commits, one neutral identity; no remote but the project's own; data/ and target/ ignored
ok    T1/T2/T5/T7 empty network namespace: the default sync fails loudly (names --no-tor), looks up no server name, resolves no name locally, writes no output; --no-tor is the control the shim sees; also flag conflicts, plain-text URL refused before Tor starts, --end-height reorg margin (crates/zcrowd/tests/tor.rs)
ok    T3/T4/T6 lookups (any name)=0, lookups of the server name=0, connects to server IPs=0, port-53 sends=0, distinct connect destinations (Tor relays)=9; through Tor: the tip, every block and 20 transactions; 6 files, all arti directory data, guard state or locks; 0 mention the server name
AUDIT OK
E2E OK
```

`diff` against the same run of the review-fix stage above shows eight lines that differ and two that are new: the test count
(139, was 96); the `check:` line (1.0 ZEC Ironwood k=160, was 162; 125 ZEC Sapling k=32, was 40); the report line (17739
unshields: NO MATCH 3695 (20.8%), RED 1412 (8.0%); was 17827, 3642 and 1405); `crowd 591` (was 592); the live one-day sync (2087
candidates in 1297 blocks; 2138 in the same 1297 blocks); the parity line (1008 comparisons, was 760, and it names the time ties);
the P11 line (84 commits, was 34); the T3/T4/T6 line (9 relays, was 8; it differs run to run); and the two P2 seccomp lines that
the sandbox stage added.

Not run: the Windows build and tests, a cold 30-day sync of the final code, and the F8 h sentence in a browser; see the S9e
bullet of the NOT VERIFIED list below.

## Code freeze stage (S9g)

Master is S9e's tip c5fb06a plus the items below, one commit each (and a comment-only commit after the first). From here on the
code is frozen: the docs stage, the demo video and the publish gate build on this tip. Against c5fb06a only
`crates/zcrowd/src/check.rs`, `crates/zcrowd/tests/cli.rs`, the new `.github/workflows/pages.yml` and this file differ. The sample,
its events and `web/index.html` are byte-identical to S9e's, so no number or example of the stages above changed; only the
final-run numbers below are new.

- **N1 of the third independent review: an address that is still open gave no "shields after ... are not counted" line (2ed0a4f;
  824e651 only rewords its comment).** `check --unshield --to ADDRESS` with an address that is clear (NO MATCH) or was only the
  destination of earlier unshields (AMBER) printed no such line on the address's account, although such an address turns RED if a
  shield from it comes after the newest block. The rule looked at the crowds only, and Orchard's exemption (nothing can enter
  Orchard since NU6.3, so later blocks cannot change its crowd) dropped it for `--pool orchard` too; but the address check looks
  at shields into any pool. `check.rs` now also prints the line when `--to` is given and its finding is clear or AMBER
  (`addr_open`), in every pool, Orchard included; a shield source is RED for good and adds nothing. The complete rule, which
  replaces those of S3-1 and F1 above: the line (after `verdict:` and `amount:`; in `--json` among the `notes`) is printed when
  (a) a pool the check shows is NO MATCH or RED, Orchard excepted (every pool under `any`, the chosen one otherwise); or (b) the
  overall verdict is NO MATCH or RED and the pool of the `crowd:` line is not Orchard (so a RED address prints it whenever that
  pool is not Orchard; `no_match_and_red_say_that_shields_after_the_last_block_are_not_counted` pins `--pool sapling`); or (c)
  `--to` is given and its finding is clear or AMBER. `--shield --to` is refused, so shield mode cannot have the case.
  Test `an_address_that_is_not_a_shield_source_says_that_shields_after_the_last_block_are_not_counted` (cli): 12 shields into
  Sapling and 12 into Ironwood, none into Orchard (GREEN, GREEN, NO MATCH), a month of data whose newest block is 10 minutes old
  (so no warning could explain the line); `--pool any`, `sapling`, `orchard`, `ironwood` x a clear and an AMBER address x text and
  `--json`: the exact `verdict:` line, the new line once and after it, no `warning:` line or note, the address never echoed; also
  a RED address under Orchard (no line), no `--to` under Sapling (no line) and the refusal of `--shield --to`. It failed on c5fb06a
  (`--pool any, address NO MATCH`) and passes now; three mutants of `check.rs` fail it (`addr.is_some()`: the RED address under
  Orchard; the address term inside Orchard's exemption: `--pool orchard, address NO MATCH`; `Clear` without AMBER: `--pool any,
  address AMBER`).
  On the committed sample 172 `check` outputs (4 pools x 5 amounts x text and `--json`, each without `--to` and with a clear, an
  AMBER and a RED address, plus `--shield` and its refusals) were captured with the old and the new binary: 44 gain exactly that
  one line (22 for a clear and 22 for an AMBER address, half of them `--json`; the other 18 + 18 had it already because of a
  pool's crowd), the other 128 are identical. The
  page has no address input (its note: "Not covered: address reuse (use the command line tool with --to)"), so it needs no
  change: `web/index.html` is untouched (835,793 bytes; `node web/build.mjs` rebuilds the same file), `parity.mjs` never passes
  `--to`, and its A10 line is as before.
- **Pages workflow (d2f6ea9).** `.github/workflows/pages.yml` publishes `web/index.html` alone to GitHub Pages, on a push to
  `master` and on "Run workflow". Job `build` copies the file into a directory it creates empty (`mkdir`, no `-p`) and uploads that
  directory with `actions/upload-pages-artifact`; job `deploy` (`environment: github-pages`) runs `actions/deploy-pages`.
  Permissions: `contents: read` at the top and in `build`; `pages: write` and `id-token: write` only in `deploy`. The three
  actions are pinned to commit SHAs with the tag in a comment, like `ci.yml` (checkout v7.0.1 is the pin `ci.yml` has;
  upload-pages-artifact v5.0.0 and deploy-pages v5.0.1 were looked up on 2026-10-25 through the GitHub API, `git/ref/tags/<tag>`,
  each tag pointing at a commit). All three workflow files parse (PyYAML `safe_load`) and a script asserted the triggers,
  permissions, job layout and pins. **It has not run:** a person must set Settings > Pages > Source to "GitHub Actions" once (a
  Free plan also needs a public repository), and the environment `github-pages` must allow `master`; until then the `deploy` job
  is expected to fail on every push, so switch Pages on before the first push of this workflow. NOT VERIFIED.
- **Left as they are: N2 and N3 of the third review (limits the docs must state, no code change).** The seccomp filter does not stop the
  same-user routes `ptrace`, `process_vm_readv`/`process_vm_writev`, `pidfd_open`/`pidfd_getfd` and `/proc/<pid>/mem` (N2), nor
  `ioctl(TIOCSTI)` (N3). The reason S4-9 above gives for calling this theoretical (Yama `ptrace_scope` 1, "the Ubuntu default") holds
  where Yama is 1 or higher; on this Debian 13 host (kernel 6.12.111) `kernel.yama.ptrace_scope` is 0 and `dev.tty.legacy_tiocsti` is 0
  (both read in this stage; no ptrace or TIOCSTI test was run). The sandbox is defence in depth against our own process doing
  network I/O, not against another process of the same user; the docs have to say on which hosts that holds.

### Final run (S9g)

The code is that of 2ed0a4f and the workflow file of d2f6ea9. The one commit after them, 824e651, rewords a comment in `check.rs`;
after it `cargo fmt --check`, a cold `cargo clippy`, `cargo test` (twice), the 1.90 check and `bash tests/e2e.sh` ran again (second
list), and the commits after that change NOTES.md only. 2026-10-25, debug profile, load average 0.5 to 4,
`PLAYWRIGHT_CORE=<playwright-core dir>`, one command after the other, `/` had 16 to 20 GB and `/tmp` 16 GB free throughout (the
stop line was 3 GB). The sample was already more than 2 h old, so every `check` on it printed the stale-data warning. Every command
ended with exit 0.

First list, on d2f6ea9 (02:33 to 02:50 UTC):

- `cargo fmt --check`: no output.
- `cargo clippy --all-targets -- -D warnings`, twice: once with all sources touched and once after `cargo clean -p zcrowd -p
  zcrowd-core` (4.4 GiB removed; both crates "Checking", no warning). 1 s each.
- `cargo test`, twice in a row: 140 tests passed, 0 failed, both times (21 s with a rebuild, 16 s; 139 in S9e). By binary: 11
  (`zcrowd` unit) + 30 (`cli`) + 2 (`stop`) + 4 (`tor`) + 33 (`zcrowd-core` unit) + 21 (`decode`) + 21 (`ingest`) + 4 (`meta`) +
  10 (`pool`) + 4 (`sample`); 0 doc tests.
- `cargo +1.90.0 check --workspace --all-targets --locked` (the `rust-version`, offline): 1 s (the dependencies were already built
  with 1.90 by earlier runs; both workspace crates were checked).
- `bash tests/e2e.sh`: `E2E OK`, 123 s, 30 `ok` lines and its own `skip  LIVE=1 not set`; inside it `AUDIT OK (1 check(s) skipped
  and NOT VERIFIED ...)`, the one skipped check being T3/T4/T6 (the live Tor leak check).
- `bash tests/audit.sh` alone: `AUDIT OK`, 98 s, 22 `ok` lines, the same one SKIP.
- `LIVE=1 bash tests/audit.sh` alone: `AUDIT OK`, 236 s, 23 `ok` lines, no SKIP. Its T3/T4/T6 line: lookups of any name 0, of the
  server name 0, connects to server IPs 0, port-53 sends 0, 5 distinct connect destinations (Tor relays); through Tor the tip,
  every block and 20 transactions; 6 files, all arti directory data, guard state or locks, none mentions the server name.
- `LIVE=1 bash tests/e2e.sh`: `E2E OK`, 470 s, all 34 lines below.
- `node web/test/parity.mjs`, 71 s: `PARITY OK: 1008 JS-vs-Rust comparisons (128 sample amounts, 20 rejected inputs, synthetic
  edge suite); identical pool, k, weighted, verdict, pools, suggestions and ordered match txids (3 sample and 39 synthetic
  comparisons with a time tie)`, the A10 line as in S9e.
- `BROWSERS=chromium,firefox,webkit node web/test/headless.mjs`, 15 s: `HEADLESS OK (chromium 154.0.8037.92, firefox 157.0,
  webkit 27.2; the server saw 3 page loads and nothing else)`.
- The three workflow files (`ci.yml`, `pages.yml`, `release.yml`) parse (PyYAML `safe_load`).
- The privacy scan of the repository over all branches (counts, run on d2f6ea9): 0 tree files and 0 history lines with a
  personal-string hit, 0 mail addresses in files, 0 home paths in files and history, 0 key-like files, 0 placeholder files, 0 hits
  in the HEAD message; one author and committer (the neutral address; 95 commits over all branches); the only mail domains in
  history are that address's and the `noreply` trailer's.

Second list, on 824e651 (02:51 to 02:54 UTC), after that comment commit: `cargo fmt --check` (no output), `cargo clippy
--all-targets -- -D warnings` (once touched, once cold after `cargo clean -p`: 4.3 GiB removed, both crates "Checking", no warning;
1 s each), `cargo test` twice (140 passed, 0 failed, both times; 21 s and 16 s), `cargo +1.90.0 check --workspace --all-targets
--locked` (1 s), the workflow files parse, and `bash tests/e2e.sh`: `E2E OK`, 126 s, the lines of the offline run above except the
P11 line (91 commits).

Real output of `PLAYWRIGHT_CORE=<playwright-core dir> LIVE=1 bash tests/e2e.sh` (it runs `tests/audit.sh` with the same `LIVE=1`)
on d2f6ea9, all 34 lines, nothing removed or condensed; exit code 0, 470 s in all (network: the live one-day sync and the Tor leak
check):

```
ok    cargo fmt --check
ok    cargo clippy --all-targets -D warnings
ok    cargo test: 140 tests passed, 0 failed
ok    offline sync from fixtures: 3 events, range ends at tip-10 (3522630), 3 of 3 txids recomputed, 0 mismatches
ok    check: one verdict line and one crowd line each time, a line per pool; 1.0 GREEN (ironwood k=160); 1.2497 k=157 weighted=114.5 GREEN (text and --json); --pool; 125 ZEC Sapling k=32 vs Ironwood NO MATCH; the Ironwood-only round trip is found; 3.73 k=1 RED; --to a shield source address RED (text and --json overall verdict)
ok    report on the committed sample: 17739 unshields in the last 14 days = NO MATCH 3695 (20.8%), RED 1412 (8.0%), as the README says
ok    verify of a fixture transaction: MATCH (decoded from the cached raw bytes, offline)
ok    example five_lines prints: crowd 591 -> GREEN
ok    LIVE one-day sync: transaction id recomputed from the raw bytes for 2101 of 2101 candidates, 0 did not match (dropped); block hash chain checked over 1297 blocks
ok    build
ok    P1 sync requests depend only on the block range (test sync_requests_depend_only_on_range)
ok    P2 check, report and verify run with the network removed from outside (unshare -rn)
ok    P2 check.rs, report.rs and verify.rs import no network code (no source, tonic, tokio, std::net, Lwd, sockets)
ok    P2 the process cuts its own network: a loopback connection works before and fails after (test sandbox_blocks_connections)
ok    P2 the seccomp filter refuses TCP, UDP, Unix sockets, socketpair, io_uring and other ABIs, through enter() and alone (tests sandbox::tests::*)
ok    P2 check, report and verify each install the seccomp filter before they read their data (test check_report_and_verify_each_install_the_seccomp_filter_before_they_read_anything)
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
ok    P10 PARITY OK: 1008 JS-vs-Rust comparisons (128 sample amounts, 20 rejected inputs, synthetic edge suite); identical pool, k, weighted, verdict, pools, suggestions and ordered match txids (3 sample and 39 synthetic comparisons with a time tie)
ok    P10 web/index.html equals what web/build.mjs builds from the template and the committed sample
ok    P10 headless Chromium: zero requests while typing, CSP blocks injected fetch, image, beacon, WebSocket and form (http:// and file://)
ok    P11 no home path, mail address or personal string in tracked or untracked files; 90 commits, one neutral identity; no remote but the project's own; data/ and target/ ignored
ok    T1/T2/T5/T7 empty network namespace: the default sync fails loudly (names --no-tor), looks up no server name, resolves no name locally, writes no output; --no-tor is the control the shim sees; also flag conflicts, plain-text URL refused before Tor starts, --end-height reorg margin (crates/zcrowd/tests/tor.rs)
ok    T3/T4/T6 lookups (any name)=0, lookups of the server name=0, connects to server IPs=0, port-53 sends=0, distinct connect destinations (Tor relays)=7; through Tor: the tip, every block and 20 transactions; 6 files, all arti directory data, guard state or locks; 0 mention the server name
AUDIT OK
E2E OK
```

`diff` against the same run of S9e above shows four lines that differ: the test count (140, was 139); the live one-day sync (2101
candidates in the same 1297 blocks; 2087 before; it differs run to run); the P11 line (90 commits, was 84); the T3/T4/T6 line (7
relays, was 9; it differs run to run).

Not run: the Windows build and tests, any run on GitHub (CI and Pages), a cold 30-day sync of the final code, the F8 h sentence in a
browser, and any ptrace or TIOCSTI test; see the S9g bullet of the NOT VERIFIED list below.

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
added the project's own private GitHub remote `origin`. The agents never push, and audit P11 now accepts that one repository
(https or SSH spelling) and no other remote.
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
24. The pool semantics of the spec (A11) were changed by the S9d stage, on the strength of ZIP 258 (Final: from NU6.3 on no new
    value may enter Orchard): in shield mode `--pool any` is the worst of Sapling and Ironwood, not of three pools (two `pool`
    lines, JSON `pools` of length 2) and `--shield --pool orchard` is refused; under `any` the "shields after ... are not counted"
    line follows every pool shown that rests on a missing entry, except Orchard; a suggestion is judged by the worst of the pools
    the check weighs; an empty Orchard crowd gets a note (text, JSON, report, page). Everything else of A11 (the events, the crowd, the
    order RED > AMBER > GREEN > NO MATCH, the A10 lines `verdict:` and `crowd:`) is as before. S9g (N1) adds a third case to that
    line, an address that is still open under `--to`: see the code freeze stage above.

## NOT VERIFIED

- `sync` over Tor on macOS and Windows, on networks that block Tor, and from servers other than `zec.rocks` (the first
  connection to another host through a freshly started Tor timed out at the exit once); the cause of one hang in an
  early Tor sync. macOS, Windows. Safari itself for the web demo (its headless test ran in Chromium 154, Firefox 157 and
  Playwright's WebKit 27.2 in the final runs of S9c and S9d).
- Release-build behaviour: the release binary was built and ran the timed syncs of the review-fix stage; the tests and
  the e2e run use the debug profile only.
- P2SH input-address detection beyond the t3 sources seen in the sample. Whether a lightwalletd server offers all three
  pools: with tree states (the live server `zec.rocks` serves them) every range is checked and a pool left out for part
  of the range is noticed. Open: a server that lies in both its blocks and its tree states; servers without tree states
  (fallback only, see the pool check above).
- The consensus side of the Orchard finding above is taken from the text of ZIP 258 (Final), not from node code: zebrad and
  zcashd were not read, and the sample holds no entry into Orchard to test the rule against (its 500 Orchard events (443 before the S9e refresh) are
  all unshields). The oldest Rust that builds the workspace
  is 1.90 (`rust-version`; `cargo +1.90.0 check --workspace --all-targets --locked` passes, S9c); the tests ran with 1.97.1 only. Whether
  the empty network namespace works on a default Ubuntu 24.04 (`kernel.apparmor_restrict_unprivileged_userns=1`): the
  seccomp layer does not need it, and the `sandbox` CI job runs without the sysctl switch to show both (no CI run has
  happened). The macOS sandbox run (compiled only).
- A lightwalletd server that lies consistently (compact blocks carry no proof of work). Independence of the default
  servers: `na.zec.rocks` and `eu.zec.rocks` share the domain of `zec.rocks` and are probably one operator (their output
  was byte-identical once). Cold-sync time of 30 days without Tor (the measured 2,048 s had 6,227 of 67,977 transactions cached; the Tor runs are in
  "Measured sync times"). The link between two sync runs (deviation 23).
- Packet-level behaviour of `sync`: a live one-day sync was sampled with `ss` (34 polls, filtered to the process id).
  Peers seen: the chosen server on 443 and the DNS resolver on UDP 53. Sampling can miss short-lived sockets. (A first
  attempt filtered by process name also matched the parallel Tor agent's `zcrowd` process: always filter by pid.)
- `strace` is not installed here, so no `connect()` list from the kernel.
- S9d: the `LIVE=1` one-day sync of `tests/e2e.sh` after the `--tor-dir` change (`bash -n` only; S9e ran it, see "Final run (S9e)"); the GitHub CI runs of the
  `sandbox` job and of the macOS step (no run exists; the YAML was parsed with PyYAML only); a bad certificate over Tor for
  `expired.badssl.com` (timed out twice in the review; the two other cases were checked by hand on 2026-10-24).
- S9e: the Windows build and test run (`tests/stop.rs` is `#![cfg(unix)]`; there is no Windows target here, the Windows CI job is
  the check); a cold 30-day sync of the final code over Tor (the refresh took 236 s with a warm cache); the F8 h line in a
  browser and on the real sample (the sample has no low-confidence shield or move; `parity.mjs` compares the page's count with the
  CLI's, no headless test reaches the sentence).
- S9g: `.github/workflows/pages.yml` has never run on GitHub (the YAML parses and its structure was asserted; whether
  `deploy-pages` accepts the artifact, whether the `github-pages` environment lets `master` through, and the page as GitHub serves it
  stay unchecked until Pages is switched on); N2 and N3 as the review states them (only the two host settings were read, no ptrace or
  TIOCSTI test); the Windows, macOS and GitHub items of S9d and S9e stand.

## Open problems and hand-over

- `web/index.html` needs hosting (publishing is the lead's job; S9g added `pages.yml`, which does it once Pages is switched on, see
  below). README and PRIVACY.md say nothing about a URL.
- Pages is not switched on (S9g): after Settings > Pages > Source = "GitHub Actions" (and a public repository on a Free plan) the
  first push to `master`, or "Run workflow", deploys `web/index.html`; the environment `github-pages` must allow `master`. `pages.yml`
  names `master` only (`ci.yml` also names `main`). Do this before the first push of `pages.yml`, or its first runs fail in the
  Actions tab.
- N2 and N3 of the third review stay as limits the docs must state (see the S9g stage): same-user ptrace and friends, and TIOCSTI, are not
  filtered, and the Yama setting that made them theoretical is Ubuntu's default (1); this Debian host has 0.
- Tor is merged (arti, inside the process; `--no-tor` goes direct): README section 7 and docs/PRIVACY.md T1..T7 describe
  it. The bulk 30-day sync of the Ironwood stage used `--no-tor` on purpose (the lead allowed it); the review-fix stage
  timed Tor syncs of one and of 30 days (see "Measured sync times").
- A Tor sync that meets a stream error in the block crawl opens the stream again after the last block received
  (`read_blocks` in `source.rs`: 5 failed attempts in a row, 20 in all, a doubling backoff from 0.5 s; a stream with no
  block for 60 s counts as failed; the hash chain is checked across the seam, tests in `tests/ingest.rs`). Blocks are
  not cached, so a sync that ends on those limits crawls again on the next run (the transaction cache is kept). Tested with
  a fake stream; a real broken connection was not seen in the sync stage (NOT VERIFIED live).
- A long Tor sync could slow down for good (30 days, above: one process fell from 11 to 1.3 transactions per second after
  35 minutes, cause not found). Now: transactions go over 4 connections, each on a circuit of its own and each replaced
  after 10 minutes (`Pool` in `source.rs`, tests in `tests/pool.rs`); the fetch no longer waits for its slowest request
  (`resolve` in `ingest.rs`; with four circuits the old ordered window of eight left 17 to 25 percent of its requests
  idle); a `sync:` line is printed whenever 40 s pass without one (checked every 10 s), so a slow crawl or fetch is silent
  for about 50 s at most (Tor start-up has its own line and a 120 s limit); Ctrl-C and SIGTERM stop a sync with the saved data untouched, and the same command goes on from `tx/`
  (`tests/stop.rs`). The 16-day run above ran at 7.6 to 11.5 transactions per second in every window of 10 minutes, with no slowdown like the
  30-day process's, so the slowdown was not reproduced and it is NOT VERIFIED that the replacement after 10 minutes
  cures it. Four circuits are not faster than one on average (three pairs, above). Not built: replacing a connection because its rate fell (a replacement by age is used instead; by
  the numbers of this stage a scheduler that prefers the faster connection would add 5 to 7 percent at most), a retry of
  a whole block range beyond the limits above, a shorter request limit (the Tor client library already sets 60 s per
  request and an HTTP/2 keep-alive).
- The sample goes stale: after 2026-10-25 about 02:05 UTC (its newest block is from 00:05 UTC) `check` on it prints stale-data
  warnings (the README says so). A new refresh is cheap while `/tmp/s9e/refresh/tx` exists (see the next point).
- `data/` (1.5 GB on disk, 67,977 raw transactions plus the events file, gitignored) from the 30-day sync is left in
  the working tree, and `target/debug/zcrowd` is a fresh build for the video agent. The S9e refresh kept its own directory
  outside the tree, `/tmp/s9e/refresh` (75,852 raw transactions, 1.76 GB on disk, the events file, the sync logs): a second
  refresh over it fetches only what is new. `/tmp` is a tmpfs here, so it is gone after a reboot.
- The README never says "linked" or "deanonymised" about the sample: it says "exactly
  one earlier shield of about that amount in the 14 days before".
- Review hints: `crates/zcrowd-core/src/ingest.rs` (`link`, `resolve`), `source.rs` (`require_tls`, `Lwd::try_one`),
  `decode.rs` (`txid_of`, net-crossing logic), `crowd.rs` (bounds and weights).
