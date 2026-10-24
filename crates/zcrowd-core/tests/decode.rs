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
// shielded_value_delta 308000000 = 344666796 - 36651796 - 15000. Must count as an exact shield.
#[test]
fn decode_shield_with_transparent_change() {
    let e = ev("a7600a4c8401c310c86e95b08c4bcccf1ecce717ab97a6cfecf1041043718c24", 3522616);
    assert_eq!((e.kind, e.zat, e.n_tin, e.n_tout, e.confidence), (Kind::Shield, 308_000_000, 2, 1, Confidence::High));
}

// A real Sapling -> Ironwood move with no transparent part: Sapling pays out 100_020_000, Ironwood takes 100_000_000,
// fee 20_000 (test E13). Decoded from the raw bytes only; no explorer was consulted for this one.
// The old decoder produced no event for a transaction without a transparent bundle.
#[test]
fn decode_move_into_ironwood() {
    let e = ev("e62e321fb4d933b1928402942981a7efbd388830875fc3d8734eedcd39e0da1f", 3520949);
    assert_eq!((e.kind, e.zat, e.pool, e.fee_zat, e.confidence), (Kind::Move, 100_000_000, Pool::Ironwood, Some(20_000), Confidence::High));
    assert!((e.n_tin, e.n_tout) == (0, 0) && e.src.is_empty() && e.dst.is_empty());
}

// E10: the four events that an explorer cross-check found wrong, and E14, decoded from the raw bytes (fixtures/raw).
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

// E14: Orchard 225_000 + Ironwood 134_000_000 leave, t_out 134_200_000, so the fee is 25_000. The old
// decoder reported an Orchard unshield of 225_000 with fee 0.
#[test]
fn decode_two_pool_unshield_e14() {
    low_multi_pool_unshield("a57f5cd4302647c2649dc182f2c9c98e5ff252a95f235ddf39ba49ecb22cb448", 3494573, 134_225_000, 25_000);
}

// #7: one transparent input of 12_297_904, no transparent output, explorer fee 25_000. Ironwood takes 12_319_850 and
// Orchard pays out 46_946. The explorer shows the net crossing (12_272_904 = 12_297_904 - 25_000). The decoder counts
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

// ZIP 320 re-encodes a P2PKH address as Bech32m (BIP 350), which may be written in all capitals (QR codes); the Rust crate
// alone refuses such a string. Base58 t1/t3 addresses are case-sensitive and must stay so.
#[test]
fn decode_tex_in_capitals_is_accepted_but_base58_and_mixed_case_are_not() {
    use zcash_address::{ToAddress, ZcashAddress};
    use zcash_protocol::consensus::NetworkType;
    use zcrowd_core::decode::{canonical_taddr, p2pkh};
    let tex = ZcashAddress::from_tex(NetworkType::Main, [7u8; 20]).to_string();
    let want = p2pkh([7u8; 20]);
    assert_eq!(canonical_taddr(&tex.to_uppercase()).unwrap(), want);
    assert_eq!(canonical_taddr(&format!(" {}\n", tex.to_uppercase())).unwrap(), want);
    // mixed case is not Bech32m
    let mixed = format!("T{}", &tex[1..]);
    assert!(canonical_taddr(&mixed).is_err(), "{mixed}");
    let half: String = tex.chars().enumerate().map(|(i, c)| if i % 2 == 0 { c.to_ascii_uppercase() } else { c }).collect();
    assert!(canonical_taddr(&half).is_err(), "{half}");
    // a base58 address in capitals (or all lowercase) is another string and no address
    let t1 = "t1XQZdZMnzXBcL8yx2PR27dSNrqctgwLgux";
    assert!(canonical_taddr(&t1.to_uppercase()).is_err());
    assert!(canonical_taddr(&t1.to_lowercase()).is_err());
    assert!(canonical_taddr(&want.to_uppercase()).is_err());
}

fn unified(rev: unified::Revision, net: NetworkType, receivers: Vec<Receiver>) -> String {
    let ua = unified::Address::try_from_items(rev, receivers.into_iter().map(Uitem::Data).collect()).unwrap();
    ZcashAddress::from_unified(net, ua).to_string()
}

use zcash_address::unified::{self, Encoding, Receiver, Uitem};
use zcash_address::{ToAddress, ZcashAddress};
use zcash_protocol::consensus::NetworkType;

// ZIP 316: a Unified Address holds one transparent receiver at most (P2PKH or P2SH) and the wallet pays the most preferred
// receiver it supports (Orchard, Sapling, then transparent). `--to` checks the transparent receiver and says so.
#[test]
fn a_unified_address_is_checked_through_its_transparent_receiver() {
    use zcrowd_core::decode::{Dest, Via, destination, p2pkh, p2sh};
    let r0 = unified::Revision::R0;
    // transparent next to a shielded receiver: the shielded one is what wallets prefer
    for (t, want) in [(Receiver::P2pkh([7; 20]), p2pkh([7; 20])), (Receiver::P2sh([9; 20]), p2sh([9; 20]))] {
        for shielded in [Receiver::Sapling([1; 43]), Receiver::Orchard([2; 43])] {
            let ua = unified(r0, NetworkType::Main, vec![shielded, t.clone()]);
            assert!(ua.starts_with("u1"), "{ua}");
            assert_eq!(destination(&ua).unwrap(), Dest { taddr: want.clone(), via: Via::Unified { shielded: true } });
            assert_eq!(destination(&format!("  {ua}\n")).unwrap().taddr, want);
        }
    }
    // Revision 2 "tu" addresses may carry a transparent receiver and nothing else
    let ua = unified(unified::Revision::R2, NetworkType::Main, vec![Receiver::P2pkh([7; 20])]);
    assert!(ua.starts_with("tu1"), "{ua}");
    assert_eq!(destination(&ua).unwrap(), Dest { taddr: p2pkh([7; 20]), via: Via::Unified { shielded: false } });
    // plain addresses are plain
    assert_eq!(destination(&p2pkh([7; 20])).unwrap().via, Via::Plain);
}

#[test]
fn a_unified_address_without_a_transparent_receiver_says_it_does_not_leave_the_shielded_pools() {
    use zcrowd_core::decode::{NO_TRANSPARENT_RECEIVER, destination};
    assert_eq!(
        NO_TRANSPARENT_RECEIVER,
        "this Unified Address has no transparent receiver: sending to it does not leave the shielded pools"
    );
    let shielded_only = [
        vec![Receiver::Sapling([1; 43])],
        vec![Receiver::Orchard([2; 43])],
        vec![Receiver::Orchard([2; 43]), Receiver::Sapling([1; 43])],
        vec![Receiver::Orchard([2; 43]), Receiver::Unknown { typecode: 0x40, data: vec![5; 8] }],
    ];
    for receivers in shielded_only {
        for rev in [unified::Revision::R0, unified::Revision::R2] {
            let ua = unified(rev, NetworkType::Main, receivers.clone());
            let e = destination(&ua).unwrap_err().to_string();
            assert_eq!(e, NO_TRANSPARENT_RECEIVER, "{ua}");
            assert!(!e.contains(&ua));
        }
    }
    // a Unified Address of another network is no mainnet address, whatever it holds
    for net in [NetworkType::Test, NetworkType::Regtest] {
        let ua = unified(unified::Revision::R0, net, vec![Receiver::P2pkh([7; 20]), Receiver::Sapling([1; 43])]);
        let e = destination(&ua).unwrap_err().to_string();
        assert!(e.starts_with("need a mainnet transparent address"), "{e}");
    }
    // a damaged one is no address
    let ua = unified(unified::Revision::R0, NetworkType::Main, vec![Receiver::P2pkh([7; 20]), Receiver::Sapling([1; 43])]);
    let damaged = format!("{}q", &ua[..ua.len() - 1]);
    assert_eq!(destination(&damaged).unwrap_err().to_string(), "not a Zcash address");
}

fn skip(txid: &str, h: u64) -> zcrowd_core::decode::Skip {
    match event_from_tx(&raw(txid), h, 1).unwrap() {
        Decoded::Skip(s) => s,
        Decoded::Event(e) => panic!("expected a skip, got {e:?}"),
    }
}

// The first transaction of the false NO MATCH of the review (Ironwood-only shields): two transparent inputs, one transparent change
// output, and all the shielded value arrives in Ironwood (no Sapling or Orchard part at all). The old decoder saw no
// pool here. The amount comes from the balances; it was NOT compared with a public explorer.
#[test]
fn decode_shield_into_ironwood_only() {
    let e = ev("43bdcfaf262ba320576337dc63683bf4817ca6b03b0ff3a45ac156cbbd62e9c9", 3528000);
    assert_eq!((e.kind, e.zat, e.pool, e.confidence), (Kind::Shield, 27_967_119, Pool::Ironwood, Confidence::High));
    assert_eq!((e.n_tin, e.n_tout, e.coinbase), (2, 1, false));
}

// Ironwood-only unshield: no transparent input, one transparent output, fee 15000 (the pool paid output + fee).
#[test]
fn decode_unshield_from_ironwood_only() {
    let e = ev("f319fec5570c158b089d97b2a04efc3d4df7e4b7131c0a66acd1874a37295cd5", 3494299);
    assert_eq!((e.kind, e.zat, e.pool, e.fee_zat), (Kind::Unshield, 4_914_001, Pool::Ironwood, Some(15000)));
    assert_eq!((e.n_tin, e.n_tout), (0, 1));
}

// Coinbase paid to a shielded address (E15): a High shield into Ironwood, flagged as coinbase.
#[test]
fn decode_coinbase_shield_into_ironwood() {
    let e = ev("6262a45c8455ce5a5b148b1a48ca13c3e32dfc052c50ccda7fcbc8d5f26e2445", 3499538);
    assert_eq!((e.kind, e.zat, e.pool, e.confidence, e.coinbase), (Kind::Shield, 125_469_612, Pool::Ironwood, Confidence::High, true));
    assert!(e.src.is_empty());
}

// B1: a coinbase input has no address; its scriptSig is the miner's own text. The text of this block reward is two pushes
// and the last one is no public key, which the P2SH rule of `input_address` reads as a redeem script: the old decoder
// invented a t3 address (the hash of "Get Sluicey Yall sluicey.xyz"). An independent check on 2026-10-24 against two
// public explorers (Cipherscan, zcashexplorer.app) found no input address for this transaction: its only input is a
// `coinbase` field.
#[test]
fn a_coinbase_input_has_no_address_even_when_its_text_looks_like_a_redeem_script() {
    let e = ev("8db5109ff953c22f872e49ebbd2236be4aedc0402b812ede2b01d5c2fa707bf2", 3496848);
    assert_eq!((e.kind, e.zat, e.pool, e.confidence, e.coinbase, e.n_tin), (Kind::Shield, 125_010_000, Pool::Ironwood, Confidence::High, true, 1));
    assert!(e.src.is_empty(), "{:?}", e.src);
}

// Real transactions that are not events: a transparent input that only pays the fee next to an empty Ironwood bundle
// (nothing crossed), and a Sapling transfer with no transparent part (the fee leaves the pool, nobody gets paid).
#[test]
fn skip_transactions_that_cross_nothing() {
    use zcrowd_core::decode::Skip;
    assert_eq!(skip("b2374054f1fb8323202b6f0c42dbbb1f1c484fe1f51b7b7bddf1f35e2c362086", 3495000), Skip::NoNetCrossing);
    assert_eq!(skip("eababfcbf9ea4e57c2800e89bc61e0458dc17d3513562fd9f814fb800e7e7c4a", 3495000), Skip::NoTransparentPart);
}
