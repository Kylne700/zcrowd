use anyhow::{Result, anyhow};
use futures::StreamExt;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use zcrowd_core::ingest::{RawCache, all_pools_from, candidates, resolve};
use zcrowd_core::source::{BlockSource, BlockSummary, Fixtures, Recording, TxSummary, read_blocks};

fn fixtures() -> Fixtures {
    Fixtures::open(&PathBuf::from(format!("{}/../../fixtures", env!("CARGO_MANIFEST_DIR")))).unwrap()
}

fn cache(name: &str) -> RawCache {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&d);
    RawCache(d)
}

#[tokio::test]
async fn fixture_source_gives_candidates_and_events_offline() {
    let src = fixtures();
    let (c, st) = candidates(&src, 3_520_900, 3_522_700).await.unwrap();
    assert!(st.shield_like >= 1 && st.unshield_like >= 1);
    let r = resolve(&src, &c, &cache("ingest-a"), 4).await.unwrap();
    assert!(r.events.len() >= 3 && r.fetched == c.len());
    // second pass reads only the raw cache
    let cache = cache("ingest-b");
    resolve(&src, &c, &cache, 4).await.unwrap();
    let r2 = resolve(&src, &c, &cache, 4).await.unwrap();
    assert_eq!((r2.fetched, r2.from_cache), (0, c.len()));
}

// The requests a sync makes depend only on the block range (and on which transactions the
// chain shows in it), never on anything the user typed, because sync takes no user input at all.
#[tokio::test]
async fn sync_requests_depend_only_on_range() {
    let run = |name: &'static str| async move {
        let src = Recording::new(fixtures());
        let tip = src.tip().await.unwrap();
        let (c, _) = candidates(&src, tip - 2000, tip - 10).await.unwrap();
        resolve(&src, &c, &cache(name), 8).await.unwrap();
        let log = src.log.lock().unwrap().clone();
        (log, c.into_iter().map(|c| c.txid).collect::<Vec<_>>())
    };
    let (a, ca) = run("c1-a").await;
    let (b, _) = run("c1-b").await;
    assert_eq!(a, b);
    assert!(a.iter().all(|l| l == "tip" || l.starts_with("blocks ") || l.starts_with("tx ")));
    let txs: Vec<_> = a.iter().filter_map(|l| l.strip_prefix("tx ")).collect();
    assert_eq!(txs, ca.iter().map(String::as_str).collect::<Vec<_>>());
}

// ---- the server is not trusted ----

type Rewrite = Box<dyn Fn(&str, Vec<u8>) -> Vec<u8> + Sync>;

/// A server that changes the bytes of chosen transactions on their way out.
struct Tamper<S>(S, Rewrite);

impl<S: BlockSource> BlockSource for Tamper<S> {
    async fn tip(&self) -> Result<u64> {
        self.0.tip().await
    }
    async fn blocks(&self, start: u64, end: u64) -> Result<Vec<BlockSummary>> {
        self.0.blocks(start, end).await
    }
    async fn raw_tx(&self, txid: &str) -> Result<(Vec<u8>, u64)> {
        let (raw, h) = self.0.raw_tx(txid).await?;
        Ok(((self.1)(txid, raw), h))
    }
}

#[tokio::test]
async fn wrong_or_broken_transactions_are_dropped_counted_and_never_cached() {
    let clean = fixtures();
    let (c, _) = candidates(&clean, 3_520_900, 3_522_700).await.unwrap();
    assert_eq!(c.len(), 3, "the fixtures hold exactly three candidates");
    let honest = resolve(&clean, &c, &cache("a9-clean"), 4).await.unwrap();
    assert_eq!(honest.events.len(), 3);
    let (swapped, broken, fine) = (c[0].txid.clone(), c[1].txid.clone(), c[2].txid.clone());
    let other = clean.raw_tx(&fine).await.unwrap().0;
    let (sw, br) = (swapped.clone(), broken.clone());
    // `swapped` is answered with the bytes of another real transaction; `broken` with half its bytes
    let liar = Tamper(
        fixtures(),
        Box::new(move |id, raw| match id {
            x if x == sw => other.clone(),
            x if x == br => raw[..raw.len() / 2].to_vec(),
            _ => raw,
        }),
    );
    let cache = cache("a9-liar");
    let r = resolve(&liar, &c, &cache, 4).await.unwrap();
    assert_eq!(r.skipped.get("txid_mismatch"), Some(&1));
    assert_eq!(r.skipped.get("undecodable"), Some(&1));
    assert_eq!(r.events.iter().map(|e| e.txid.as_str()).collect::<Vec<_>>(), [fine.as_str()]);
    assert!(cache.get(&swapped).unwrap().is_none() && cache.get(&broken).unwrap().is_none(), "lies must not reach the cache");
    assert!(cache.get(&fine).unwrap().is_some());
    // a cache file that does not hash to its name is not believed: it is fetched again from an honest server
    cache.put(&swapped, &clean.raw_tx(&fine).await.unwrap().0).unwrap();
    let again = resolve(&clean, &c, &cache, 4).await.unwrap();
    assert_eq!((again.events.len(), again.fetched), (3, 2));
    assert_eq!(cache.get(&swapped).unwrap(), Some(clean.raw_tx(&swapped).await.unwrap().0));
}

// A cache file that is not even hex (a disk error, an editor, a half-written copy) costs one fetch, not the whole sync.
#[tokio::test]
async fn a_corrupt_cache_file_is_fetched_again_and_overwritten() {
    let src = fixtures();
    let (c, _) = candidates(&src, 3_520_900, 3_522_700).await.unwrap();
    let cache = cache("a9-corrupt");
    let first = resolve(&src, &c, &cache, 4).await.unwrap();
    assert_eq!((first.events.len(), first.fetched), (3, 3));
    let file = |i: usize| cache.0.join(format!("{}.hex", c[i].txid));
    std::fs::write(file(0), "not hex at all").unwrap();
    std::fs::write(file(1), "abc").unwrap(); // odd length
    let again = resolve(&src, &c, &cache, 4).await.unwrap();
    assert_eq!((again.events.len(), again.fetched, again.from_cache), (3, 2, 1));
    // the bad files were replaced by the real bytes
    assert_eq!(cache.get(&c[0].txid).unwrap(), Some(src.raw_tx(&c[0].txid).await.unwrap().0));
    assert_eq!(cache.get(&c[1].txid).unwrap(), Some(src.raw_tx(&c[1].txid).await.unwrap().0));
}

/// A server that is slow to answer for one chosen transaction, and counts the transactions asked for.
struct SlowOne<S> {
    inner: S,
    slow: String,
    asked: std::sync::atomic::AtomicUsize,
    asked_when_the_slow_one_answered: std::sync::atomic::AtomicUsize,
}

impl<S: BlockSource> BlockSource for SlowOne<S> {
    async fn tip(&self) -> Result<u64> {
        self.inner.tip().await
    }
    async fn blocks(&self, start: u64, end: u64) -> Result<Vec<BlockSummary>> {
        self.inner.blocks(start, end).await
    }
    async fn raw_tx(&self, txid: &str) -> Result<(Vec<u8>, u64)> {
        use std::sync::atomic::Ordering::SeqCst;
        self.asked.fetch_add(1, SeqCst);
        if txid == self.slow {
            tokio::time::sleep(Duration::from_millis(300)).await;
            self.asked_when_the_slow_one_answered.store(self.asked.load(SeqCst), SeqCst);
        }
        self.inner.raw_tx(txid).await
    }
}

// One slow answer holds no other request back. A fetch that kept its window in order stopped asking for new transactions
// while the slow one was on its way (a window of eight shrank to the slow one), and a sync over Tor spent a fifth of
// its time that way.
#[tokio::test]
async fn one_slow_answer_does_not_hold_back_the_other_requests() {
    use std::sync::atomic::Ordering::SeqCst;
    let plain = fixtures();
    let tip = plain.tip().await.unwrap();
    let (c, _) = candidates(&plain, tip - 2000, tip - 10).await.unwrap();
    assert!(c.len() > 2, "{} candidates", c.len());
    let expected = resolve(&plain, &c, &cache("slow-a"), 2).await.unwrap();

    let src = SlowOne {
        inner: fixtures(),
        slow: c[0].txid.clone(),
        asked: Default::default(),
        asked_when_the_slow_one_answered: Default::default(),
    };
    let r = resolve(&src, &c, &cache("slow-b"), 2).await.unwrap();
    // every transaction was asked for before the first answered, and the events are the same, in the same order
    assert_eq!(src.asked_when_the_slow_one_answered.load(SeqCst), c.len());
    assert_eq!(r.events, expected.events);
    assert!(r.events.len() > 1);
}

/// A chain of synthetic compact blocks, served like a server would (every height, linked by hashes).
struct Chain(Vec<BlockSummary>);

fn tx(id: u64, n_vin: usize, n_vout: usize, shielded: bool, multi_pool: bool, coinbase: bool) -> TxSummary {
    TxSummary { txid: format!("{id:064x}"), n_vin, n_vout, shielded, multi_pool, pools: [false; 3], coinbase }
}

/// Every block carries one shield-like transaction: a long range with none is refused.
fn chain(from: u64, n: u64) -> Chain {
    let hash = |h: u64| format!("{h:064x}");
    Chain(
        (from..from + n)
            .map(|h| BlockSummary {
                h,
                t: 1_700_000_000 + h,
                txs: vec![tx(h, 1, 0, true, false, false)],
                hash: hash(h),
                prev_hash: hash(h - 1),
                ..Default::default()
            })
            .collect(),
    )
}

impl BlockSource for Chain {
    async fn tip(&self) -> Result<u64> {
        Ok(self.0.last().unwrap().h)
    }
    async fn blocks(&self, start: u64, end: u64) -> Result<Vec<BlockSummary>> {
        Ok(self.0.iter().filter(|b| (start..=end).contains(&b.h)).cloned().collect())
    }
    async fn raw_tx(&self, _: &str) -> Result<(Vec<u8>, u64)> {
        Err(anyhow!("no transactions here"))
    }
    fn strict_chain(&self) -> bool {
        true
    }
}

async fn crawl_err(c: &Chain, start: u64, end: u64) -> String {
    format!("{:#}", candidates(c, start, end).await.expect_err("the crawl must fail"))
}

#[tokio::test]
async fn broken_block_chain_is_an_error() {
    let (start, end) = (1_000, 1_099);
    let (_, st) = candidates(&chain(start, 100), start, end).await.unwrap();
    assert_eq!(st.blocks, 100);
    // prev_hash that is not the previous block's hash
    let mut c = chain(start, 100);
    c.0[50].prev_hash = format!("{:064x}", 7);
    let e = crawl_err(&c, start, end).await;
    assert!(e.contains("block 1050 does not follow block 1049"), "{e}");
    // a block the server leaves out
    let mut c = chain(start, 100);
    c.0.remove(50);
    let e = crawl_err(&c, start, end).await;
    assert!(e.contains("expected block 1050 but the server sent block 1051"), "{e}");
    // a range cut short, with the first block missing, and with blocks that carry no hashes
    let mut c = chain(start, 100);
    c.0.truncate(90);
    assert!(crawl_err(&c, start, end).await.contains("stopped at block Some(1089)"));
    assert!(crawl_err(&chain(start + 1, 99), start, end).await.contains("expected block 1000"));
    let mut c = chain(start, 100);
    c.0[3].hash.clear();
    assert!(crawl_err(&c, start, end).await.contains("without a 32-byte hash"));
    assert!(crawl_err(&Chain(vec![]), start, end).await.contains("stopped at block None"));
}

#[tokio::test]
async fn block_chain_is_checked_across_request_chunks() {
    // CHUNK = 2000 blocks per request: break the link exactly where the second request starts
    let (start, n) = (10_000, 2_500);
    let end = start + n - 1;
    assert_eq!(candidates(&chain(start, n), start, end).await.unwrap().1.blocks as u64, n);
    let mut c = chain(start, n);
    c.0[2_000].prev_hash = format!("{:064x}", 9);
    assert!(crawl_err(&c, start, end).await.contains("block 12000 does not follow block 11999"));
}

/// A server whose block stream breaks. `blocks` reads through `read_blocks`; the n-th stream it opens (from 0) sends
/// `cut(n)` blocks and then fails like a lost connection (`None`: it runs to the end). `opened` lists the start heights.
struct Flaky {
    chain: Chain,
    cut: Box<dyn Fn(usize) -> Option<usize> + Sync>,
    opened: Mutex<Vec<u64>>,
}

fn flaky(chain: Chain, cut: impl Fn(usize) -> Option<usize> + Sync + 'static) -> Flaky {
    Flaky { chain, cut: Box::new(cut), opened: Mutex::new(vec![]) }
}

impl BlockSource for Flaky {
    async fn tip(&self) -> Result<u64> {
        self.chain.tip().await
    }
    async fn blocks(&self, start: u64, end: u64) -> Result<Vec<BlockSummary>> {
        read_blocks(start, end, Duration::from_secs(5), Duration::from_millis(1), |from| {
            let n = {
                let mut v = self.opened.lock().unwrap();
                v.push(from);
                v.len() - 1
            };
            let mut items: Vec<Result<BlockSummary, tonic::Status>> =
                self.chain.0.iter().filter(|b| (from..=end).contains(&b.h)).cloned().map(Ok).collect();
            if let Some(k) = (self.cut)(n) {
                items.truncate(k);
                items.push(Err(tonic::Status::unavailable("h2 protocol error: error reading a body from connection")));
            }
            async move { Ok::<_, tonic::Status>(futures::stream::iter(items)) }
        })
        .await
    }
    async fn raw_tx(&self, _: &str) -> Result<(Vec<u8>, u64)> {
        Err(anyhow!("no transactions here"))
    }
    fn strict_chain(&self) -> bool {
        true
    }
}

// A lost connection in the middle of a range costs the blocks not yet received, not the sync: the stream is opened again
// after the last block received, with no gap and no repeat, and the hash chain is still checked over the whole range.
#[tokio::test]
async fn a_block_stream_that_breaks_mid_range_is_resumed_after_the_last_block_received() {
    let (start, n) = (1_000, 300);
    let end = start + n - 1;
    // the first stream dies after 120 blocks, the second after 50, the third runs to the end
    let src = flaky(chain(start, n), |i| [Some(120), Some(50), None].get(i).copied().flatten());
    let got = src.blocks(start, end).await.unwrap();
    assert_eq!(got.iter().map(|b| b.h).collect::<Vec<_>>(), (start..=end).collect::<Vec<_>>(), "no gap and no repeat");
    assert_eq!(*src.opened.lock().unwrap(), [1_000, 1_120, 1_170], "each stream starts after the last block received");
    let src = flaky(chain(start, n), |i| [Some(120), Some(50), None].get(i).copied().flatten());
    let (_, st) = candidates(&src, start, end).await.unwrap();
    assert_eq!(st.blocks as u64, n);
    // the stream fails after the very last block: everything is there, so nothing is opened again
    let src = flaky(chain(start, n), move |_| Some(n as usize));
    assert_eq!(src.blocks(start, end).await.unwrap().len() as u64, n);
    assert_eq!(*src.opened.lock().unwrap(), [1_000]);
    // a stream that dies on the open and before any block: the first block still comes from `start`
    let src = flaky(chain(start, n), |i| (i < 2).then_some(0));
    assert_eq!(src.blocks(start, end).await.unwrap().len() as u64, n);
    assert_eq!(*src.opened.lock().unwrap(), [1_000, 1_000, 1_000]);
}

#[tokio::test]
async fn a_resumed_stream_that_does_not_follow_the_last_block_is_refused() {
    let (start, n) = (1_000, 300);
    let mut c = chain(start, n);
    c.0[120].prev_hash = format!("{:064x}", 7); // the block the second stream starts with
    let src = flaky(c, |i| (i == 0).then_some(120));
    let e = format!("{:#}", candidates(&src, start, start + n - 1).await.unwrap_err());
    assert!(e.contains("block 1120 does not follow block 1119"), "{e}");
}

#[tokio::test]
async fn resuming_is_bounded_in_attempts_in_a_row_and_in_all() {
    let (start, n) = (1_000, 300);
    let end = start + n - 1;
    // fails before sending anything: 5 attempts, then the error says why
    let src = flaky(chain(start, n), |_| Some(0));
    let e = format!("{:#}", src.blocks(start, end).await.unwrap_err());
    assert!(e.contains("h2 protocol error") && e.contains("gave up at block 1000") && e.contains("after 5 failed attempts"), "{e}");
    assert_eq!(src.opened.lock().unwrap().len(), 5);
    // one block per stream, for ever: every attempt makes progress, so only the total stops it
    let src = flaky(chain(start, n), |_| Some(1));
    let e = format!("{:#}", src.blocks(start, end).await.unwrap_err());
    assert!(e.contains("gave up at block 1019") && e.contains("after 20 failed attempts"), "{e}");
    assert_eq!(src.opened.lock().unwrap().len(), 20);
    // a block received starts the count in a row again: four empty failures, one block, three more, then it runs
    let src = flaky(chain(start, n), |i| match i {
        0..=3 | 5..=7 => Some(0),
        4 => Some(1),
        _ => None,
    });
    assert_eq!(src.blocks(start, end).await.unwrap().len() as u64, n);
}

// A stream that goes quiet counts as failed, and is resumed like one that errors.
#[tokio::test]
async fn a_block_stream_that_goes_quiet_is_resumed() {
    let (start, n) = (1_000, 50);
    let c = chain(start, n);
    let opens = Mutex::new(vec![]);
    let got = read_blocks(start, start + n - 1, Duration::from_millis(30), Duration::from_millis(1), |from| {
        opens.lock().unwrap().push(from);
        let first = opens.lock().unwrap().len() == 1;
        let items: Vec<Result<BlockSummary, tonic::Status>> =
            c.0.iter().filter(|b| b.h >= from).take(if first { 10 } else { usize::MAX }).cloned().map(Ok).collect();
        let s = futures::stream::iter(items);
        let s: futures::stream::BoxStream<'static, Result<BlockSummary, tonic::Status>> =
            if first { s.chain(futures::stream::pending()).boxed() } else { s.boxed() };
        async move { Ok::<_, tonic::Status>(s) }
    })
    .await
    .unwrap();
    assert_eq!(got.iter().map(|b| b.h).collect::<Vec<_>>(), (start..start + n).collect::<Vec<_>>());
    assert_eq!(*opens.lock().unwrap(), [1_000, 1_010]);
}

// What the compact block says decides which transactions are fetched in full.
#[tokio::test]
async fn candidate_kinds_cover_coinbase_and_moves() {
    let txs = vec![
        tx(1, 0, 1, true, false, true),   // shielded coinbase: the compact vin leaves its input out, so n_vin is 0
        tx(2, 2, 0, true, false, false),  // transparent inputs
        tx(3, 0, 1, true, false, false),  // transparent output only
        tx(4, 0, 0, true, true, false),   // no transparent part, two pools: a possible move
        tx(5, 0, 0, true, false, false),  // no transparent part, one pool: nothing to see
        tx(6, 1, 1, false, false, false), // no shielded part
        tx(7, 3, 2, true, true, false),   // everything: it is shield-like, decode sorts it out
    ];
    let src =
        Chain(vec![BlockSummary { h: 5, t: 9, txs, hash: format!("{:064x}", 5), prev_hash: format!("{:064x}", 4), ..Default::default() }]);
    let (c, st) = candidates(&src, 5, 5).await.unwrap();
    let got: Vec<_> = c.iter().map(|c| (c.txid.trim_start_matches('0').to_string(), c.kind)).collect();
    use zcrowd_core::Kind::{Move, Shield, Unshield};
    assert_eq!(got, [("1".into(), Shield), ("2".into(), Shield), ("3".into(), Unshield), ("4".into(), Move), ("7".into(), Shield)]);
    assert_eq!((st.shield_like, st.coinbase_like, st.unshield_like, st.move_like, st.txs), (3, 1, 1, 1, 7));
}

// A server that leaves pool data out must not look like a quiet month.
#[tokio::test]
async fn a_long_range_without_any_candidate_is_an_error() {
    let quiet = |n: u64| {
        let mut c = chain(1_000, n);
        c.0.iter_mut().for_each(|b| b.txs = vec![tx(b.h, 0, 0, true, false, false)]);
        c
    };
    // 100 blocks may be quiet; 101 may not
    assert_eq!(candidates(&quiet(100), 1_000, 1_099).await.unwrap().1.blocks, 100);
    let e = crawl_err(&quiet(101), 1_000, 1_100).await;
    assert!(e.contains("101 blocks came back without one transaction") && e.contains("try another --server"), "{e}");
    // one shield-like transaction anywhere in the range is enough
    let mut c = quiet(500);
    c.0[499].txs.push(tx(1, 1, 0, true, false, false));
    assert_eq!(candidates(&c, 1_000, 1_499).await.unwrap().1.shield_like, 1);
    // moves alone do not count: they have no transparent part, which is what a server without it would show
    let mut c = quiet(500);
    c.0[3].txs.push(tx(2, 0, 0, true, true, false));
    assert!(crawl_err(&c, 1_000, 1_499).await.contains("500 blocks"));
}

// Block times are not monotonic: the median-time-past rule lets a block be stamped earlier than the one before it.
#[tokio::test]
async fn the_windows_times_are_the_extremes_of_the_block_times_not_those_of_its_edge_blocks() {
    let mut c = chain(1_000, 100);
    c.0[40].t = 1_800_000_000; // stamped later than the last block
    c.0[60].t = 1_600_000_000; // stamped earlier than the first
    let cov = candidates(&c, 1_000, 1_099).await.unwrap().1.coverage.unwrap();
    assert_eq!((cov.first_h, cov.last_h), (1_000, 1_099));
    assert_eq!((cov.first_t, cov.last_t), (1_600_000_000, 1_800_000_000));
    // in order, as before: the first and the last block
    let cov = candidates(&chain(1_000, 100), 1_000, 1_099).await.unwrap().1.coverage.unwrap();
    assert_eq!((cov.first_t, cov.last_t), (1_700_001_000, 1_700_001_099));
}

/// `n` blocks from the activation of NU6.3 on, each with data in the given pools (Sapling, Orchard, Ironwood).
fn pooled(n: u64, pools: [bool; 3]) -> Chain {
    let mut c = chain(all_pools_from(), n);
    c.0.iter_mut().for_each(|b| b.txs[0].pools = pools);
    c
}

async fn crawl(c: &Chain) -> anyhow::Result<zcrowd_core::ingest::Stats> {
    candidates(c, c.0[0].h, c.0.last().unwrap().h).await.map(|r| r.1)
}

// A server that leaves one pool out must not look like a quiet pool: that would be a false NO MATCH.
#[tokio::test]
async fn a_server_that_leaves_a_pool_out_is_refused_after_the_activation_of_nu6_3() {
    assert_eq!(all_pools_from(), 3_428_143);
    let st = crawl(&pooled(1_001, [true; 3])).await.unwrap();
    assert_eq!((st.nu6_3_blocks, st.any_pool_blocks, st.pool_blocks), (1_001, 1_001, [1_001; 3]));
    for (pools, missing) in [
        ([true, true, false], "Ironwood"), // a lightwalletd from before NU6.3
        ([true, false, true], "Orchard"),
        ([false, true, true], "Sapling"),
        ([true, false, false], "Orchard"), // the first missing pool is named
    ] {
        let e = format!("{:#}", crawl(&pooled(1_001, pools)).await.unwrap_err());
        assert!(e.contains(&format!("server returns no {missing} data (outdated lightwalletd?)")), "{pools:?}: {e}");
    }
    // 1000 blocks may lack a pool, 1001 may not
    assert!(crawl(&pooled(1_000, [true, true, false])).await.is_ok());
    // one block with the pool is enough, wherever it sits
    let mut c = pooled(1_500, [true, true, false]);
    c.0[1_400].txs.push(TxSummary { pools: [false, false, true], ..tx(9, 0, 0, true, false, false) });
    assert_eq!(crawl(&c).await.unwrap().pool_blocks, [1_500, 1_500, 1]);
}

#[tokio::test]
async fn blocks_before_the_activation_of_nu6_3_do_not_count_for_the_pool_check() {
    let from = all_pools_from();
    let mut c = chain(from - 1_500, 2_000); // 1500 blocks before the activation, 500 from it on
    c.0.iter_mut().for_each(|b| b.txs[0].pools = [true, true, false]);
    let st = crawl(&c).await.unwrap();
    assert_eq!((st.nu6_3_blocks, st.any_pool_blocks, st.pool_blocks), (500, 500, [500, 500, 0]));
    // before the activation there is no Ironwood to miss, however long the range
    let mut c = chain(from - 3_000, 2_500);
    c.0.iter_mut().for_each(|b| b.txs[0].pools = [true, true, false]);
    assert_eq!(crawl(&c).await.unwrap().nu6_3_blocks, 0);
}

/// A server that serves tree states: `sizes(h)` is what it says its trees (Sapling, Orchard, Ironwood) hold after block `h`.
/// `asked` counts the block requests it got.
struct Trees {
    chain: Chain,
    sizes: Box<dyn Fn(u64) -> Option<[u64; 3]> + Sync>,
    asked: std::sync::atomic::AtomicUsize,
}

impl BlockSource for Trees {
    async fn tip(&self) -> Result<u64> {
        self.chain.tip().await
    }
    async fn blocks(&self, start: u64, end: u64) -> Result<Vec<BlockSummary>> {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.chain.blocks(start, end).await
    }
    async fn raw_tx(&self, txid: &str) -> Result<(Vec<u8>, u64)> {
        self.chain.raw_tx(txid).await
    }
    async fn tree_sizes(&self, h: u64) -> Result<Option<[u64; 3]>> {
        Ok((self.sizes)(h))
    }
    fn strict_chain(&self) -> bool {
        true
    }
}

fn with_trees(chain: Chain, sizes: impl Fn(u64) -> Option<[u64; 3]> + Sync + 'static) -> Trees {
    Trees { chain, sizes: Box::new(sizes), asked: Default::default() }
}

async fn crawl_trees(t: &Trees) -> anyhow::Result<zcrowd_core::ingest::Stats> {
    candidates(t, t.chain.0[0].h, t.chain.0.last().unwrap().h).await.map(|r| r.1)
}

/// A one-day range like the live one of Oct 24 2026 (1297 blocks from 3527704): Sapling data in every 9th block, Orchard in
/// every 21st, Ironwood in four of five, each with the commitments they add.
fn day() -> Chain {
    let mut c = chain(3_527_704, 1_297);
    for b in &mut c.0 {
        let has = [b.h % 9 == 0, b.h % 21 == 0, b.h % 5 != 0];
        let adds = [3, 2, 60];
        b.txs[0].pools = has;
        b.commitments = std::array::from_fn(|i| if has[i] { adds[i] } else { 0 });
    }
    c
}

/// What an honest server says its trees hold after block `h`: a starting size plus the commitments of the blocks up to `h`.
fn honest(c: &Chain) -> impl Fn(u64) -> Option<[u64; 3]> + Sync + 'static {
    let adds: Vec<(u64, [u32; 3])> = c.0.iter().map(|b| (b.h, b.commitments)).collect();
    move |h| {
        let mut t = [1_000_000u64, 500_000, 2_000_000];
        adds.iter().filter(|a| a.0 <= h).for_each(|a| t.iter_mut().zip(a.1).for_each(|(t, n)| *t += u64::from(n)));
        Some(t)
    }
}

/// The same blocks with the Ironwood data taken out, as an outdated lightwalletd serves them.
fn without_ironwood(mut c: Chain) -> Chain {
    c.0.iter_mut().for_each(|b| {
        b.txs[0].pools[2] = false;
        b.commitments[2] = 0;
    });
    c
}

async fn trees_err(t: &Trees) -> String {
    format!("{:#}", crawl_trees(t).await.expect_err("the crawl must fail"))
}

// The tree states settle what the blocks alone cannot, at any length of range and for a pool as quiet as you like.
#[tokio::test]
async fn a_normal_one_day_range_agrees_with_the_tree_states_of_the_server() {
    let st = crawl_trees(&with_trees(day(), honest(&day()))).await.unwrap();
    assert!(st.commitments.iter().all(|&n| n > 0), "{:?}", st.commitments);
    assert_eq!(st.tree_growth, Some(st.commitments));
    assert_eq!((st.nu6_3_blocks, st.blocks), (1_297, 1_297));
}

#[tokio::test]
async fn a_server_that_leaves_ironwood_out_of_its_blocks_is_refused_for_a_one_day_range() {
    let honest_trees = honest(&day()); // its node's trees still grow with the Ironwood transactions
    let e = trees_err(&with_trees(without_ironwood(day()), honest_trees)).await;
    assert!(e.contains("blocks 3527704..3529000 hold 0 Ironwood commitments but its tree state went from 2000000 to"), "{e}");
    assert!(e.contains("try another --server"), "{e}");
}

#[tokio::test]
async fn one_missing_commitment_is_enough_to_refuse_a_range() {
    let mut c = day();
    let honest_trees = honest(&c);
    let i = c.0.iter().position(|b| b.commitments[1] == 2).unwrap();
    c.0[i].commitments[1] = 1;
    let e = trees_err(&with_trees(c, honest_trees)).await;
    assert!(e.contains("hold 123 Orchard commitments but its tree state went from 500000 to 500124"), "{e}");
    // and trees that shrink are as wrong as trees that grow too little
    let honest_trees = honest(&day());
    let shrinking = move |h: u64| honest_trees(h).map(|[s, o, i]| [s, o, if h < 3_528_000 { i } else { 1_999_999 }]);
    let e = trees_err(&with_trees(day(), shrinking)).await;
    assert!(e.contains("Ironwood commitments but its tree state went from 2000000 to 1999999"), "{e}");
}

// An outdated lightwalletd answers an empty Ironwood tree at both ends: 0 = 0 would pass a comparison, and the crawl of
// days of blocks would be wasted before the refusal.
#[tokio::test]
async fn a_server_whose_ironwood_tree_is_empty_is_refused_before_any_block_is_asked_for() {
    let t = with_trees(without_ironwood(day()), |_| Some([5_000_000, 600_000, 0]));
    let e = trees_err(&t).await;
    assert!(e.contains("server returns no Ironwood data (outdated lightwalletd?)"), "{e}");
    assert_eq!(t.asked.load(std::sync::atomic::Ordering::SeqCst), 0);
    // a range from before the pool has had time to fill is not judged by its empty tree
    let early = chain(all_pools_from() + 100, 300);
    assert!(crawl_trees(&with_trees(early, |_| Some([5_000_000, 600_000, 0]))).await.is_ok());
}

// A server without tree states (no GetTreeState) has only its blocks to be judged by.
#[tokio::test]
async fn without_tree_states_a_pool_missing_from_a_one_day_range_is_refused() {
    let e = crawl_err(&without_ironwood(day()), 3_527_704, 3_529_000).await;
    assert!(e.contains("server returns no Ironwood data (outdated lightwalletd?)"), "{e}");
    let st = crawl(&day()).await.unwrap();
    assert_eq!((st.tree_growth, st.nu6_3_blocks), (None, 1_297));
    // a range of 1000 blocks or fewer is too short to tell a missing pool from a quiet one
    let short = Chain(without_ironwood(day()).0[..1_000].to_vec());
    assert!(crawl(&short).await.is_ok());
}
