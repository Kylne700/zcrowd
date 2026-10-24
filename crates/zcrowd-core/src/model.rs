use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Shield,
    Unshield,
}

impl Kind {
    pub fn opposite(self) -> Kind {
        match self {
            Kind::Shield => Kind::Unshield,
            Kind::Unshield => Kind::Shield,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pool {
    Sapling,
    Orchard,
    Mixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    High,
    Low,
}

/// One public turnstile crossing. `zat` is the amount that crossed (see decode.rs).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub h: u64,
    pub t: u64,
    pub kind: Kind,
    pub zat: u64,
    pub pool: Pool,
    pub fee_zat: Option<u64>,
    pub n_tin: usize,
    pub n_tout: usize,
    pub txid: String,
    pub confidence: Confidence,
}

pub const ZAT_PER_ZEC: u64 = 100_000_000;

/// Parse a decimal ZEC amount ("12.5") to zatoshi. Rejects negatives, zero, junk and more than 8 decimals.
pub fn parse_zec(s: &str) -> Result<u64> {
    let s = s.trim();
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    let digits = |p: &str| p.chars().all(|c| c.is_ascii_digit());
    if int.is_empty() && frac.is_empty() || !digits(int) || !digits(frac) {
        bail!("not a plain non-negative decimal amount: {s:?}");
    }
    if frac.len() > 8 {
        bail!("more than 8 decimal places: {s:?}");
    }
    let i: u64 = if int.is_empty() { 0 } else { int.parse()? };
    let f: u64 = if frac.is_empty() { 0 } else { format!("{frac:0<8}").parse()? };
    let z = i.checked_mul(ZAT_PER_ZEC).and_then(|v| v.checked_add(f));
    match z {
        Some(z) if z > 0 && z <= 21_000_000 * ZAT_PER_ZEC => Ok(z),
        _ => bail!("amount must be > 0 and <= 21000000 ZEC: {s:?}"),
    }
}

pub fn fmt_zec(zat: u64) -> String {
    format!("{}.{:08}", zat / ZAT_PER_ZEC, zat % ZAT_PER_ZEC)
}

/// Number of significant decimal places (trailing zeros stripped), 0..=8.
pub fn sig_decimals(zat: u64) -> usize {
    let f = format!("{:08}", zat % ZAT_PER_ZEC);
    f.trim_end_matches('0').len()
}

pub fn read_ndjson(path: &std::path::Path) -> Result<Vec<Event>> {
    let text = std::fs::read_to_string(path)?;
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| Ok(serde_json::from_str(l)?))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse() {
        assert_eq!(parse_zec("1").unwrap(), 100_000_000);
        assert_eq!(parse_zec("12.5").unwrap(), 1_250_000_000);
        assert_eq!(parse_zec("0.00000001").unwrap(), 1);
        assert!(parse_zec("1.123456789").is_err());
        assert!(parse_zec("-1").is_err());
        assert!(parse_zec("0").is_err());
        assert!(parse_zec("abc").is_err());
        assert!(parse_zec("").is_err());
        assert!(parse_zec(".").is_err());
        assert_eq!(sig_decimals(parse_zec("1.0001").unwrap()), 4);
        assert_eq!(sig_decimals(parse_zec("1.23456789").unwrap()), 8);
    }
}
