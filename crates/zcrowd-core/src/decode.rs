use crate::model::{Confidence, Event, Kind, Pool};
use anyhow::{Context, Result};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::{BlockHeight, BranchId, MainNetwork};

/// Turn a raw mainnet transaction into a turnstile event, or None if it has no transparent part
/// or no net movement between the transparent and shielded sides.
///
/// pool_in = -(sapling value_balance + orchard value_balance); positive = value entered shielded pools.
/// - shield (pool_in > 0): zat = pool_in.
/// - unshield (pool_in < 0): zat = sum of transparent outputs when the tx has no transparent inputs
///   (the public amount an explorer shows; fee_zat = pool decrease - outputs), else the pool decrease.
/// Transactions with both transparent inputs and outputs are marked low confidence.
pub fn event_from_tx(raw: &[u8], height: u64, time: u64) -> Result<Option<Event>> {
    let branch = BranchId::for_height(&MainNetwork, BlockHeight::from_u32(height as u32));
    let tx = Transaction::read(raw, branch).context("parse transaction")?;
    let Some(tb) = tx.transparent_bundle() else { return Ok(None) };
    if tb.is_coinbase() {
        return Ok(None);
    }
    let sap = tx.sapling_bundle().map(|b| i64::from(*b.value_balance())).unwrap_or(0);
    let orc = tx.orchard_bundle().map(|b| i64::from(*b.value_balance())).unwrap_or(0);
    let pool_in = -(sap + orc);
    let (n_tin, n_tout) = (tb.vin.len(), tb.vout.len());
    let t_out: u64 = tb.vout.iter().map(|o| u64::from(o.value())).sum();
    let pool = match (sap != 0, orc != 0) {
        (true, true) => Pool::Mixed,
        (false, true) => Pool::Orchard,
        (true, false) => Pool::Sapling,
        (false, false) => return Ok(None),
    };
    let (kind, zat, fee_zat) = match pool_in {
        0 => return Ok(None),
        p if p > 0 => (Kind::Shield, p as u64, None),
        p => {
            let drop = p.unsigned_abs();
            if n_tin == 0 { (Kind::Unshield, t_out, Some(drop.saturating_sub(t_out))) } else { (Kind::Unshield, drop, None) }
        }
    };
    if zat == 0 {
        return Ok(None);
    }
    let confidence = if n_tin > 0 && n_tout > 0 { Confidence::Low } else { Confidence::High };
    Ok(Some(Event { h: height, t: time, kind, zat, pool, fee_zat, n_tin, n_tout, txid: tx.txid().to_string(), confidence }))
}
