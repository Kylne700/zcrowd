use zcrowd_core::decode::{Decoded, Skip, event_from_tx, input_address};
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

#[test]
fn decode_shielded_only_is_ignored() {
    let d = event_from_tx(&raw("e62e321fb4d933b1928402942981a7efbd388830875fc3d8734eedcd39e0da1f"), 3520949, 1).unwrap();
    assert!(matches!(d, Decoded::Skip(Skip::NoTransparentPart)));
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
