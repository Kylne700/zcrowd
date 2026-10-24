use crate::Src;
use anyhow::Result;
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::time::Instant;
use zcrowd_core::ingest::{candidates, resolve};
use zcrowd_core::model::{Event, read_ndjson};
use zcrowd_core::source::BlockSource;

const BLOCKS_PER_DAY: u64 = 1152; // 75 s blocks

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
    let tip = s.tip().await?;
    let start = tip.saturating_sub(days * BLOCKS_PER_DAY);
    let (cands, st) = candidates(s, start, tip).await?;
    println!(
        "blocks={} txs={} shield_like={} unshield_like={} range={}..{} secs={:.1}",
        st.blocks, st.txs, st.shield_like, st.unshield_like, start, tip, t0.elapsed().as_secs_f32()
    );
    if candidates_only {
        return Ok(());
    }
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut events: Vec<Event> = if out.exists() { read_ndjson(out)? } else { vec![] };
    let have: HashSet<String> = events.iter().map(|e| e.txid.clone()).collect();
    // Append as we go so an interrupted run resumes where it stopped.
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(out)?;
    let mut new = 0;
    let fetched = resolve(s, &cands, &have, concurrency, |ev| {
        writeln!(f, "{}", serde_json::to_string(&ev)?)?;
        events.push(ev);
        new += 1;
        Ok(())
    })
    .await?;
    drop(f);
    // Canonical order, one line per txid: re-running is idempotent.
    events.sort_by(|a, b| (a.h, &a.txid).cmp(&(b.h, &b.txid)));
    events.dedup_by(|a, b| a.txid == b.txid);
    let tmp = out.with_extension("tmp");
    let mut w = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
    for e in &events {
        writeln!(w, "{}", serde_json::to_string(e)?)?;
    }
    w.flush()?;
    std::fs::rename(&tmp, out)?;
    let sh = events.iter().filter(|e| e.kind == zcrowd_core::Kind::Shield).count();
    println!(
        "fetched_txs={fetched} new_events={new} total_events={} shields={sh} unshields={} secs={:.1}",
        events.len(),
        events.len() - sh,
        t0.elapsed().as_secs_f32()
    );
    println!("wrote {}", out.display());
    Ok(())
}
