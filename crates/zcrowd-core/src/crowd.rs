//! Amount-and-timing crowd scoring (spec 41 A1-A5, A11, B2).
//!
//! Adversary model: an observer matches an unshield to earlier entries into the same shielded pool by amount,
//! allowing for the fee, and weighs recent matches more. This is a heuristic; it says nothing about graph, wallet
//! or network leaks, and a link through a visible move between pools is not counted.
use crate::model::{Confidence, Event, Kind, Pool, sig_decimals};

pub const DAY: u64 = 86_400;
/// A "send max" round trip pays the fee out of the shielded note, so the recipient gets pool_in - fee.
/// ZIP-317 fees are 5000 zat x max(2, actions); legacy wallets used 1000 or 10000.
pub const DEFAULT_FEE_ALLOWANCE: u64 = 50_000;

#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub lookback_days: u64,
    pub tol_zat: u64,
    pub fee_allowance_zat: u64,
    /// Reference time (unix s): the moment of the hypothetical transaction.
    pub now: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    NoMatch,
    Green,
    Amber,
    Red,
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Verdict::NoMatch => "NO MATCH",
            Verdict::Green => "GREEN",
            Verdict::Amber => "AMBER",
            Verdict::Red => "RED",
        })
    }
}

#[derive(Clone, Debug)]
pub struct Crowd {
    /// The pool whose entries were counted.
    pub pool: Pool,
    pub k: usize,
    pub weighted: f64,
    pub verdict: Verdict,
    /// 5 or more significant decimals: advisory only, never changes the verdict.
    pub odd_amount: bool,
    /// Up to 10 most recent matching shields.
    pub matches: Vec<Event>,
}

/// An entry into `pool` (spec 41 A11): a High-confidence shield (a shielded coinbase included) or move whose pool is
/// `pool`, inside the lookback window and the amount window. Low-confidence multi-pool events are never entries.
pub(crate) fn in_scope(e: &Event, user: Kind, pool: Pool, zat: u64, p: &Params) -> bool {
    let (lo, hi) = bounds(user, zat, p);
    matches!(e.kind, Kind::Shield | Kind::Move)
        && e.pool == pool
        && e.confidence == Confidence::High
        && e.t <= p.now
        && e.t >= p.now.saturating_sub(p.lookback_days * DAY)
        && (lo..=hi).contains(&e.zat)
}

/// Amount window of shields that can be tied to `zat`.
/// Unshield: [X - tol, X + fee_allowance + tol] (the recipient gets pool_in minus the fee).
/// Shield: [X - tol, X + tol] (the decoys for your later send-max unshield are other shields of about X).
pub fn bounds(user: Kind, zat: u64, p: &Params) -> (u64, u64) {
    let allow = if user == Kind::Unshield { p.fee_allowance_zat } else { 0 };
    (zat.saturating_sub(p.tol_zat), zat.saturating_add(allow).saturating_add(p.tol_zat))
}

/// (count, weighted count) of entries into `pool`. Entries older than 14 days weigh 0.5.
pub fn count(events: &[Event], user: Kind, pool: Pool, zat: u64, p: &Params) -> (usize, f64) {
    events
        .iter()
        .filter(|e| in_scope(e, user, pool, zat, p))
        .fold((0, 0.0), |(k, w), e| (k + 1, w + if p.now - e.t > 14 * DAY { 0.5 } else { 1.0 }))
}

pub fn verdict_for(user: Kind, k: usize, weighted: f64) -> Verdict {
    match user {
        Kind::Unshield if k == 0 => Verdict::NoMatch,
        Kind::Unshield if weighted <= 1.0 => Verdict::Red,
        Kind::Shield if k == 0 => Verdict::Red,
        _ if weighted < 10.0 => Verdict::Amber,
        _ => Verdict::Green,
    }
}

/// `user` is what the user plans to do; `pool` is where their funds are (unshield) or will arrive (shield).
pub fn score(events: &[Event], user: Kind, pool: Pool, zat: u64, p: &Params) -> Crowd {
    let (k, weighted) = count(events, user, pool, zat, p);
    let mut matches: Vec<Event> = events.iter().filter(|e| in_scope(e, user, pool, zat, p)).cloned().collect();
    matches.sort_by(|a, b| b.t.cmp(&a.t).then_with(|| a.txid.cmp(&b.txid)));
    matches.truncate(10);
    Crowd { pool, k, weighted, verdict: verdict_for(user, k, weighted), odd_amount: sig_decimals(zat) >= 5, matches }
}

/// One crowd per pool, in `Pool::SINGLE` order.
pub fn score_pools(events: &[Event], user: Kind, zat: u64, p: &Params) -> [Crowd; 3] {
    Pool::SINGLE.map(|pool| score(events, user, pool, zat, p))
}

/// The worse of two verdicts: RED > AMBER > GREEN > NO MATCH.
pub fn worse(a: Verdict, b: Verdict) -> Verdict {
    let rank = |v| match v {
        Verdict::NoMatch => 0,
        Verdict::Green => 1,
        Verdict::Amber => 2,
        Verdict::Red => 3,
    };
    if rank(b) > rank(a) { b } else { a }
}

/// The pool with the worst verdict (spec 41 A11, `--pool any`). A tie goes to the smaller weighted crowd, then to the
/// pool listed first in `Pool::SINGLE`. web/index.template.html does the same.
pub fn worst(crowds: &[Crowd; 3]) -> &Crowd {
    crowds.iter().skip(1).fold(&crowds[0], |a, b| {
        let b_is_worse = worse(a.verdict, b.verdict) != a.verdict || (a.verdict == b.verdict && b.weighted < a.weighted);
        if b_is_worse { b } else { a }
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddrFinding {
    Clear,
    /// The address received `n` earlier unshields (first/last time).
    UsedAsDestination {
        n: usize,
        first_t: u64,
        last_t: u64,
    },
    /// The address was the transparent source of an earlier shield.
    ShieldSource {
        t: u64,
        txid: String,
    },
}

/// Address reuse, the strongest real-world link: independent of the amount. Offline, on cached public data.
pub fn address_check(events: &[Event], addr: &str, now: u64) -> AddrFinding {
    let past = || events.iter().filter(|e| e.t <= now);
    if let Some(e) = past().filter(|e| e.kind == Kind::Shield && e.src.iter().any(|a| a == addr)).max_by_key(|e| e.t) {
        return AddrFinding::ShieldSource { t: e.t, txid: e.txid.clone() };
    }
    let d: Vec<u64> = past().filter(|e| e.kind == Kind::Unshield && e.dst.iter().any(|a| a == addr)).map(|e| e.t).collect();
    match (d.iter().min(), d.iter().max()) {
        (Some(&first_t), Some(&last_t)) => AddrFinding::UsedAsDestination { n: d.len(), first_t, last_t },
        _ => AddrFinding::Clear,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ZAT_PER_ZEC;

    // E1-E8 are about one pool (Sapling, which `ev` makes); E11-E15 below are about several and call `super::`.
    fn score(events: &[Event], user: Kind, zat: u64, p: &Params) -> Crowd {
        super::score(events, user, Pool::Sapling, zat, p)
    }
    fn count(events: &[Event], user: Kind, zat: u64, p: &Params) -> (usize, f64) {
        super::count(events, user, Pool::Sapling, zat, p)
    }

    fn ev(kind: Kind, zat: u64, t: u64) -> Event {
        Event {
            h: 1,
            t,
            kind,
            zat,
            pool: Pool::Sapling,
            fee_zat: None,
            n_tin: 1,
            n_tout: 0,
            txid: format!("{kind:?}-{t}-{zat}"),
            confidence: Confidence::High,
            coinbase: false,
            src: vec![],
            dst: vec![],
        }
    }
    const NOW: u64 = 100 * DAY;
    const X: u64 = ZAT_PER_ZEC;
    fn p(tol: u64) -> Params {
        Params { lookback_days: 30, tol_zat: tol, fee_allowance_zat: DEFAULT_FEE_ALLOWANCE, now: NOW }
    }
    fn shields(zat: u64, n: u64) -> Vec<Event> {
        (0..n).map(|i| ev(Kind::Shield, zat, NOW - 1000 - i)).collect()
    }

    // E1: the user's own send-max round trip must be found (a false GREEN/NO MATCH here is a privacy harm).
    #[test]
    fn e1_send_max_round_trip_is_found() {
        let evs = vec![ev(Kind::Shield, X + 15_000, NOW - 3600)];
        let c = score(&evs, Kind::Unshield, X, &p(0));
        assert_eq!((c.k, c.verdict), (1, Verdict::Red));
        let none = Params { fee_allowance_zat: 0, ..p(0) };
        assert_eq!(score(&evs, Kind::Unshield, X, &none).k, 0, "without the fee allowance it would be missed");
        // window edges: X + 50000 matches, X + 50001 does not, X - 1 does not
        let edge = |z| score(&[ev(Kind::Shield, z, NOW - 5)], Kind::Unshield, X, &p(0)).k;
        assert_eq!((edge(X + 50_000), edge(X + 50_001), edge(X - 1), edge(X)), (1, 0, 0, 1));
    }

    #[test]
    fn e2_no_shield_near_is_no_match() {
        let c = score(&shields(2 * X, 30), Kind::Unshield, X, &p(0));
        assert_eq!((c.k, c.verdict), (0, Verdict::NoMatch));
    }

    #[test]
    fn e3_twelve_shields_is_green_and_two_is_amber() {
        assert_eq!(score(&shields(X, 12), Kind::Unshield, X, &p(0)).verdict, Verdict::Green);
        assert_eq!(score(&shields(X, 2), Kind::Unshield, X, &p(0)).verdict, Verdict::Amber);
        assert_eq!(score(&shields(X, 12), Kind::Unshield, X, &p(0)).matches.len(), 10);
    }

    #[test]
    fn e4_future_and_out_of_window_excluded_old_weighted() {
        let evs = vec![ev(Kind::Shield, X, NOW + 10), ev(Kind::Shield, X, NOW - 40 * DAY), ev(Kind::Shield, X, NOW - 20 * DAY)];
        let c = score(&evs, Kind::Unshield, X, &p(0));
        assert_eq!((c.k, c.weighted, c.verdict), (1, 0.5, Verdict::Red));
    }

    #[test]
    fn e5_shield_with_transparent_change_counts_low_confidence_does_not() {
        let mut change = ev(Kind::Shield, X, NOW - 10);
        change.n_tin = 2;
        change.n_tout = 1;
        assert_eq!(score(&[change.clone()], Kind::Unshield, X, &p(0)).k, 1);
        change.confidence = Confidence::Low;
        assert_eq!(score(&[change], Kind::Unshield, X, &p(0)).k, 0);
    }

    #[test]
    fn e6_shield_mode_counts_shields_not_unshields() {
        let unsh: Vec<_> = (0..12).map(|i| ev(Kind::Unshield, X, NOW - 10 - i)).collect();
        let c = score(&unsh, Kind::Shield, X, &p(0));
        assert_eq!((c.k, c.verdict), (0, Verdict::Red));
        assert_eq!(score(&shields(X, 12), Kind::Shield, X, &p(0)).verdict, Verdict::Green);
        // no fee allowance in shield mode: a shield of X + 15000 is not a decoy for X
        assert_eq!(score(&shields(X + 15_000, 12), Kind::Shield, X, &p(0)).k, 0);
    }

    #[test]
    fn e7_address_reuse() {
        let mut s = ev(Kind::Shield, X, NOW - 100);
        s.src = vec!["t1src".into()];
        let mut u = ev(Kind::Unshield, X, NOW - 50);
        u.dst = vec!["t1dst".into()];
        let evs = vec![s, u];
        assert!(matches!(address_check(&evs, "t1src", NOW), AddrFinding::ShieldSource { .. }));
        assert!(matches!(address_check(&evs, "t1dst", NOW), AddrFinding::UsedAsDestination { n: 1, .. }));
        assert_eq!(address_check(&evs, "t1other", NOW), AddrFinding::Clear);
        assert_eq!(address_check(&evs, "t1src", NOW - 200), AddrFinding::Clear, "future events are ignored");
    }

    #[test]
    fn e8_widening_never_lowers_k() {
        let evs: Vec<_> = (0..60).map(|i| ev(Kind::Shield, X + i * 1000, NOW - 5000)).collect();
        for user in [Kind::Unshield, Kind::Shield] {
            let mut last = 0;
            for tol in [0, 1000, 5000, 20_000, 1_000_000] {
                for allow in [0, 10_000, 50_000, 100_000] {
                    let k = score(&evs, user, X, &Params { fee_allowance_zat: allow, ..p(tol) }).k;
                    if allow == 50_000 {
                        assert!(k >= last);
                        last = k;
                    }
                }
            }
            let (a, b) = (count(&evs, user, X, &p(0)).0, count(&evs, user, X, &Params { fee_allowance_zat: 90_000, ..p(0) }).0);
            assert!(b >= a);
        }
    }

    #[test]
    fn odd_amount_is_a_note_not_a_verdict() {
        let z = 123_456_789;
        let c = score(&shields(z, 50), Kind::Unshield, z, &p(0));
        assert!(c.odd_amount && c.verdict == Verdict::Green);
        let none = score(&[], Kind::Unshield, z, &p(0));
        assert!(none.odd_amount && none.verdict == Verdict::NoMatch);
    }

    fn ev_in(kind: Kind, pool: Pool, zat: u64, t: u64) -> Event {
        Event { pool, ..ev(kind, zat, t) }
    }
    fn by_pool(evs: &[Event], user: Kind, zat: u64) -> Vec<(Pool, usize, Verdict)> {
        score_pools(evs, user, zat, &p(0)).iter().map(|c| (c.pool, c.k, c.verdict)).collect()
    }

    // E11: a shield whose only shielded part is Ironwood is an entry into Ironwood (it was invisible before A11).
    #[test]
    fn e11_ironwood_only_shield_is_counted() {
        let evs = vec![ev_in(Kind::Shield, Pool::Ironwood, X + 15_000, NOW - 3600)];
        let c = super::score(&evs, Kind::Unshield, Pool::Ironwood, X, &p(0));
        assert_eq!((c.pool, c.k, c.verdict), (Pool::Ironwood, 1, Verdict::Red));
        assert_eq!(super::score(&evs, Kind::Shield, Pool::Ironwood, X + 15_000, &p(0)).k, 1);
    }

    // E12: a Sapling shield of X is no decoy for an Ironwood unshield of X, and `any` is the worst pool.
    #[test]
    fn e12_other_pools_are_not_counted_and_any_is_the_worst_pool() {
        let sapling = shields(X, 1);
        assert_eq!(super::score(&sapling, Kind::Unshield, Pool::Ironwood, X, &p(0)).verdict, Verdict::NoMatch);
        assert_eq!(super::score(&sapling, Kind::Shield, Pool::Ironwood, X, &p(0)).verdict, Verdict::Red);
        let mut evs = sapling;
        evs.extend((0..12).map(|i| ev_in(Kind::Shield, Pool::Ironwood, X, NOW - 100 - i)));
        evs.extend((0..2).map(|i| ev_in(Kind::Shield, Pool::Orchard, X, NOW - 200 - i)));
        let all = by_pool(&evs, Kind::Unshield, X);
        assert_eq!(all, [(Pool::Sapling, 1, Verdict::Red), (Pool::Orchard, 2, Verdict::Amber), (Pool::Ironwood, 12, Verdict::Green)]);
        assert_eq!(worst(&score_pools(&evs, Kind::Unshield, X, &p(0))).pool, Pool::Sapling);
        // the same crowd without the lone Sapling shield: AMBER (Orchard) is the worst of AMBER, GREEN, NO MATCH
        assert_eq!(worst(&score_pools(&evs[1..], Kind::Unshield, X, &p(0))).pool, Pool::Orchard);
    }

    #[test]
    fn worst_pool_order_and_ties() {
        let crowd = |pool, k: usize, weighted: f64, verdict| Crowd { pool, k, weighted, verdict, odd_amount: false, matches: vec![] };
        let (s, o, i) = (Pool::Sapling, Pool::Orchard, Pool::Ironwood);
        // GREEN is worse than NO MATCH
        assert_eq!(
            worst(&[crowd(s, 0, 0.0, Verdict::NoMatch), crowd(o, 20, 20.0, Verdict::Green), crowd(i, 0, 0.0, Verdict::NoMatch)]).pool,
            o
        );
        // equal verdicts: the smaller weighted crowd, then the first pool
        assert_eq!(worst(&[crowd(s, 5, 5.0, Verdict::Amber), crowd(o, 4, 2.0, Verdict::Amber), crowd(i, 4, 2.0, Verdict::Amber)]).pool, o);
        assert_eq!(
            worst(&[crowd(s, 0, 0.0, Verdict::NoMatch), crowd(o, 0, 0.0, Verdict::NoMatch), crowd(i, 0, 0.0, Verdict::NoMatch)]).pool,
            s
        );
        assert_eq!(worst(&[crowd(s, 1, 1.0, Verdict::Red), crowd(o, 1, 0.5, Verdict::Red), crowd(i, 1, 1.0, Verdict::Red)]).pool, o);
    }

    // E13: value that moved into Ironwood from another pool can be what an Ironwood unshield spends.
    #[test]
    fn e13_a_move_into_ironwood_is_a_candidate_for_an_ironwood_unshield() {
        let mv = ev_in(Kind::Move, Pool::Ironwood, X + 15_000, NOW - 60);
        let at = |pool, user| super::score(std::slice::from_ref(&mv), user, pool, X, &p(0)).k;
        assert_eq!((at(Pool::Ironwood, Kind::Unshield), at(Pool::Orchard, Kind::Unshield), at(Pool::Sapling, Kind::Unshield)), (1, 0, 0));
        // a move is also a decoy for someone who is about to shield the same amount into that pool
        assert_eq!(super::score(&[mv], Kind::Shield, Pool::Ironwood, X + 15_000, &p(0)).k, 1);
    }

    // E15: a shielded coinbase is an entry like any other shield.
    #[test]
    fn e15_a_shielded_coinbase_is_an_entry() {
        let mut cb = ev_in(Kind::Shield, Pool::Ironwood, X + 15_000, NOW - 60);
        cb.coinbase = true;
        let c = super::score(&[cb], Kind::Unshield, Pool::Ironwood, X, &p(0));
        assert_eq!((c.k, c.matches.len(), c.matches[0].coinbase), (1, 1, true));
    }

    #[test]
    fn low_confidence_multi_pool_events_are_no_entries_but_their_addresses_count() {
        let mut shield = ev_in(Kind::Shield, Pool::Mixed, X, NOW - 100);
        shield.confidence = Confidence::Low;
        shield.src = vec!["t1src".into()];
        let mut unshield = ev_in(Kind::Unshield, Pool::Mixed, X, NOW - 50);
        unshield.confidence = Confidence::Low;
        unshield.dst = vec!["t1dst".into()];
        let evs = vec![shield, unshield];
        assert!(by_pool(&evs, Kind::Unshield, X).iter().all(|c| c.1 == 0));
        assert!(by_pool(&evs, Kind::Shield, X).iter().all(|c| c.1 == 0));
        assert!(matches!(address_check(&evs, "t1src", NOW), AddrFinding::ShieldSource { .. }));
        assert!(matches!(address_check(&evs, "t1dst", NOW), AddrFinding::UsedAsDestination { n: 1, .. }));
    }

    #[test]
    fn shield_mode_counts_entries_into_the_chosen_pool_only() {
        let evs: Vec<_> = (0..12).map(|i| ev_in(Kind::Shield, Pool::Orchard, X, NOW - 10 - i)).collect();
        assert_eq!(
            by_pool(&evs, Kind::Shield, X),
            [(Pool::Sapling, 0, Verdict::Red), (Pool::Orchard, 12, Verdict::Green), (Pool::Ironwood, 0, Verdict::Red)]
        );
    }
}
