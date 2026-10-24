use crate::decode::{Decoded, Skip, event_from_tx, txid_of};
use crate::model::{Coverage, Event, Kind};
use crate::source::{BlockSource, BlockSummary};
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

/// Spec 41 A9, for a server as the source: heights must be consecutive from `start`, and each block's prev_hash must be
/// the hash of the block before. Catches a server that skips blocks, cuts a range short or splices two forks.
/// It cannot tell a consistent liar from the real chain (the compact blocks carry no proof of work).
fn link(blk: &BlockSummary, prev: &mut Option<(u64, String)>, start: u64) -> Result<()> {
    let want = prev.as_ref().map_or(start, |p| p.0 + 1);
    if blk.h != want {
        bail!("expected block {want} but the server sent block {}: refusing incomplete data (try another --server)", blk.h);
    }
    let hex32 = |h: &str| h.len() == 64 && h.bytes().all(|c| c.is_ascii_hexdigit());
    if !hex32(&blk.hash) || !hex32(&blk.prev_hash) {
        bail!("block {} came without a 32-byte hash and previous hash (try another --server)", blk.h);
    }
    if let Some((h, hash)) = prev.as_ref()
        && blk.prev_hash != *hash
    {
        bail!(
            "block {} does not follow block {h}: its prev_hash is not that block's hash, so the server's chain is inconsistent (try another --server)",
            blk.h
        );
    }
    *prev = Some((blk.h, blk.hash.clone()));
    Ok(())
}

/// Shield-like: transparent inputs + shielded activity. Unshield-like: no transparent inputs, transparent
/// outputs + shielded activity. Exact amounts come later from the full transaction.
/// The requests made depend only on [start, end] and on which transactions are candidates in it.
pub async fn candidates<S: BlockSource>(src: &S, start: u64, end: u64) -> Result<(Vec<Cand>, Stats)> {
    let (mut out, mut st) = (vec![], Stats::default());
    let (strict, mut prev) = (src.strict_chain(), None);
    let mut a = start;
    while a <= end {
        let b = (a + CHUNK - 1).min(end);
        for blk in src.blocks(a, b).await? {
            if strict {
                link(&blk, &mut prev, start)?;
            }
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
    if strict && prev.as_ref().map(|p| p.0) != Some(end) {
        bail!("the server stopped at block {:?} but {end} was asked for (try another --server)", prev.map(|p| p.0));
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
            // The server is not trusted (spec 41 A9): bytes count only if they hash to the id that was asked for.
            // A cached file gets the same test; one that fails is fetched again.
            let cached = cache.get(&c.txid)?.filter(|r| txid_of(r, c.h).is_ok_and(|id| id == c.txid));
            let cached_hit = cached.is_some();
            let raw = match cached {
                Some(r) => r,
                None => src.raw_tx(&c.txid).await?.0,
            };
            // Wrong id and unparsable bytes are counted and reported, never silently dropped, and never cached.
            let d = match txid_of(&raw, c.h) {
                Ok(id) if id == c.txid => {
                    if !cached_hit {
                        cache.put(&c.txid, &raw)?;
                    }
                    event_from_tx(&raw, c.h, c.t).ok()
                }
                Ok(_) => Some(Decoded::Skip(Skip::TxidMismatch)),
                Err(_) => None,
            };
            anyhow::Ok((d, cached_hit))
        })
        .buffered(concurrency.clamp(1, MAX_CONCURRENCY));
    let mut r = Resolved::default();
    while let Some(x) = results.next().await {
        let (d, cached) = x?;
        if cached {
            r.from_cache += 1
        } else {
            r.fetched += 1
        }
        match d {
            Some(Decoded::Event(e)) => r.events.push(e),
            Some(Decoded::Skip(s)) => *r.skipped.entry(s.label()).or_default() += 1,
            None => *r.skipped.entry("undecodable").or_default() += 1,
        }
    }
    Ok(r)
}
