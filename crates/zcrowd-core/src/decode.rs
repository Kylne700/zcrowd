use crate::model::{Confidence, Event, Kind, Pool};
use anyhow::{Context, Result};
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use zcash_address::{ConversionError, ToAddress, TryFromAddress, ZcashAddress};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::{BlockHeight, BranchId, MainNetwork, NetworkType};
use zcash_transparent::address::TransparentAddress;

/// Why a transaction produced no event. Counted and printed by `sync`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Skip {
    NoTransparentPart,
    /// Transparent and shielded sides both present, but nothing crossed on balance.
    NoNetCrossing,
}

impl Skip {
    pub fn label(self) -> &'static str {
        match self {
            Skip::NoTransparentPart => "no_transparent_part",
            Skip::NoNetCrossing => "no_net_crossing",
        }
    }
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

/// Turn a raw mainnet transaction into a turnstile event.
///
/// net = -(sapling value_balance + orchard value_balance): positive means value entered the shielded pools.
/// - net > 0: shield of `net` (exact even with transparent change; fee not visible without prevout values).
/// - net < 0: unshield of `-net` (fee included). With no transparent inputs, fee_zat = -net - sum(outputs).
/// - Shielded coinbase is kept and flagged. Cross-pool transactions whose Sapling and Orchard balances
///   have opposite signs may hide a second crossing: kept as low confidence, excluded from crowds.
pub fn event_from_tx(raw: &[u8], height: u64, time: u64) -> Result<Decoded> {
    let branch = BranchId::for_height(&MainNetwork, BlockHeight::from_u32(height as u32));
    let tx = Transaction::read(raw, branch).context("parse transaction")?;
    let Some(tb) = tx.transparent_bundle() else { return Ok(Decoded::Skip(Skip::NoTransparentPart)) };
    let sap = tx.sapling_bundle().map(|b| i64::from(*b.value_balance())).unwrap_or(0);
    let orc = tx.orchard_bundle().map(|b| i64::from(*b.value_balance())).unwrap_or(0);
    let net = -(sap + orc);
    let pool = match (sap != 0, orc != 0) {
        (true, true) => Pool::Mixed,
        (false, true) => Pool::Orchard,
        _ => Pool::Sapling,
    };
    let (n_tin, n_tout) = (tb.vin.len(), tb.vout.len());
    let t_out: u64 = tb.vout.iter().map(|o| u64::from(o.value())).sum();
    let (kind, zat, fee_zat) = match net {
        0 => return Ok(Decoded::Skip(Skip::NoNetCrossing)),
        n if n > 0 => (Kind::Shield, n as u64, None),
        n => {
            let z = n.unsigned_abs();
            (Kind::Unshield, z, (n_tin == 0).then(|| z.saturating_sub(t_out)))
        }
    };
    let ambiguous = sap != 0 && orc != 0 && (sap > 0) != (orc > 0);
    let addrs = |it: BTreeSet<String>| it.into_iter().collect::<Vec<_>>();
    let src = if kind == Kind::Shield {
        #[allow(deprecated)]
        addrs(tb.vin.iter().filter_map(|i| input_address(&i.script_sig().0.0)).collect())
    } else {
        vec![]
    };
    let dst = if kind == Kind::Unshield {
        addrs(
            tb.vout
                .iter()
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
        kind,
        zat,
        pool,
        fee_zat,
        n_tin,
        n_tout,
        txid: tx.txid().to_string(),
        confidence: if ambiguous { Confidence::Low } else { Confidence::High },
        coinbase: tb.is_coinbase(),
        src,
        dst,
    }))
}

struct Taddr(String);

fn mainnet(net: NetworkType, a: [u8; 20], f: fn([u8; 20]) -> String) -> Result<Taddr, ConversionError<&'static str>> {
    if net == NetworkType::Main { Ok(Taddr(f(a))) } else { Err(ConversionError::User("not a mainnet address")) }
}

impl TryFromAddress for Taddr {
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
}

/// Canonical mainnet t1/t3 form of a transparent address (tex1 maps to its P2PKH hash). Shielded or
/// non-mainnet addresses are rejected: zcrowd only compares transparent addresses.
pub fn canonical_taddr(s: &str) -> Result<String> {
    let a = ZcashAddress::try_from_encoded(s.trim()).map_err(|_| anyhow::anyhow!("not a Zcash address"))?;
    a.convert::<Taddr>().map(|t| t.0).map_err(|_| anyhow::anyhow!("need a mainnet transparent address (t1..., t3... or tex1...)"))
}
