use anyhow::{Context, Result, anyhow, bail};
use std::path::Path;
use zcrowd_core::decode::{Decoded, event_from_tx};
use zcrowd_core::ingest::RawCache;
use zcrowd_core::model::read_ndjson;

/// Recompute one event from the cached raw transaction. Offline by construction: asking a server for a
/// single transaction would tell it which one you care about.
pub fn run(txid: &str, data: &Path, raw_dir: Option<&Path>) -> Result<()> {
    let txid = txid.to_ascii_lowercase();
    let events = read_ndjson(data).with_context(|| format!("cannot read {}", data.display()))?;
    let cached = events.iter().find(|e| e.txid == txid).ok_or_else(|| anyhow!("{txid} is not an event in {}", data.display()))?;
    let dir = match raw_dir {
        Some(d) => d.to_path_buf(),
        None => data.parent().unwrap_or(Path::new(".")).join("tx"),
    };
    let Some(raw) = RawCache(dir.clone()).get(&txid)? else {
        bail!("no cached raw transaction in {} (sync stores them; verify never contacts a server)", dir.display());
    };
    match event_from_tx(&raw, cached.h, cached.t)? {
        Decoded::Event(ev) => {
            println!("{}", serde_json::to_string(&ev)?);
            if ev == *cached {
                println!("MATCH: recomputed from {} raw bytes, identical to the line in {}", raw.len(), data.display());
                println!("compare on any explorer: shielded value delta / transparent inputs and outputs of {txid}");
                Ok(())
            } else {
                bail!("DIFF: recomputed event differs from the cached line");
            }
        }
        Decoded::Skip(s) => bail!("DIFF: transaction now decodes to no event ({})", s.label()),
    }
}
