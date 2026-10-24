use crate::Src;
use anyhow::{Result, bail};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;
use zcrowd_core::Kind;
use zcrowd_core::ingest::{RawCache, all_pools_from, candidates, resolve};
use zcrowd_core::model::{FORMAT_VERSION, Meta, read_meta, read_ndjson, save};
use zcrowd_core::source::{BlockSource, BlockSummary};

const BLOCKS_PER_DAY: u64 = 1152; // 75 s blocks
/// Stay this far behind the tip so a reorg cannot leave us with blocks that no longer exist.
const REORG_MARGIN: u64 = 10;

pub async fn run(src: Src, days: u64, out: &Path, candidates_only: bool, concurrency: usize, end_height: Option<u64>) -> Result<()> {
    match src {
        Src::Net(s) => {
            eprintln!("server: {} ({})", s.url, if s.tor { "via Tor" } else { "direct" });
            go(&s, days, out, candidates_only, concurrency, end_height).await
        }
        Src::Fix(s) => go(&s, days, out, candidates_only, concurrency, end_height).await,
    }
}

/// Extra blocks fetched beyond `days`, so that `check --lookback-days <days>` does not find the window a hair short:
/// block times vary (a 30-day span of 34560 blocks is 30 days only on average), so 5% and at least 144 blocks (~3 h).
fn margin(days: u64) -> u64 {
    (days.saturating_mul(BLOCKS_PER_DAY) / 20).max(144)
}

/// One sync per output file at a time: a second one would read the same old data, and whichever finishes last would
/// silently drop the other's blocks. The lock goes away when the process ends, even if it is killed.
fn lock(out: &Path) -> Result<std::fs::File> {
    if let Some(d) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d)?;
    }
    let p = out.with_extension("lock");
    let f = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(&p)?;
    match f.try_lock() {
        Ok(()) => Ok(f),
        Err(std::fs::TryLockError::WouldBlock) => {
            bail!("another zcrowd sync is already writing {} (lock file {}): wait for it to finish", out.display(), p.display())
        }
        Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
    }
}

/// Skipped counts of an earlier sync plus this one's. shortcut: a re-synced overlap is counted twice, so the counts only
/// grow until the data is replaced: they can over-report holes, never under-report; upgrade by storing them per block.
/// An earlier sync that recorded none stays unrecorded.
fn merge_skipped(old: Option<BTreeMap<String, usize>>, new: BTreeMap<String, usize>) -> Option<BTreeMap<String, usize>> {
    let mut m = old?;
    for (k, v) in new {
        *m.entry(k).or_default() += v;
    }
    Some(m)
}

/// Counts finished work and says when a progress line is due: at every 5 percent and at the end, so any range
/// prints about 20 lines.
struct Meter {
    total: AtomicU64,
    done: AtomicU64,
    last: AtomicU64,
}

impl Meter {
    fn new(total: u64) -> Self {
        Meter { total: AtomicU64::new(total), done: AtomicU64::new(0), last: AtomicU64::new(0) }
    }

    /// `n` more items are done. Some((done, total, percent)) when a line is due.
    fn add(&self, n: u64) -> Option<(u64, u64, u64)> {
        let done = self.done.fetch_add(n, Relaxed).saturating_add(n);
        let total = self.total.load(Relaxed);
        let pct = done.min(total).saturating_mul(100).checked_div(total)?;
        let last = self.last.load(Relaxed);
        (pct >= last + 5 || (pct == 100 && last < 100)).then(|| {
            self.last.store(pct, Relaxed);
            (done, total, pct)
        })
    }
}

/// The source of a sync, with progress on stderr: heights and counts only, nothing about the user.
struct Progress<'a, S> {
    inner: &'a S,
    blocks: Meter,
    txs: Meter,
}

impl<S: BlockSource> BlockSource for Progress<'_, S> {
    async fn tip(&self) -> Result<u64> {
        self.inner.tip().await
    }
    async fn blocks(&self, start: u64, end: u64) -> Result<Vec<BlockSummary>> {
        let r = self.inner.blocks(start, end).await?;
        if let Some((done, total, pct)) = self.blocks.add(end.saturating_sub(start).saturating_add(1)) {
            eprintln!("sync: blocks {done} of {total} ({pct}%)");
        }
        Ok(r)
    }
    async fn raw_tx(&self, txid: &str) -> Result<(Vec<u8>, u64)> {
        let r = self.inner.raw_tx(txid).await?;
        if let Some((done, total, _)) = self.txs.add(1) {
            eprintln!("sync: fetched {done} of up to {total} transactions");
        }
        Ok(r)
    }
    fn strict_chain(&self) -> bool {
        self.inner.strict_chain()
    }
}

async fn go<S: BlockSource>(
    s: &S,
    days: u64,
    out: &Path,
    candidates_only: bool,
    concurrency: usize,
    end_height: Option<u64>,
) -> Result<()> {
    let t0 = Instant::now();
    let _lock = if candidates_only { None } else { Some(lock(out)?) };
    let safe = s.tip().await?.saturating_sub(REORG_MARGIN);
    let end = match end_height {
        Some(h) if h > safe => {
            eprintln!("note: --end-height {h} is inside the reorg margin; stopping at {safe}");
            safe
        }
        Some(h) => h,
        None => safe,
    };
    let start = end.saturating_sub(days.saturating_mul(BLOCKS_PER_DAY).saturating_add(margin(days)));
    let p = Progress { inner: s, blocks: Meter::new(end.saturating_sub(start).saturating_add(1)), txs: Meter::new(0) };
    eprintln!("sync: fetching blocks {start}..{end}");
    let (cands, st) = candidates(&p, start, end).await?;
    println!(
        "blocks={} txs={} shield_like={} (coinbase {}) unshield_like={} move_like={} range={}..{} secs={:.1}",
        st.blocks,
        st.txs,
        st.shield_like,
        st.coinbase_like,
        st.unshield_like,
        st.move_like,
        start,
        end,
        t0.elapsed().as_secs_f32()
    );
    if st.any_pool_blocks > 0 {
        println!(
            "pools: of the {} blocks from block {} (NU6.3) on, {} hold Sapling data, {} Orchard, {} Ironwood",
            st.nu6_3_blocks,
            all_pools_from(),
            st.pool_blocks[0],
            st.pool_blocks[1],
            st.pool_blocks[2]
        );
    }
    if candidates_only {
        return Ok(());
    }
    let Some(new_cov) = st.coverage else { anyhow::bail!("server returned no blocks") };
    let dir = out.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let cache = RawCache(dir.join("tx"));
    p.txs.total.store(cands.len() as u64, Relaxed);
    eprintln!("sync: fetching up to {} transactions (the ones already saved are not fetched again)", cands.len());
    let r = resolve(&p, &cands, &cache, concurrency).await?;

    // Merge into any existing file: events inside the new range are replaced, older ones kept
    // when the ranges touch; a gap would make the coverage claim false, so then the old data is dropped.
    // Data without a usable meta of this version (an older zcrowd, an interrupted sync) is never merged.
    let new_skipped: BTreeMap<String, usize> = r.skipped.iter().map(|(k, v)| ((*k).to_string(), *v)).collect();
    let old = read_meta(out).ok().zip(read_ndjson(out).ok());
    let (mut events, cov, skipped) = match old {
        Some((om, old_events)) if om.cov.overlaps(&new_cov) => (
            old_events.into_iter().filter(|e| e.h < new_cov.first_h || e.h > new_cov.last_h).collect(),
            om.cov.union(new_cov),
            merge_skipped(om.skipped, new_skipped),
        ),
        Some(_) => {
            eprintln!("note: new range does not touch the existing data; replacing it");
            (vec![], new_cov, Some(new_skipped))
        }
        None => {
            if out.exists() {
                eprintln!("note: the existing data has no usable coverage file of this zcrowd version; replacing it");
            }
            (vec![], new_cov, Some(new_skipped))
        }
    };
    events.extend(r.events);
    events.sort_by(|a, b| (a.h, &a.txid).cmp(&(b.h, &b.txid)));
    events.dedup_by(|a, b| a.txid == b.txid);
    save(out, &events, &Meta { version: FORMAT_VERSION, cov, skipped })?;

    let count = |k: Kind| events.iter().filter(|e| e.kind == k).count();
    let low = events.iter().filter(|e| e.confidence == zcrowd_core::Confidence::Low).count();
    println!(
        "events={} shields={} unshields={} moves={} fetched_txs={} cached_txs={} secs={:.1}",
        events.len(),
        count(Kind::Shield),
        count(Kind::Unshield),
        count(Kind::Move),
        r.fetched,
        r.from_cache,
        t0.elapsed().as_secs_f32()
    );
    let skipped: Vec<String> = r.skipped.iter().map(|(k, v)| format!("{k}={v}")).collect();
    println!(
        "excluded: {} multi_pool(kept, low confidence, not counted in crowds)={low}",
        if skipped.is_empty() { "none".into() } else { skipped.join(" ") }
    );
    println!(
        "integrity: transaction id recomputed from the raw bytes for {} of {} candidates, {} did not match (dropped); block hash chain {}",
        r.fetched + r.from_cache,
        cands.len(),
        r.skipped.get("txid_mismatch").unwrap_or(&0),
        if s.strict_chain() {
            format!("checked over {} blocks", st.blocks)
        } else {
            "NOT checked (offline fixtures are single blocks, not a chain)".into()
        }
    );
    println!("wrote {} (raw transactions cached in {})", out.display(), cache.0.display());
    println!(
        "tip: sync on a schedule, not right before you transact. The server learns nothing about your wallet, and over Tor (the default) it does not see your IP address either; with --no-tor it does."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Meter, margin, merge_skipped};
    use std::collections::BTreeMap;

    fn m(v: &[(&str, usize)]) -> BTreeMap<String, usize> {
        v.iter().map(|(k, n)| (k.to_string(), *n)).collect()
    }

    #[test]
    fn the_margin_is_five_percent_with_a_floor_and_never_overflows() {
        assert_eq!((margin(1), margin(7), margin(30), margin(3650)), (144, 403, 1728, 210_240));
        assert_eq!(margin(u64::MAX), u64::MAX / 20);
    }

    #[test]
    fn progress_prints_about_twenty_lines_for_any_size_and_ends_at_one_hundred() {
        // blocks in chunks of 2000 (a day, a month, ten years), transactions one at a time, and tiny totals
        for (total, step) in [(1_296u64, 2000u64), (36_288, 2000), (4_200_000, 2000), (70_000, 1), (3, 1), (1, 1)] {
            let m = Meter::new(total);
            let (mut lines, mut left) = (vec![], total);
            while left > 0 {
                let n = step.min(left);
                lines.extend(m.add(n));
                left -= n;
            }
            assert!(lines.len() <= 21, "{total}: {} lines", lines.len());
            assert_eq!(lines.last(), Some(&(total, total, 100)), "{total}: the last line says 100%");
            assert!(lines.windows(2).all(|w| w[0].2 < w[1].2), "{total}: percents only go up: {lines:?}");
        }
        let m = Meter::new(10);
        assert_eq!((m.add(10), m.add(1)), (Some((10, 10, 100)), None), "100% is said once");
        assert_eq!(Meter::new(0).add(1), None, "no total, nothing to say");
    }

    #[test]
    fn skipped_counts_add_up_and_unrecorded_stays_unrecorded() {
        let sum = merge_skipped(Some(m(&[("a", 1), ("b", 2)])), m(&[("b", 3), ("c", 4)]));
        assert_eq!(sum, Some(m(&[("a", 1), ("b", 5), ("c", 4)])));
        assert_eq!(merge_skipped(None, m(&[("a", 1)])), None);
    }
}
