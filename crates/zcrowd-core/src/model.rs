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

/// One public turnstile crossing. `zat` is the net amount that crossed the turnstile
/// (shield: value entering the shielded pools; unshield: value leaving them, fee included).
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
    /// Shielded coinbase (block reward shielded by the miner).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub coinbase: bool,
    /// Transparent input addresses of a shield (public chain data; unknown script shapes omitted).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub src: Vec<String>,
    /// Transparent output addresses of an unshield.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dst: Vec<String>,
}

/// Block range and time span a data file covers (written by `sync`, next to the events).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    pub first_h: u64,
    pub first_t: u64,
    pub last_h: u64,
    pub last_t: u64,
}

impl Coverage {
    pub fn path_for(data: &std::path::Path) -> std::path::PathBuf {
        data.with_extension("meta.json")
    }

    pub fn union(self, o: Coverage) -> Coverage {
        Coverage {
            first_h: self.first_h.min(o.first_h),
            first_t: self.first_t.min(o.first_t),
            last_h: self.last_h.max(o.last_h),
            last_t: self.last_t.max(o.last_t),
        }
    }

    pub fn overlaps(&self, o: &Coverage) -> bool {
        self.first_h <= o.last_h + 1 && o.first_h <= self.last_h + 1
    }
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian (Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

pub fn fmt_utc(t: u64) -> String {
    let (y, m, d) = civil_from_days((t / 86_400) as i64);
    let s = t % 86_400;
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", s / 3600, s % 3600 / 60)
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

pub fn read_coverage(data: &std::path::Path) -> Option<Coverage> {
    serde_json::from_slice(&std::fs::read(Coverage::path_for(data)).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dates() {
        assert_eq!(fmt_utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(fmt_utc(1_792_240_286), "2026-10-17 12:31 UTC");
        assert_eq!(fmt_utc(951_782_400), "2000-02-29 00:00 UTC");
    }

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
        // E9: strict grammar
        for bad in ["1e3", "+1", "1,5", "1 2", "١٢", "0.000000001", "21000000.00000001", "0.0", "-0.5"] {
            assert!(parse_zec(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_zec("1.").unwrap(), 100_000_000);
        assert_eq!(parse_zec(".5").unwrap(), 50_000_000);
        assert_eq!(parse_zec("21000000").unwrap(), 21_000_000 * 100_000_000);
        assert_eq!(sig_decimals(parse_zec("1.0001").unwrap()), 4);
        assert_eq!(sig_decimals(parse_zec("1.23456789").unwrap()), 8);
    }
}

