use crate::check::secret_line;
use anyhow::{Result, anyhow, bail};
use std::path::Path;
use zcrowd_core::decode::{Decoded, event_from_tx};
use zcrowd_core::ingest::RawCache;
use zcrowd_core::model::load;

/// Recompute one event from the cached raw transaction. Offline by construction: asking a server for a
/// single transaction would tell it which one you care about.
/// `-` reads the transaction id privately (no echo on a terminal, else one line of stdin); one given as an argument
/// stays in shell history, which says which transaction you looked at.
pub fn run(txid: &str, data: &Path, raw_dir: Option<&Path>) -> Result<()> {
    let txid = if txid == "-" {
        secret_line("Transaction id (not shown): ")?
    } else {
        eprintln!(
            "note: a transaction id given as an argument is visible in shell history and the process list; use `-` to type it privately"
        );
        txid.to_string()
    };
    let txid = txid.trim().to_ascii_lowercase();
    if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("not a transaction id (64 hexadecimal characters)");
    }
    let (events, _) = load(data)?;
    let cached = events.iter().find(|e| e.txid == txid).ok_or_else(|| anyhow!("that transaction is not an event in {}", data.display()))?;
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
                println!(
                    "next: compare its shielded value delta and its transparent inputs and outputs with a block explorer. Spot-check random events, not your own matches, and open the explorer over Tor: it sees which transaction you look up"
                );
                Ok(())
            } else {
                bail!("DIFF: recomputed event differs from the cached line");
            }
        }
        Decoded::Skip(s) => bail!("DIFF: transaction now decodes to no event ({})", s.label()),
    }
}
