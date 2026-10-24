use std::path::PathBuf;
use zcrowd_core::ingest::{RawCache, candidates, resolve};
use zcrowd_core::source::{BlockSource, Fixtures, Recording};

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
