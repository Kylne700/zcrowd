use crate::crowd::{Params, Verdict, count, in_scope, verdict_for};
use crate::model::{Event, Kind, Pool};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct Suggestion {
    pub zat: u64,
    pub k: usize,
    pub weighted: f64,
    pub verdict: Verdict,
}

/// Up to `n` amounts within +-10% of `zat` that have a bigger crowd than `zat` itself (and more than one
/// match), biggest first; ties: closer to `zat`, then smaller amount. Scored with exactly the same
/// window and weights as `crowd::score`, so a suggestion never promises more than the check shows.
pub fn popular(events: &[Event], user: Kind, pool: Pool, zat: u64, p: &Params, n: usize) -> Vec<Suggestion> {
    let (lo, hi) = (zat - zat / 10, zat.saturating_add(zat / 10));
    let base = count(events, user, pool, zat, p).1.max(1.0);
    // Candidates are amounts that entries into `pool` actually used inside the lookback window. Asking `in_scope` for
    // the entry's own amount keeps this list and the count below on one definition of "entry".
    let cands: BTreeSet<u64> = events
        .iter()
        .filter(|e| in_scope(e, user, pool, e.zat, &Params { tol_zat: 0, fee_allowance_zat: 0, ..*p }))
        .map(|e| e.zat)
        .filter(|z| (lo..=hi).contains(z) && *z != zat)
        .collect();
    let mut out: Vec<Suggestion> = cands
        .into_iter()
        .map(|a| {
            let (k, weighted) = count(events, user, pool, a, p);
            Suggestion { zat: a, k, weighted, verdict: verdict_for(user, k, weighted) }
        })
        .filter(|s| s.weighted > base)
        .collect();
    out.sort_by(|a, b| b.weighted.total_cmp(&a.weighted).then(a.zat.abs_diff(zat).cmp(&b.zat.abs_diff(zat))).then(a.zat.cmp(&b.zat)));
    out.truncate(n);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crowd::DEFAULT_FEE_ALLOWANCE;
    use crate::model::{Confidence, ZAT_PER_ZEC};

    fn sh(zat: u64, i: u64) -> Event {
        Event {
            h: 1,
            t: 1000 + i,
            kind: Kind::Shield,
            zat,
            pool: Pool::Orchard,
            fee_zat: None,
            n_tin: 1,
            n_tout: 0,
            txid: format!("{zat}-{i}"),
            confidence: Confidence::High,
            coinbase: false,
            src: vec![],
            dst: vec![],
        }
    }

    #[test]
    fn suggestions_not_smaller_and_ordered() {
        let mut evs: Vec<Event> = (0..20).map(|i| sh(ZAT_PER_ZEC, i)).collect();
        evs.extend((0..3).map(|i| sh(ZAT_PER_ZEC * 105 / 100, i)));
        for user in [Kind::Unshield, Kind::Shield] {
            let p = Params { lookback_days: 30, tol_zat: 0, fee_allowance_zat: DEFAULT_FEE_ALLOWANCE, now: 5000 };
            let x = ZAT_PER_ZEC * 102 / 100;
            let base = count(&evs, user, Pool::Orchard, x, &p).1;
            let s = popular(&evs, user, Pool::Orchard, x, &p, 5);
            assert!(!s.is_empty() && s.iter().all(|c| c.weighted >= base));
            assert_eq!(s[0].zat, ZAT_PER_ZEC);
            assert!(s.windows(2).all(|w| w[0].weighted >= w[1].weighted));
        }
    }
}
