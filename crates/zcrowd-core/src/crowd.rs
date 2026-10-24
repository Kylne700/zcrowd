use crate::model::{Confidence, Event, Kind, sig_decimals};

pub const DAY: u64 = 86_400;

#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub lookback_days: u64,
    pub tol_zat: u64,
    /// Reference time (unix s): the moment of the hypothetical transaction.
    pub now: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Red,
    Amber,
    Green,
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Verdict::Red => "RED",
            Verdict::Amber => "AMBER",
            Verdict::Green => "GREEN",
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Crowd {
    pub k: usize,
    pub weighted: f64,
    pub verdict: Verdict,
    pub odd_amount: bool,
}

/// Opposite-direction events inside the window whose amount is within `tol_zat` of `zat`.
/// Events older than 14 days weigh 0.5. Future events and low-confidence (mixed) events are excluded.
pub fn count(events: &[Event], user: Kind, zat: u64, p: &Params) -> (usize, f64) {
    let from = p.now.saturating_sub(p.lookback_days * DAY);
    let (mut k, mut w) = (0, 0.0);
    for e in events {
        if e.kind != user.opposite()
            || e.confidence != Confidence::High
            || e.t > p.now
            || e.t < from
            || e.zat.abs_diff(zat) > p.tol_zat
        {
            continue;
        }
        k += 1;
        w += if p.now - e.t > 14 * DAY { 0.5 } else { 1.0 };
    }
    (k, w)
}

/// `user` is what the user plans to do: Unshield counts earlier shields; Shield counts unshields
/// (the amount demand on the way out). Heuristic only: says nothing about graph or intra-pool linkage.
pub fn score(events: &[Event], user: Kind, zat: u64, p: &Params) -> Crowd {
    let (k, weighted) = count(events, user, zat, p);
    let odd_amount = sig_decimals(zat) >= 5;
    let verdict = if odd_amount || weighted <= 1.0 {
        Verdict::Red
    } else if weighted < 10.0 {
        Verdict::Amber
    } else {
        Verdict::Green
    };
    Crowd { k, weighted, verdict, odd_amount }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Pool, ZAT_PER_ZEC};

    fn ev(kind: Kind, zat: u64, t: u64) -> Event {
        Event { h: 1, t, kind, zat, pool: Pool::Sapling, fee_zat: None, n_tin: 1, n_tout: 0, txid: format!("{t}-{zat}"), confidence: Confidence::High }
    }
    const NOW: u64 = 100 * DAY;
    fn p(tol: u64) -> Params {
        Params { lookback_days: 30, tol_zat: tol, now: NOW }
    }

    #[test]
    fn none_is_red() {
        let c = score(&[], Kind::Unshield, ZAT_PER_ZEC, &p(0));
        assert_eq!((c.k, c.verdict), (0, Verdict::Red));
    }

    #[test]
    fn twelve_exact_is_green() {
        let evs: Vec<_> = (0..12).map(|i| ev(Kind::Shield, ZAT_PER_ZEC, NOW - 1000 - i)).collect();
        let c = score(&evs, Kind::Unshield, ZAT_PER_ZEC, &p(0));
        assert_eq!((c.k, c.verdict), (12, Verdict::Green));
        // same-direction events do not count
        assert_eq!(score(&evs, Kind::Shield, ZAT_PER_ZEC, &p(0)).k, 0);
    }

    #[test]
    fn widening_tolerance_never_lowers() {
        let evs: Vec<_> = (0..40).map(|i| ev(Kind::Shield, ZAT_PER_ZEC + i * 1000, NOW - 5000)).collect();
        let mut last = 0;
        for tol in [0, 1000, 5000, 20_000, 1_000_000] {
            let k = score(&evs, Kind::Unshield, ZAT_PER_ZEC, &p(tol)).k;
            assert!(k >= last);
            last = k;
        }
    }

    #[test]
    fn future_and_old_excluded_and_weighted() {
        let evs = vec![
            ev(Kind::Shield, ZAT_PER_ZEC, NOW + 10),
            ev(Kind::Shield, ZAT_PER_ZEC, NOW - 40 * DAY),
            ev(Kind::Shield, ZAT_PER_ZEC, NOW - 20 * DAY),
        ];
        let c = score(&evs, Kind::Unshield, ZAT_PER_ZEC, &p(0));
        assert_eq!(c.k, 1);
        assert_eq!(c.weighted, 0.5);
    }

    #[test]
    fn odd_amount_red_even_if_crowded() {
        let z = 123_456_789;
        let evs: Vec<_> = (0..50).map(|i| ev(Kind::Shield, z, NOW - 10 - i)).collect();
        let c = score(&evs, Kind::Unshield, z, &p(0));
        assert!(c.k == 50 && c.verdict == Verdict::Red && c.odd_amount);
    }

    #[test]
    fn low_confidence_excluded() {
        let mut e = ev(Kind::Shield, ZAT_PER_ZEC, NOW - 10);
        e.confidence = Confidence::Low;
        assert_eq!(score(&[e], Kind::Unshield, ZAT_PER_ZEC, &p(0)).k, 0);
    }
}
