//! Checks on the committed sample (fixtures/flows-sample.ndjson, made by `zcrowd sync` with all three shielded pools):
//! the 20 transactions that research/44 compared with public explorers, and the false NO MATCH that research/43 S1-1
//! reproduced with Ironwood-only shields.
use std::path::PathBuf;
use zcrowd_core::crowd::{DEFAULT_FEE_ALLOWANCE, Params, Verdict, count, score, score_pools, worst};
use zcrowd_core::model::{read_coverage, read_ndjson};
use zcrowd_core::{Confidence, Event, Kind, Pool};

fn sample() -> (Vec<Event>, u64) {
    let p: PathBuf = [env!("CARGO_MANIFEST_DIR"), "../../fixtures/flows-sample.ndjson"].iter().collect();
    (read_ndjson(&p).unwrap(), read_coverage(&p).unwrap().last_t)
}

fn params(now: u64) -> Params {
    Params { lookback_days: 30, tol_zat: 0, fee_allowance_zat: DEFAULT_FEE_ALLOWANCE, now }
}

// The 20 events of research/44 (read from two explorers on 2026-10-24 between 14:35 and 14:48 UTC: Cipherscan
// api.mainnet.cipherscan.app/api/tx/<txid>, zcashexplorer.app, and Blockchair for the older ones). Before the Ironwood
// fix 4 of them were wrong (#1, #6, #7, #9). `zat` is the explorer's amount for a plain event, and for the unshields
// that leave two or three pools at once the sum of the leaving balances: spec 41 A11 keeps those as Low confidence
// "multi-pool" events, which no crowd counts. #7 is the one deliberate difference: the explorer shows the net crossing
// 12_272_904 (transparent input 12_297_904 minus the fee 25_000), A11 counts what arrives in the pool: Ironwood takes
// 12_319_850 while Orchard pays out 46_946.
#[test]
fn the_twenty_events_of_research_44_decode_as_the_explorers_show() {
    use Confidence::{High as H, Low as L};
    use Kind::{Shield as S, Unshield as U};
    use Pool::{Ironwood as I, Mixed as M, Orchard as O, Sapling as Sa};
    let rows: [(&str, Kind, u64, Pool, Confidence); 20] = [
        ("91c68e50d342", U, 694_586_845, M, L),
        ("36dd4c96549a", U, 3_272_877_966, M, L),
        ("c9b08b67d300", U, 900_000, M, L),
        ("1eaff6c6de20", U, 850_000_000, M, L),
        ("368ff5b2a985", U, 120_014_670, M, L),
        ("c292feb1072f", U, 730_035_000, M, L),
        ("48ca0a26177c", S, 12_319_850, I, H),
        ("ad155fa7f196", U, 108_100_113, Sa, H),
        ("ce7e196a8d6a", U, 4_025_000, M, L),
        ("1463993eb90e", U, 315_000, O, H),
        ("2f453ce87ec3", U, 10_515_000, O, H),
        ("6c124059f5d1", U, 1_250_110_000, Sa, H),
        ("d86df60ef9e8", U, 2_244_399_410, Sa, H),
        ("7d67cf66a189", S, 1_700_065, Sa, H),
        ("214534c0148d", S, 60_260_000, Sa, H),
        ("4a80efbb3828", S, 1_120_000_000, Sa, H),
        ("2c43d7153a50", S, 12_500_000_000, Sa, H),
        ("446debbd5216", S, 500_553_249, Sa, H),
        ("2dcbf925e152", S, 875_526_006, Sa, H),
        ("4b48e369d6e4", S, 625_645_598, Sa, H),
    ];
    let (events, _) = sample();
    for (prefix, kind, zat, pool, conf) in rows {
        let hit: Vec<&Event> = events.iter().filter(|e| e.txid.starts_with(prefix)).collect();
        assert_eq!(hit.len(), 1, "{prefix}: expected exactly one event in the sample");
        let e = hit[0];
        assert_eq!((e.kind, e.zat, e.pool, e.confidence), (kind, zat, pool, conf), "{prefix}");
    }
}

// research/43 S1-1: with Ironwood missing from the data, `check --unshield` printed a false NO MATCH for the amount of a
// send-max round trip through an Ironwood-only shield (the reviewer's 40 shields of blocks 3528000..3528199, amount =
// shield - 15_000). Every such shield must now be found in its own pool, and `any` (the worst pool) must not say NO MATCH.
#[test]
fn a_send_max_round_trip_through_an_ironwood_shield_is_never_a_false_no_match() {
    let (events, now) = sample();
    let p = params(now);
    let shields: Vec<&Event> = events
        .iter()
        .filter(|e| {
            e.kind == Kind::Shield
                && e.pool == Pool::Ironwood
                && e.confidence == Confidence::High
                && !e.coinbase
                && (3_528_000..=3_528_199).contains(&e.h)
        })
        .collect();
    assert!(shields.len() >= 100, "only {} Ironwood shields in blocks 3528000..3528199", shields.len());
    for s in shields {
        let x = s.zat - 15_000;
        let own = score(&events, Kind::Unshield, Pool::Ironwood, x, &p);
        assert!(own.k >= 1 && own.verdict != Verdict::NoMatch, "{}: {} zat in Ironwood: k={} {}", s.txid, x, own.k, own.verdict);
        assert!(own.matches.iter().any(|m| m.txid == s.txid) || own.k > own.matches.len(), "{}: its own shield is not a match", s.txid);
        let any = worst(&score_pools(&events, Kind::Unshield, x, &p)).verdict;
        assert_ne!(any, Verdict::NoMatch, "{}: --pool any says NO MATCH for {} zat", s.txid, x);
    }
}

// E15 on real data: a shielded coinbase (the first transaction of a block, miner reward to a shielded address) is an
// entry into its pool like any other shield.
#[test]
fn shielded_coinbase_entries_in_the_sample_are_counted() {
    let (events, _) = sample();
    let cb: Vec<&Event> = events.iter().filter(|e| e.coinbase).collect();
    assert!(!cb.is_empty(), "no shielded coinbase in the sample");
    for e in cb.iter().take(50) {
        assert_eq!((e.kind, e.confidence), (Kind::Shield, Confidence::High), "{}", e.txid);
        let (k, _) = count(&events, Kind::Shield, e.pool, e.zat, &params(e.t));
        assert!(k >= 1, "{}: its own coinbase entry is not counted", e.txid);
    }
}
