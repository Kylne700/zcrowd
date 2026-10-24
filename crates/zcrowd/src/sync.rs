use crate::Src;
use anyhow::{Result, bail};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;
use zcrowd_core::Kind;
use zcrowd_core::ingest::{RawCache, all_pools_from, candidates, resolve};
use zcrowd_core::model::{FORMAT_VERSION, Meta, read_meta, read_ndjson, save};
use zcrowd_core::source::BlockSource;

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
    // 144 extra blocks (~3 h) so the window is not a hair short of `days`.
    let start = end.saturating_sub(days * BLOCKS_PER_DAY + 144);
    let (cands, st) = candidates(s, start, end).await?;
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
    let r = resolve(s, &cands, &cache, concurrency).await?;

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
    use super::merge_skipped;
    use std::collections::BTreeMap;

    fn m(v: &[(&str, usize)]) -> BTreeMap<String, usize> {
        v.iter().map(|(k, n)| (k.to_string(), *n)).collect()
    }

    #[test]
    fn skipped_counts_add_up_and_unrecorded_stays_unrecorded() {
        let sum = merge_skipped(Some(m(&[("a", 1), ("b", 2)])), m(&[("b", 3), ("c", 4)]));
        assert_eq!(sum, Some(m(&[("a", 1), ("b", 5), ("c", 4)])));
        assert_eq!(merge_skipped(None, m(&[("a", 1)])), None);
    }
}
