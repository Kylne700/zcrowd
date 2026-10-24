use crate::Src;
use anyhow::Result;
use std::io::Write;
use std::path::Path;
use std::time::Instant;
use zcrowd_core::Kind;
use zcrowd_core::ingest::{RawCache, candidates, resolve};
use zcrowd_core::model::{Coverage, Event, read_coverage, read_ndjson};
use zcrowd_core::source::BlockSource;

const BLOCKS_PER_DAY: u64 = 1152; // 75 s blocks
/// Stay this far behind the tip so a reorg cannot leave us with blocks that no longer exist.
const REORG_MARGIN: u64 = 10;

pub async fn run(src: Src, days: u64, out: &Path, candidates_only: bool, concurrency: usize) -> Result<()> {
    match src {
        Src::Net(s) => {
            eprintln!("server: {}", s.url);
            go(&s, days, out, candidates_only, concurrency).await
        }
        Src::Fix(s) => go(&s, days, out, candidates_only, concurrency).await,
    }
}

async fn go<S: BlockSource>(s: &S, days: u64, out: &Path, candidates_only: bool, concurrency: usize) -> Result<()> {
    let t0 = Instant::now();
    let end = s.tip().await?.saturating_sub(REORG_MARGIN);
    // 144 extra blocks (~3 h) so the window is not a hair short of `days`.
    let start = end.saturating_sub(days * BLOCKS_PER_DAY + 144);
    let (cands, st) = candidates(s, start, end).await?;
    println!(
        "blocks={} txs={} shield_like={} (coinbase {}) unshield_like={} range={}..{} secs={:.1}",
        st.blocks,
        st.txs,
        st.shield_like,
        st.coinbase_like,
        st.unshield_like,
        start,
        end,
        t0.elapsed().as_secs_f32()
    );
    if candidates_only {
        return Ok(());
    }
    let Some(new_cov) = st.coverage else { anyhow::bail!("server returned no blocks") };
    let dir = out.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let cache = RawCache(dir.join("tx"));
    let r = resolve(s, &cands, &cache, concurrency).await?;

    // Merge into any existing file: events inside the new range are replaced, older ones kept
    // when the ranges touch; a gap would make the coverage claim false, so then the old data is dropped.
    let old: Vec<Event> = if out.exists() { read_ndjson(out)? } else { vec![] };
    let old_cov = read_coverage(out);
    let (mut events, cov) = match old_cov {
        Some(oc) if oc.overlaps(&new_cov) => {
            (old.into_iter().filter(|e| e.h < new_cov.first_h || e.h > new_cov.last_h).collect(), oc.union(new_cov))
        }
        Some(_) => {
            eprintln!("note: new range does not touch the existing data; replacing it");
            (vec![], new_cov)
        }
        None => (vec![], new_cov),
    };
    events.extend(r.events);
    events.sort_by(|a, b| (a.h, &a.txid).cmp(&(b.h, &b.txid)));
    events.dedup_by(|a, b| a.txid == b.txid);

    let tmp = out.with_extension("tmp");
    let mut w = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
    for e in &events {
        writeln!(w, "{}", serde_json::to_string(e)?)?;
    }
    w.flush()?;
    std::fs::rename(&tmp, out)?;
    std::fs::write(Coverage::path_for(out), serde_json::to_vec(&cov)?)?;

    let sh = events.iter().filter(|e| e.kind == Kind::Shield).count();
    let low = events.iter().filter(|e| e.confidence == zcrowd_core::Confidence::Low).count();
    println!(
        "events={} shields={sh} unshields={} fetched_txs={} cached_txs={} secs={:.1}",
        events.len(),
        events.len() - sh,
        r.fetched,
        r.from_cache,
        t0.elapsed().as_secs_f32()
    );
    let skipped: Vec<String> = r.skipped.iter().map(|(k, v)| format!("{k}={v}")).collect();
    println!(
        "excluded: {} ambiguous_cross_pool(kept, low confidence, not counted in crowds)={low}",
        if skipped.is_empty() { "none".into() } else { skipped.join(" ") }
    );
    println!("wrote {} (raw transactions cached in {})", out.display(), cache.0.display());
    println!(
        "tip: sync on a schedule, not right before you transact. The server sees your IP and that you run zcrowd, nothing about your wallet; use Tor or a VPN at system level."
    );
    Ok(())
}
