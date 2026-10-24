use zcrowd_core::decode::{Decoded, event_from_tx, input_address};
use zcrowd_core::{Confidence, Event, Kind, Pool};

fn raw(txid: &str) -> Vec<u8> {
    let p = format!("{}/../../fixtures/raw/{txid}.hex", env!("CARGO_MANIFEST_DIR"));
    hex::decode(std::fs::read_to_string(p).unwrap().trim()).unwrap()
}

fn ev(txid: &str, h: u64) -> Event {
    match event_from_tx(&raw(txid), h, 1).unwrap() {
        Decoded::Event(e) => e,
        Decoded::Skip(s) => panic!("skipped: {s:?}"),
    }
}

// Hand-checked on 2026-10-24 against the Blockchair public API (api.blockchair.com/zcash):
// 12 transparent inputs totalling 1501076756, fee 70000, shielded_value_delta 1501006756.
#[test]
fn decode_shield() {
    let e = ev("954c19b2c73e667170c28b8d3a94fd96582cba736a5ed7275a10fc0c3e64e149", 3520931);
    assert_eq!((e.kind, e.zat, e.pool, e.n_tin, e.n_tout), (Kind::Shield, 1_501_006_756, Pool::Sapling, 12, 0));
    assert_eq!(e.confidence, Confidence::High);
    assert!(!e.coinbase && e.dst.is_empty());
    // Blockchair lists the same single spending address for all 12 inputs.
    assert_eq!(e.src, ["t1MKn34KBa8Xh4g8qU8psibBXvURafphVn7"]);
}

// One output of 1753058272 and no inputs, fee 15000 (same API): the pool lost 1753058272 + 15000.
#[test]
fn decode_unshield() {
    let e = ev("486a00611eb3c9f1bcce0ec39c423b04787a2f121d544aeffde3c440ccb71a76", 3520943);
    assert_eq!((e.kind, e.zat, e.fee_zat), (Kind::Unshield, 1_753_073_272, Some(15000)));
    assert_eq!(e.dst, ["t1XQZdZMnzXBcL8yx2PR27dSNrqctgwLgux"]); // Blockchair output recipient
}

// Shield with transparent change (same API): inputs 344666796, change output 36651796, fee 15000,
// shielded_value_delta 308000000 = 344666796 - 36651796 - 15000. Must count as an exact shield (spec 41 A5).
#[test]
fn decode_shield_with_transparent_change() {
    let e = ev("a7600a4c8401c310c86e95b08c4bcccf1ecce717ab97a6cfecf1041043718c24", 3522616);
    assert_eq!((e.kind, e.zat, e.n_tin, e.n_tout, e.confidence), (Kind::Shield, 308_000_000, 2, 1, Confidence::High));
}

// A real Sapling -> Ironwood move with no transparent part: Sapling pays out 100_020_000, Ironwood takes 100_000_000,
// fee 20_000 (spec 41 A11, test E13). Decoded from the raw bytes only; no explorer was consulted for this one.
// The old decoder produced no event for a transaction without a transparent bundle.
#[test]
fn decode_move_into_ironwood() {
    let e = ev("e62e321fb4d933b1928402942981a7efbd388830875fc3d8734eedcd39e0da1f", 3520949);
    assert_eq!((e.kind, e.zat, e.pool, e.fee_zat, e.confidence), (Kind::Move, 100_000_000, Pool::Ironwood, Some(20_000), Confidence::High));
    assert!((e.n_tin, e.n_tout) == (0, 0) && e.src.is_empty() && e.dst.is_empty());
}

// E10: the four events that research/44 found wrong, and E14, decoded from the raw bytes (fixtures/raw).
// Per-pool balances and fees were read on 2026-10-24 between 14:35 and 14:48 UTC from two explorers: Cipherscan
// (api.mainnet.cipherscan.app/api/tx/<txid>, primary) and zcashexplorer.app. The old decoder saw Sapling and Orchard
// only, so it reported the Orchard share alone (or nothing) and a fee of 0.
fn low_multi_pool_unshield(txid: &str, h: u64, zat: u64, fee: u64) {
    let e = ev(txid, h);
    assert_eq!((e.kind, e.zat, e.pool, e.fee_zat, e.confidence), (Kind::Unshield, zat, Pool::Mixed, Some(fee), Confidence::Low));
    assert_eq!((e.n_tin, e.n_tout), (0, 1));
    assert_eq!(e.dst.len(), 1);
}

// #1: Sapling 6_700 + Orchard 5_000 + Ironwood 694_575_145 leave, one output, explorer fee 35_000.
#[test]
fn decode_three_pool_unshield_1() {
    low_multi_pool_unshield("91c68e50d342033e69241957b1ebafabcc56f4e0f3db0464e983852120754c12", 3511683, 694_586_845, 35_000);
}

// #6: Sapling 30_000 + Orchard 7_125_000 + Ironwood 722_880_000 leave, one output, explorer fee 35_000.
#[test]
fn decode_three_pool_unshield_6() {
    low_multi_pool_unshield("c292feb1072f3b78ef140f88945b561e5f4d105e5cbb4c9d9556643ce85aa4f6", 3521500, 730_035_000, 35_000);
}

// #9: Orchard 3_494_678 + Ironwood 530_322 leave, one output, explorer fee 25_000.
#[test]
fn decode_two_pool_unshield_9() {
    low_multi_pool_unshield("ce7e196a8d6ab7d855f8eac4d9d53a8c207f42e9f6ba8d1d66e7587e40eaa751", 3521263, 4_025_000, 25_000);
}

// E14 (spec 41 A11): Orchard 225_000 + Ironwood 134_000_000 leave, t_out 134_200_000, so the fee is 25_000. The old
// decoder reported an Orchard unshield of 225_000 with fee 0.
#[test]
fn decode_two_pool_unshield_e14() {
    low_multi_pool_unshield("a57f5cd4302647c2649dc182f2c9c98e5ff252a95f235ddf39ba49ecb22cb448", 3494573, 134_225_000, 25_000);
}

// #7: one transparent input of 12_297_904, no transparent output, explorer fee 25_000. Ironwood takes 12_319_850 and
// Orchard pays out 46_946. The explorer shows the net crossing (12_272_904 = 12_297_904 - 25_000). Spec 41 A11 counts
// the value that arrives in the pool: a High shield into Ironwood of 12_319_850, with no fee (the input is not in the
// transaction), which is what a later unshield from Ironwood would have to match.
#[test]
fn decode_shield_into_ironwood_while_orchard_pays_in_7() {
    let e = ev("48ca0a26177cd32bab994e720c26b8393f0c86cde10da2ab65ff944fb67b3799", 3516743);
    assert_eq!((e.kind, e.zat, e.pool, e.fee_zat, e.confidence), (Kind::Shield, 12_319_850, Pool::Ironwood, None, Confidence::High));
    assert_eq!((e.n_tin, e.n_tout), (1, 0));
    assert_eq!(e.src, ["t1R7MByh2iEMaaXXnJw7mjpWMADy6BMm1pQ"]);
}

#[test]
fn decode_input_address_shapes() {
    // P2PKH scriptSig: <71-byte sig starting 0x30> <33-byte compressed key>; the address must be a t1.
    let mut s = vec![71u8, 0x30];
    s.extend([0u8; 70]);
    s.push(33);
    s.extend([2u8; 33]);
    assert!(input_address(&s).unwrap().starts_with("t1"));
    // Not pushes only (OP_DUP): no address.
    assert_eq!(input_address(&[0x76, 0x76]), None);
    // Truncated push: no address, no panic.
    assert_eq!(input_address(&[0x4b, 1, 2]), None);
}

#[test]
fn decode_canonical_taddr() {
    use zcrowd_core::decode::canonical_taddr;
    let a = "t1XQZdZMnzXBcL8yx2PR27dSNrqctgwLgux";
    assert_eq!(canonical_taddr(a).unwrap(), a);
    assert_eq!(canonical_taddr(&format!("  {a}\n")).unwrap(), a);
    assert!(canonical_taddr("not an address").is_err());
    assert!(canonical_taddr("u1abc").is_err());
}

#[test]
fn decode_tex_maps_to_its_p2pkh() {
    use zcash_address::{ToAddress, ZcashAddress};
    use zcash_protocol::consensus::NetworkType;
    let tex = ZcashAddress::from_tex(NetworkType::Main, [7u8; 20]).to_string();
    assert!(tex.starts_with("tex1"));
    assert_eq!(zcrowd_core::decode::canonical_taddr(&tex).unwrap(), zcrowd_core::decode::p2pkh([7u8; 20]));
    let test = ZcashAddress::from_transparent_p2pkh(NetworkType::Test, [7u8; 20]).to_string();
    assert!(zcrowd_core::decode::canonical_taddr(&test).is_err());
}
