use crate::crowd::{Crowd, Params, score};
use crate::model::{Confidence, Event, Kind};
use std::collections::BTreeSet;

/// Up to `n` amounts within +-10% of `zat` whose crowd is at least as big as the original, biggest first.
pub fn popular(events: &[Event], user: Kind, zat: u64, p: &Params, n: usize) -> Vec<(u64, Crowd)> {
    let (lo, hi) = (zat - zat / 10, zat + zat / 10);
    let base = score(events, user, zat, p).weighted;
    let cands: BTreeSet<u64> = events
        .iter()
        .filter(|e| e.kind == user.opposite() && e.confidence == Confidence::High && (lo..=hi).contains(&e.zat))
        .map(|e| e.zat)
        .collect();
    let mut out: Vec<(u64, Crowd)> = cands
        .into_iter()
        .filter(|&a| a != zat)
        .map(|a| (a, score(events, user, a, p)))
        .filter(|(_, c)| !c.odd_amount && c.weighted >= base)
        .collect();
    out.sort_by(|a, b| b.1.weighted.total_cmp(&a.1.weighted).then(a.0.abs_diff(zat).cmp(&b.0.abs_diff(zat))));
    out.truncate(n);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Pool, ZAT_PER_ZEC};
    #[test]
    fn suggestions_not_smaller() {
        let mk = |zat, i: u64| Event { h: 1, t: 1000 + i, kind: Kind::Shield, zat, pool: Pool::Orchard, fee_zat: None, n_tin: 1, n_tout: 0, txid: format!("{zat}{i}"), confidence: Confidence::High, coinbase: false, src: vec![], dst: vec![] };
        let mut evs: Vec<Event> = (0..20).map(|i| mk(ZAT_PER_ZEC, i)).collect();
        evs.extend((0..3).map(|i| mk(ZAT_PER_ZEC * 105 / 100, i)));
        let p = Params { lookback_days: 30, tol_zat: 0, now: 5000 };
        let x = ZAT_PER_ZEC * 102 / 100;
        let base = score(&evs, Kind::Unshield, x, &p).weighted;
        let s = popular(&evs, Kind::Unshield, x, &p, 5);
        assert!(!s.is_empty() && s.iter().all(|(_, c)| c.weighted >= base));
        assert_eq!(s[0].0, ZAT_PER_ZEC);
    }
}
