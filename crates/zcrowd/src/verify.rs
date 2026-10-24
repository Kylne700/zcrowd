use crate::Src;
use anyhow::{Result, anyhow};
use std::path::Path;
use zcrowd_core::decode::event_from_tx;
use zcrowd_core::ingest::candidates;
use zcrowd_core::model::read_ndjson;
use zcrowd_core::source::BlockSource;

pub async fn run(src: Src, txid: &str, data: &Path) -> Result<()> {
    match src {
        Src::Net(s) => go(&s, txid, data).await,
        Src::Fix(s) => go(&s, txid, data).await,
    }
}

async fn go<S: BlockSource>(s: &S, txid: &str, data: &Path) -> Result<()> {
    let (raw, h) = s.raw_tx(txid).await?;
    let t = s.block_time(h).await?;
    let ev = event_from_tx(&raw, h, t)?.ok_or_else(|| anyhow!("{txid}: no transparent/shielded crossing (not an event)"))?;
    let line = serde_json::to_string(&ev)?;
    println!("{line}");
    if let Ok(cached) = read_ndjson(data) {
        match cached.iter().find(|e| e.txid == ev.txid) {
            Some(c) if *c == ev => println!("MATCH: identical to cached line in {}", data.display()),
            Some(_) => println!("DIFF: cached line differs"),
            None => println!("not in cache {}", data.display()),
        }
    }
    Ok(())
}

/// Developer helper used once to create the committed fixtures: compact summary of one block plus the
/// raw hex of every shield/unshield candidate in it.
pub async fn fixture(src: Src, height: u64, dir: &Path) -> Result<()> {
    let Src::Net(s) = src else { return Err(anyhow!("fixture needs a live server")) };
    std::fs::create_dir_all(dir)?;
    let blocks = s.blocks(height, height).await?;
    std::fs::write(dir.join(format!("blocks-{height}.json")), serde_json::to_vec_pretty(&blocks)?)?;
    let (cands, _) = candidates(&s, height, height).await?;
    for c in cands {
        let (raw, _) = s.raw_tx(&c.txid).await?;
        std::fs::write(dir.join(format!("tx-{}.hex", c.txid)), hex_of(&raw))?;
        println!("{} {:?}", c.txid, c.kind);
    }
    Ok(())
}

fn hex_of(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
