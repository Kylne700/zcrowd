use crate::decode::event_from_tx;
use crate::model::{Event, Kind};
use crate::source::BlockSource;
use anyhow::Result;
use futures::{StreamExt, stream};
use std::collections::HashSet;

pub const CHUNK: u64 = 2000;
pub const MAX_CONCURRENCY: usize = 8;

#[derive(Clone, Debug)]
pub struct Cand {
    pub h: u64,
    pub t: u64,
    pub txid: String,
    pub kind: Kind,
}

#[derive(Default, Debug)]
pub struct Stats {
    pub blocks: usize,
    pub txs: usize,
    pub shield_like: usize,
    pub unshield_like: usize,
}

/// Shield-like: transparent inputs + shielded activity. Unshield-like: no transparent inputs, transparent
/// outputs + shielded activity. Coinbase is skipped. Exact amounts come later from the full tx.
pub async fn candidates<S: BlockSource>(src: &S, start: u64, end: u64) -> Result<(Vec<Cand>, Stats)> {
    let (mut out, mut st) = (vec![], Stats::default());
    let mut a = start;
    while a <= end {
        let b = (a + CHUNK - 1).min(end);
        for blk in src.blocks(a, b).await? {
            st.blocks += 1;
            for t in &blk.txs {
                st.txs += 1;
                if !t.shielded || t.coinbase {
                    continue;
                }
                let kind = if t.n_vin > 0 {
                    st.shield_like += 1;
                    Kind::Shield
                } else if t.n_vout > 0 {
                    st.unshield_like += 1;
                    Kind::Unshield
                } else {
                    continue;
                };
                out.push(Cand { h: blk.h, t: blk.t, txid: t.txid.clone(), kind });
            }
        }
        a = b + 1;
    }
    Ok((out, st))
}

/// Fetch and decode every candidate not in `have`, `concurrency` at a time (capped at 8), handing each
/// new event to `sink` in order. Returns the number of full transactions fetched.
pub async fn resolve<S: BlockSource>(
    src: &S,
    cands: &[Cand],
    have: &HashSet<String>,
    concurrency: usize,
    mut sink: impl FnMut(Event) -> Result<()>,
) -> Result<usize> {
    let todo: Vec<&Cand> = cands.iter().filter(|c| !have.contains(&c.txid)).collect();
    let n = todo.len();
    let mut results = stream::iter(todo)
        .map(|c| async move {
            let (raw, _) = src.raw_tx(&c.txid).await?;
            event_from_tx(&raw, c.h, c.t)
        })
        .buffered(concurrency.clamp(1, MAX_CONCURRENCY));
    while let Some(r) = results.next().await {
        if let Some(ev) = r? {
            sink(ev)?;
        }
    }
    Ok(n)
}
