use crate::decode::{Decoded, event_from_tx};
use crate::model::{Coverage, Event, Kind};
use crate::source::BlockSource;
use anyhow::{Result, bail};
use futures::{StreamExt, stream};
use std::collections::BTreeMap;
use std::path::PathBuf;

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
    /// Of shield_like: coinbase transactions (shielded block rewards).
    pub coinbase_like: usize,
    pub coverage: Option<Coverage>,
}

/// Shield-like: transparent inputs + shielded activity. Unshield-like: no transparent inputs, transparent
/// outputs + shielded activity. Exact amounts come later from the full transaction.
/// The requests made depend only on [start, end] and on which transactions are candidates in it.
pub async fn candidates<S: BlockSource>(src: &S, start: u64, end: u64) -> Result<(Vec<Cand>, Stats)> {
    let (mut out, mut st) = (vec![], Stats::default());
    let mut a = start;
    while a <= end {
        let b = (a + CHUNK - 1).min(end);
        for blk in src.blocks(a, b).await? {
            st.blocks += 1;
            let cov = st.coverage.get_or_insert(Coverage { first_h: blk.h, first_t: blk.t, last_h: blk.h, last_t: blk.t });
            cov.last_h = blk.h;
            cov.last_t = blk.t;
            for t in &blk.txs {
                st.txs += 1;
                if !t.shielded {
                    continue;
                }
                let kind = if t.n_vin > 0 {
                    st.shield_like += 1;
                    st.coinbase_like += usize::from(t.coinbase);
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

/// Raw transactions kept on disk (public chain data) so that decoding and `verify` work offline.
pub struct RawCache(pub PathBuf);

impl RawCache {
    fn path(&self, txid: &str) -> Result<PathBuf> {
        if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("not a txid: {txid:?}");
        }
        Ok(self.0.join(format!("{}.hex", txid.to_ascii_lowercase())))
    }

    pub fn get(&self, txid: &str) -> Result<Option<Vec<u8>>> {
        match std::fs::read_to_string(self.path(txid)?) {
            Ok(s) => Ok(Some(hex::decode(s.trim())?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn put(&self, txid: &str, raw: &[u8]) -> Result<()> {
        std::fs::create_dir_all(&self.0)?;
        let p = self.path(txid)?;
        let tmp = p.with_extension("tmp");
        std::fs::write(&tmp, hex::encode(raw))?;
        std::fs::rename(tmp, p)?;
        Ok(())
    }
}

#[derive(Default, Debug)]
pub struct Resolved {
    pub events: Vec<Event>,
    /// Transactions that produced no event, by reason.
    pub skipped: BTreeMap<&'static str, usize>,
    pub fetched: usize,
    pub from_cache: usize,
}

/// Decode every candidate, `concurrency` at a time (capped at 8): from the raw cache when present,
/// otherwise fetched from `src` and cached. Requests are made in candidate order.
pub async fn resolve<S: BlockSource>(src: &S, cands: &[Cand], cache: &RawCache, concurrency: usize) -> Result<Resolved> {
    let mut results = stream::iter(cands)
        .map(|c| async move {
            let (raw, cached) = match cache.get(&c.txid)? {
                Some(r) => (r, true),
                None => {
                    let (r, _) = src.raw_tx(&c.txid).await?;
                    cache.put(&c.txid, &r)?;
                    (r, false)
                }
            };
            // A transaction we cannot parse is counted and reported, never silently dropped.
            anyhow::Ok((event_from_tx(&raw, c.h, c.t).ok(), cached))
        })
        .buffered(concurrency.clamp(1, MAX_CONCURRENCY));
    let mut r = Resolved::default();
    while let Some(x) = results.next().await {
        let (d, cached) = x?;
        if cached { r.from_cache += 1 } else { r.fetched += 1 }
        match d {
            Some(Decoded::Event(e)) => r.events.push(e),
            Some(Decoded::Skip(s)) => *r.skipped.entry(s.label()).or_default() += 1,
            None => *r.skipped.entry("undecodable").or_default() += 1,
        }
    }
    Ok(r)
}
