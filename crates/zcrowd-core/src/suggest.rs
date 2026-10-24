use crate::crowd::{Crowd, Params, Verdict, in_scope, score, worst};
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
/// match), biggest first; ties: closer to `zat`, then smaller amount. `judged` are the pools the check weighs: its
/// headline crowd is the one pool, or the worst of several (`--pool any`). Every amount, `zat` included, is scored as the
/// check would score it, so a suggestion never promises more than the check shows (research/46 F2). The amounts offered are
/// those `pool` (the one that decided the verdict) saw entries at. `judged` is never empty.
/// shortcut: an amount only another judged pool saw entries at is not offered, though checking it could read better.
pub fn popular(events: &[Event], user: Kind, pool: Pool, judged: &[Pool], zat: u64, p: &Params, n: usize) -> Vec<Suggestion> {
    let (lo, hi) = (zat - zat / 10, zat.saturating_add(zat / 10));
    // Only entries whose amount lies in some candidate's window can count. Every candidate is scored in every judged pool, so
    // walk those few entries instead of all the data each time (a debug build, as `cargo run` makes, took 4x longer without).
    let reach = lo.saturating_sub(p.tol_zat)..=hi.saturating_add(p.fee_allowance_zat).saturating_add(p.tol_zat);
    let near: Vec<Event> = events.iter().filter(|e| reach.contains(&e.zat)).cloned().collect();
    let events = &near[..];
    let headline = |a: u64| {
        let crowds: Vec<Crowd> = judged.iter().map(|&q| score(events, user, q, a, p)).collect();
        let w = worst(&crowds);
        (w.k, w.weighted, w.verdict)
    };
    let base = headline(zat).1.max(1.0);
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
            let (k, weighted, verdict) = headline(a);
            Suggestion { zat: a, k, weighted, verdict }
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
        sh_in(Pool::Orchard, zat, i)
    }

    fn sh_in(pool: Pool, zat: u64, i: u64) -> Event {
        Event {
            h: 1,
            t: 1000 + i,
            kind: Kind::Shield,
            zat,
            pool,
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
            let base = crate::crowd::count(&evs, user, Pool::Orchard, x, &p).1;
            let s = popular(&evs, user, Pool::Orchard, &[Pool::Orchard], x, &p, 5);
            assert!(!s.is_empty() && s.iter().all(|c| c.weighted >= base));
            assert_eq!(s[0].zat, ZAT_PER_ZEC);
            assert!(s.windows(2).all(|w| w[0].weighted >= w[1].weighted));
        }
    }

    // research/46 F2. 2.0 ZEC went into Sapling once and Ironwood 20 times, 2.1 into Sapling 15 times and Ironwood once, 1.95
    // into both 12 times.
    #[test]
    fn a_suggestion_is_judged_by_the_worst_of_the_pools_the_check_weighs() {
        let mut evs = vec![];
        for (zat, sapling, ironwood) in [(200_000_000, 1, 20), (210_000_000, 15, 1), (195_000_000, 12, 12)] {
            evs.extend((0..sapling).map(|i| sh_in(Pool::Sapling, zat, i)));
            evs.extend((0..ironwood).map(|i| sh_in(Pool::Ironwood, zat, i)));
        }
        let p = Params { lookback_days: 30, tol_zat: 0, fee_allowance_zat: DEFAULT_FEE_ALLOWANCE, now: 5000 };
        let offered = |judged: &[Pool]| -> Vec<(u64, usize)> {
            popular(&evs, Kind::Unshield, Pool::Sapling, judged, 200_000_000, &p, 5).iter().map(|s| (s.zat, s.k)).collect()
        };
        // all three weighed: 2.1 is GREEN in Sapling but RED in Ironwood, so only 1.95 is a way out of Sapling's k=1
        assert_eq!(offered(&Pool::SINGLE), vec![(195_000_000, 12)]);
        // Sapling alone: 2.1 is the biggest crowd
        assert_eq!(offered(&[Pool::Sapling]), vec![(210_000_000, 15), (195_000_000, 12)]);
    }
}
