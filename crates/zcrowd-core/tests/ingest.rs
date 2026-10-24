use anyhow::{Result, anyhow};
use std::path::PathBuf;
use zcrowd_core::ingest::{RawCache, candidates, resolve};
use zcrowd_core::source::{BlockSource, BlockSummary, Fixtures, Recording};

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

// Spec 41 C1: the requests a sync makes depend only on the block range (and on which transactions the
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

// ---- spec 41 A9: the server is not trusted ----

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

/// A chain of synthetic compact blocks, served like a server would (every height, linked by hashes).
struct Chain(Vec<BlockSummary>);

fn chain(from: u64, n: u64) -> Chain {
    let hash = |h: u64| format!("{h:064x}");
    Chain((from..from + n).map(|h| BlockSummary { h, t: 1_700_000_000 + h, txs: vec![], hash: hash(h), prev_hash: hash(h - 1) }).collect())
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
