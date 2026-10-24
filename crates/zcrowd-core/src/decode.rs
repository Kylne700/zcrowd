use crate::model::{Confidence, Event, Kind, Pool};
use anyhow::{Context, Result};
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use zcash_address::unified::{self, Container, Receiver};
use zcash_address::{ConversionError, ToAddress, TryFromAddress, ZcashAddress};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::{BlockHeight, BranchId, MainNetwork, NetworkType};
use zcash_transparent::address::TransparentAddress;

/// Why a transaction produced no event. Counted and printed by `sync`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Skip {
    /// A shielded transfer with no transparent part and no move between pools.
    NoTransparentPart,
    /// Transparent and shielded sides both present, but nothing crossed on balance.
    NoNetCrossing,
    /// t_in + value balances = t_out + fee with fee >= 0 does not close, or a value is out of range: the amounts
    /// cannot be trusted, so there is no event.
    ConservationFails,
    /// A Sprout JoinSplit moves value on the transparent side too; Sprout is not decoded.
    SproutValue,
    /// The server sent bytes whose transaction id is not the one asked for: dropped.
    TxidMismatch,
}

impl Skip {
    pub fn label(self) -> &'static str {
        match self {
            Skip::NoTransparentPart => "no_transparent_part",
            Skip::NoNetCrossing => "no_net_crossing",
            Skip::ConservationFails => "conservation_fails",
            Skip::SproutValue => "sprout_value",
            Skip::TxidMismatch => "txid_mismatch",
        }
    }
}

/// True for a skip label (as stored in the meta) whose transactions may have been a shield or an unshield:
/// their amounts could not be trusted or read. The two others crossed nothing. An unknown label counts as hiding.
pub fn hides_crossing(label: &str) -> bool {
    !matches!(label, "no_transparent_part" | "no_net_crossing")
}

pub enum Decoded {
    Event(Event),
    Skip(Skip),
}

pub fn p2pkh(hash: [u8; 20]) -> String {
    ZcashAddress::from_transparent_p2pkh(NetworkType::Main, hash).to_string()
}

pub fn p2sh(hash: [u8; 20]) -> String {
    ZcashAddress::from_transparent_p2sh(NetworkType::Main, hash).to_string()
}

fn hash160(b: &[u8]) -> [u8; 20] {
    Ripemd160::digest(Sha256::digest(b)).into()
}

/// Data pushes of a scriptSig. None if it contains anything but pushes.
fn pushes(s: &[u8]) -> Option<Vec<&[u8]>> {
    let (mut i, mut out) = (0, vec![]);
    while i < s.len() {
        let op = s[i];
        i += 1;
        let len = match op {
            0x00 => 0,
            0x01..=0x4b => op as usize,
            0x4c => {
                i += 1;
                *s.get(i - 1)? as usize
            }
            0x4d => {
                i += 2;
                u16::from_le_bytes([*s.get(i - 2)?, *s.get(i - 1)?]) as usize
            }
            _ => return None,
        };
        out.push(s.get(i..i + len)?);
        i += len;
    }
    Some(out)
}

/// Address that spent an input, read from its scriptSig. Heuristic, public data only:
/// `<sig> <pubkey>` is P2PKH; `... <redeem script>` with at least two pushes is P2SH. Anything else: None.
/// Not for a coinbase input: its scriptSig is free text and has no address (`event_from_tx` never asks).
pub fn input_address(script_sig: &[u8]) -> Option<String> {
    let p = pushes(script_sig)?;
    let last = *p.last()?;
    let is_pubkey = matches!((last.len(), last.first()), (33, Some(2 | 3)) | (65, Some(4)));
    if p.len() == 2 && is_pubkey && p[0].first() == Some(&0x30) {
        Some(p2pkh(hash160(last)))
    } else if p.len() >= 2 && !last.is_empty() && !is_pubkey {
        Some(p2sh(hash160(last)))
    } else {
        None
    }
}

fn parse(raw: &[u8], height: u64) -> Result<Transaction> {
    let branch = BranchId::for_height(&MainNetwork, BlockHeight::from_u32(u32::try_from(height).context("block height")?));
    Transaction::read(raw, branch).context("parse transaction")
}

/// The transaction id these bytes have (ZIP-244 digest for v5, double SHA-256 of the serialization for v4),
/// in explorer order. A server's answer is only believed when this equals the id that was asked for.
pub fn txid_of(raw: &[u8], height: u64) -> Result<String> {
    Ok(parse(raw, height)?.txid().to_string())
}

/// What the public numbers of one transaction say (all zatoshi); see `classify`.
#[derive(Debug, PartialEq, Eq)]
pub struct Class {
    pub kind: Kind,
    pub zat: u64,
    pub pool: Pool,
    /// Known only without transparent inputs: the prevout values are not in the transaction.
    pub fee: Option<u64>,
    /// More than one shielded pool is involved in a way that is not a plain shield, unshield or move.
    pub low: bool,
}

/// `vb` = value balances of Sapling, Orchard, Ironwood (positive: value leaves that pool, negative:
/// enters it). E = pools with a negative balance, L = pools with a positive one.
/// - Shield: one pool in E, L empty, transparent inputs (or coinbase). zat = value entering that pool.
/// - Unshield: E empty, one pool in L, transparent outputs. zat = value leaving it, fee included.
/// - Move: one pool in E, L not empty, no transparent output (a Shield instead when there are transparent inputs).
/// - Anything else that crossed or moved value: low confidence, pool Mixed, net amount.
///
/// Conservation t_in + sum(vb) = t_out + fee: with no transparent input t_in is 0, so the fee is what is left and a
/// negative one means the numbers are wrong: `ConservationFails`, never a clamped fee. All sums are 128-bit.
pub fn classify(vb: [i64; 3], n_tin: usize, n_tout: usize, t_out: u128) -> Result<Class, Skip> {
    let e: Vec<usize> = (0..3).filter(|&i| vb[i] < 0).collect();
    let l: Vec<usize> = (0..3).filter(|&i| vb[i] > 0).collect();
    if e.is_empty() && l.is_empty() {
        return Err(Skip::NoNetCrossing);
    }
    let entering: i128 = e.iter().map(|&i| -i128::from(vb[i])).sum();
    let leaving: i128 = l.iter().map(|&i| i128::from(vb[i])).sum();
    let (ti, to) = (n_tin > 0, n_tout > 0);
    let fee = i128::try_from(t_out).ok().map(|o| leaving - entering - o).ok_or(Skip::ConservationFails)?;
    if !ti && fee < 0 {
        return Err(Skip::ConservationFails);
    }
    let fee_zat = if ti { None } else { Some(u64::try_from(fee).map_err(|_| Skip::ConservationFails)?) };
    let (kind, amount, pool, low) = match (e.len(), l.len()) {
        (1, 0) if ti => (Kind::Shield, entering, Pool::SINGLE[e[0]], false),
        (0, 1) if to => (Kind::Unshield, leaving, Pool::SINGLE[l[0]], false),
        (1, 1..) if !to => (if ti { Kind::Shield } else { Kind::Move }, entering, Pool::SINGLE[e[0]], false),
        (0, _) if !to => return Err(Skip::NoNetCrossing), // shielded value left only as the fee
        _ if !ti && !to => (Kind::Move, entering, Pool::Mixed, true),
        _ if leaving > entering => (Kind::Unshield, leaving - entering, Pool::Mixed, true),
        _ => (Kind::Shield, entering - leaving, Pool::Mixed, true),
    };
    if amount == 0 {
        return Err(Skip::NoNetCrossing);
    }
    Ok(Class { kind, zat: u64::try_from(amount).map_err(|_| Skip::ConservationFails)?, pool, fee: fee_zat, low })
}

/// Turn a raw mainnet transaction into an event: a turnstile crossing or a move between shielded pools.
/// The three pool balances come from `Transaction::{sapling,orchard,ironwood}_bundle().value_balance()`; the Ironwood
/// bundle exists in v6 transactions only (zcash_primitives 0.31 src/transaction/mod.rs:661). `classify` does the rest.
/// Shielded coinbase is kept and flagged.
pub fn event_from_tx(raw: &[u8], height: u64, time: u64) -> Result<Decoded> {
    let tx = parse(raw, height)?;
    // A bundle that is absent moved nothing in its pool, so its balance is 0.
    let vb = [
        tx.sapling_bundle().map_or(0, |b| i64::from(*b.value_balance())),
        tx.orchard_bundle().map_or(0, |b| i64::from(*b.value_balance())),
        tx.ironwood_bundle().map_or(0, |b| i64::from(*b.value_balance())),
    ];
    // Sprout `value_balance`: positive = added to the transparent pool (zcash_primitives 0.31 sprout.rs). Not decoded.
    if tx.sprout_bundle().is_some_and(|b| b.value_balance().map(i64::from) != Some(0)) {
        return Ok(Decoded::Skip(Skip::SproutValue));
    }
    let tb = tx.transparent_bundle();
    let (n_tin, n_tout) = tb.map_or((0, 0), |b| (b.vin.len(), b.vout.len()));
    let t_out: u128 = tb.map_or(0, |b| b.vout.iter().map(|o| u128::from(u64::from(o.value()))).sum());
    let c = match classify(vb, n_tin, n_tout, t_out) {
        Err(Skip::NoNetCrossing) if tb.is_none() => return Ok(Decoded::Skip(Skip::NoTransparentPart)),
        Err(s) => return Ok(Decoded::Skip(s)),
        Ok(c) => c,
    };
    let addrs = |it: BTreeSet<String>| it.into_iter().collect::<Vec<_>>();
    let coinbase = tb.is_some_and(|b| b.is_coinbase());
    // A coinbase input spends nothing, so it has no address: its scriptSig is whatever text the miner wrote, and
    // `input_address` would read the last piece of that text as a P2SH redeem script and invent a t3 address.
    let src = if c.kind == Kind::Shield && !coinbase {
        #[allow(deprecated)]
        addrs(tb.iter().flat_map(|b| &b.vin).filter_map(|i| input_address(&i.script_sig().0.0)).collect())
    } else {
        vec![]
    };
    let dst = if c.kind == Kind::Unshield {
        addrs(
            tb.iter()
                .flat_map(|b| &b.vout)
                .filter_map(|o| match o.recipient_address()? {
                    TransparentAddress::PublicKeyHash(h) => Some(p2pkh(h)),
                    TransparentAddress::ScriptHash(h) => Some(p2sh(h)),
                })
                .collect(),
        )
    } else {
        vec![]
    };
    Ok(Decoded::Event(Event {
        h: height,
        t: time,
        kind: c.kind,
        zat: c.zat,
        pool: c.pool,
        fee_zat: c.fee,
        n_tin,
        n_tout,
        txid: tx.txid().to_string(),
        confidence: if c.low { Confidence::Low } else { Confidence::High },
        coinbase,
        src,
        dst,
    }))
}

/// How the address given to `--to` was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    /// t1..., t3... or tex1...
    Plain,
    /// A Unified Address with a transparent receiver; `shielded` says whether it also has a shielded one.
    Unified { shielded: bool },
}

/// The transparent address to check, in its canonical t1/t3 form, and how it was given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dest {
    pub taddr: String,
    pub via: Via,
}

pub const NO_TRANSPARENT_RECEIVER: &str =
    "this Unified Address has no transparent receiver: sending to it does not leave the shielded pools";

fn mainnet(net: NetworkType, a: [u8; 20], f: fn([u8; 20]) -> String) -> Result<Dest, ConversionError<&'static str>> {
    if net == NetworkType::Main { Ok(Dest { taddr: f(a), via: Via::Plain }) } else { Err(ConversionError::User("not a mainnet address")) }
}

impl TryFromAddress for Dest {
    type Error = &'static str;
    fn try_from_transparent_p2pkh(net: NetworkType, data: [u8; 20]) -> Result<Self, ConversionError<Self::Error>> {
        mainnet(net, data, p2pkh)
    }
    fn try_from_transparent_p2sh(net: NetworkType, data: [u8; 20]) -> Result<Self, ConversionError<Self::Error>> {
        mainnet(net, data, p2sh)
    }
    // ZIP-320 TEX addresses encode the same hash as a P2PKH address.
    fn try_from_tex(net: NetworkType, data: [u8; 20]) -> Result<Self, ConversionError<Self::Error>> {
        mainnet(net, data, p2pkh)
    }
    // ZIP-316: a Unified Address holds at most one transparent receiver (P2PKH or P2SH, never both). Without one, a
    // payment to it cannot leave the shielded pools.
    fn try_from_unified(net: NetworkType, ua: unified::Address) -> Result<Self, ConversionError<Self::Error>> {
        if net != NetworkType::Main {
            return Err(ConversionError::User("not a mainnet address"));
        }
        let items = ua.items();
        let shielded = items.iter().any(|r| matches!(r, Receiver::Orchard(_) | Receiver::Sapling(_)));
        items
            .iter()
            .find_map(|r| match r {
                Receiver::P2pkh(h) => Some(p2pkh(*h)),
                Receiver::P2sh(h) => Some(p2sh(*h)),
                _ => None,
            })
            .map(|taddr| Dest { taddr, via: Via::Unified { shielded } })
            .ok_or(ConversionError::User(NO_TRANSPARENT_RECEIVER))
    }
}

/// The transparent address behind what was typed after `--to`: a mainnet t1/t3/tex1 address (tex1 maps to its P2PKH hash,
/// in all-lowercase or all-uppercase as Bech32m allows), or the transparent receiver of a mainnet Unified Address.
/// Other shielded or non-mainnet addresses are rejected: zcrowd only compares transparent addresses.
pub fn destination(s: &str) -> Result<Dest> {
    let s = s.trim();
    // Base58 t1/t3 addresses are case-sensitive and stay so; only a TEX address may be written in capitals.
    let lower;
    let s = if s.starts_with("TEX1") && !s.bytes().any(|b| b.is_ascii_lowercase()) {
        lower = s.to_ascii_lowercase();
        &lower
    } else {
        s
    };
    let a = ZcashAddress::try_from_encoded(s).map_err(|_| anyhow::anyhow!("not a Zcash address"))?;
    a.convert::<Dest>().map_err(|e| match e {
        ConversionError::User(NO_TRANSPARENT_RECEIVER) => anyhow::anyhow!(NO_TRANSPARENT_RECEIVER),
        _ => anyhow::anyhow!(
            "need a mainnet transparent address (t1..., t3... or tex1...) or a Unified Address (u1...) with a transparent receiver"
        ),
    })
}

/// `destination` without the note on how it was written.
pub fn canonical_taddr(s: &str) -> Result<String> {
    destination(s).map(|d| d.taddr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(vb: [i64; 3], tin: usize, tout: usize, t_out: u128) -> Class {
        classify(vb, tin, tout, t_out).unwrap()
    }

    #[test]
    fn single_pool_shield_and_unshield() {
        let c = ok([-5_000_000, 0, 0], 1, 1, 100);
        assert_eq!((c.kind, c.zat, c.pool, c.fee, c.low), (Kind::Shield, 5_000_000, Pool::Sapling, None, false));
        // fee is inside the unshield amount: 1_000_000 leaves Ironwood, 985_000 reaches the address
        let c = ok([0, 0, 1_000_000], 0, 1, 985_000);
        assert_eq!((c.kind, c.zat, c.pool, c.fee, c.low), (Kind::Unshield, 1_000_000, Pool::Ironwood, Some(15_000), false));
    }

    // E10 (48ca0a26..., an explorer cross-check): Ironwood +12_319_850 in, Orchard 46_946 out, one transparent input, no output.
    // Per pool this is a shield into Ironwood of the value that arrived there, not the 12_272_904 net crossing.
    #[test]
    fn shield_into_one_pool_while_another_pays_in() {
        let c = ok([0, 46_946, -12_319_850], 1, 0, 0);
        assert_eq!((c.kind, c.zat, c.pool, c.fee, c.low), (Kind::Shield, 12_319_850, Pool::Ironwood, None, false));
    }

    // explorer cross-check (ce7e196a...): two pools pay out to a transparent output, so it is not a single-pool unshield.
    #[test]
    fn two_leaving_pools_are_low_confidence_unshield() {
        let c = ok([0, 3_494_678, 530_322], 0, 1, 4_000_000);
        assert_eq!((c.kind, c.zat, c.pool, c.fee, c.low), (Kind::Unshield, 4_025_000, Pool::Mixed, Some(25_000), true));
    }

    #[test]
    fn move_between_pools() {
        // Orchard pays out 30_000, Ironwood receives 5_000, 25_000 is the fee
        let c = ok([0, 30_000, -5_000], 0, 0, 0);
        assert_eq!((c.kind, c.zat, c.pool, c.fee, c.low), (Kind::Move, 5_000, Pool::Ironwood, Some(25_000), false));
        // the same with a transparent input is a shield into Ironwood
        let c = ok([0, 30_000, -5_000], 1, 0, 0);
        assert_eq!((c.kind, c.pool, c.fee), (Kind::Shield, Pool::Ironwood, None));
        // a transparent output makes it ambiguous
        let c = ok([0, 30_000, -5_000], 0, 1, 20_000);
        assert_eq!((c.kind, c.zat, c.pool, c.low), (Kind::Unshield, 25_000, Pool::Mixed, true));
        // two pools receive from one: still a move, but not a single-pool one
        let c = ok([-4_000, -6_000, 20_000], 0, 0, 0);
        assert_eq!((c.kind, c.zat, c.pool, c.fee, c.low), (Kind::Move, 10_000, Pool::Mixed, Some(10_000), true));
    }

    #[test]
    fn shielded_to_shielded_with_only_a_fee_is_no_event() {
        assert_eq!(classify([15_000, 0, 0], 0, 0, 0), Err(Skip::NoNetCrossing));
        assert_eq!(classify([0, 0, 0], 1, 1, 7), Err(Skip::NoNetCrossing));
        assert_eq!(classify([7_000, 8_000, 0], 0, 0, 0), Err(Skip::NoNetCrossing));
    }

    // Conservation: no transparent input means t_in = 0, so the fee t_out leaves is what is left and cannot be < 0.
    #[test]
    fn numbers_that_do_not_close_are_refused_not_clamped() {
        assert_eq!(classify([10_000, 0, 0], 0, 1, 20_000), Err(Skip::ConservationFails)); // pays out more than left
        assert_eq!(classify([-100, 0, 0], 0, 0, 0), Err(Skip::ConservationFails)); // value from nowhere
        assert_eq!(classify([0, 0, i64::MIN], 0, 1, 0), Err(Skip::ConservationFails));
        assert_eq!(classify([i64::MAX; 3], 0, 1, 0), Err(Skip::ConservationFails)); // sum does not fit a u64 fee
        assert_eq!(classify([0, 0, 1], 0, 1, u128::MAX), Err(Skip::ConservationFails));
        // with a transparent input the prevouts are unknown, so the fee is unknown and nothing is refused
        assert_eq!(ok([10_000, 0, 0], 1, 1, 20_000).fee, None);
    }
}
