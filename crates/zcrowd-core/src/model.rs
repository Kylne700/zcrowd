use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Shield,
    Unshield,
    /// Value moves from one shielded pool into another with no transparent output.
    Move,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pool {
    Sapling,
    Orchard,
    /// The third shielded pool of v6 transactions (zcash_protocol 0.11 `ShieldedPool::Ironwood`).
    Ironwood,
    /// More than one shielded pool has a nonzero value balance.
    Mixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    High,
    Low,
}

impl Pool {
    /// The three real pools, in the order `check` prints them.
    pub const SINGLE: [Pool; 3] = [Pool::Sapling, Pool::Orchard, Pool::Ironwood];

    pub fn name(self) -> &'static str {
        match self {
            Pool::Sapling => "sapling",
            Pool::Orchard => "orchard",
            Pool::Ironwood => "ironwood",
            Pool::Mixed => "mixed",
        }
    }
}

/// One public turnstile crossing or pool move. `zat` is the value that arrives in `pool`
/// (shield and move) or that leaves it, fee included (unshield). `pool` is Mixed only for low-confidence
/// multi-pool transactions.
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

/// Version of the data files. 1 = written before the Ironwood pool (the meta has no `version`); 2 = all three pools.
/// `check` and `report` refuse any other version: older data would be scored without Ironwood and give a false NO MATCH.
pub const FORMAT_VERSION: u32 = 2;

/// What `sync` writes next to the events (`<data>.meta.json`): the window they cover and what was left out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub version: u32,
    #[serde(flatten)]
    pub cov: Coverage,
    /// Transactions that produced no event, by reason; `None` = not recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<BTreeMap<String, usize>>,
}

/// The meta next to `data`, or the reason it cannot be used. Nothing is guessed: a data file without a usable meta of
/// this version is refused, so a crash or an upgrade can never leave a window that claims more than the events hold.
pub fn read_meta(data: &Path) -> Result<Meta> {
    let p = Coverage::path_for(data);
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!("no coverage file next to the data (an interrupted sync?): run `zcrowd sync` again")
        }
        Err(e) => bail!("cannot read {}: {e}", p.display()),
    };
    match serde_json::from_slice::<Meta>(&bytes) {
        Ok(m) if m.version == FORMAT_VERSION => Ok(m),
        _ => bail!("data from another zcrowd version: run `zcrowd sync` again"),
    }
}

/// The events and the meta of a data file, for `check` and `report`. The meta is read first so that data of another
/// version is refused before any event is parsed.
pub fn load(data: &Path) -> Result<(Vec<Event>, Meta)> {
    if !data.exists() {
        bail!("cannot read {} (run `zcrowd sync` first; check and report never fetch anything)", data.display());
    }
    let meta = read_meta(data)?;
    let events = read_ndjson(data).with_context(|| format!("cannot read {}", data.display()))?;
    Ok((events, meta))
}

/// Write the events and their meta so that a crash never leaves a meta that claims more than the events hold: both go
/// to temp files that are synced, the old meta is removed, the events are renamed into place and the meta last. A crash
/// in between leaves events with no meta, which `read_meta` refuses.
pub fn save(data: &Path, events: &[Event], meta: &Meta) -> Result<()> {
    save_with(data, events, meta, || Ok(()))
}

/// `save`, with `between` run after the events are in place and before the meta is (tests fail it to simulate a crash).
pub fn save_with(data: &Path, events: &[Event], meta: &Meta, between: impl FnOnce() -> Result<()>) -> Result<()> {
    let (mpath, dtmp) = (Coverage::path_for(data), data.with_extension("tmp"));
    let mtmp = mpath.with_extension("tmp");
    let mut w = std::io::BufWriter::new(std::fs::File::create(&dtmp)?);
    for e in events {
        writeln!(w, "{}", serde_json::to_string(e)?)?;
    }
    w.into_inner()?.sync_all()?;
    let mut m = std::fs::File::create(&mtmp)?;
    m.write_all(&serde_json::to_vec(meta)?)?;
    m.sync_all()?;
    match std::fs::remove_file(&mpath) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    std::fs::rename(&dtmp, data)?;
    sync_dir(data);
    between()?;
    std::fs::rename(&mtmp, &mpath)?;
    sync_dir(data);
    Ok(())
}

/// Make a rename durable. shortcut: Unix only; on Windows a power cut right after a sync can still undo the last rename
/// (a killed process cannot), upgrade when Windows becomes a supported target.
fn sync_dir(file: &Path) {
    #[cfg(unix)]
    {
        let dir = file.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let _ = std::fs::File::open(dir).and_then(|d| d.sync_all());
    }
    #[cfg(not(unix))]
    let _ = file;
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
/// The errors never repeat the text: an amount typed privately must not come back out on stderr.
pub fn parse_zec(s: &str) -> Result<u64> {
    let s = s.trim();
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    let digits = |p: &str| p.chars().all(|c| c.is_ascii_digit());
    if int.is_empty() && frac.is_empty() || !digits(int) || !digits(frac) {
        bail!("not a plain non-negative decimal amount (digits and one dot, like 12.5)");
    }
    if frac.len() > 8 {
        bail!("more than 8 decimal places");
    }
    let i: u64 = if int.is_empty() { 0 } else { int.parse()? };
    let f: u64 = if frac.is_empty() { 0 } else { format!("{frac:0<8}").parse()? };
    let z = i.checked_mul(ZAT_PER_ZEC).and_then(|v| v.checked_add(f));
    match z {
        Some(z) if z > 0 && z <= 21_000_000 * ZAT_PER_ZEC => Ok(z),
        _ => bail!("amount must be > 0 and <= 21000000 ZEC"),
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
    text.lines().filter(|l| !l.trim().is_empty()).map(|l| Ok(serde_json::from_str(l)?)).collect()
}

pub fn coinbase_tag(e: &Event) -> String {
    if e.coinbase { " (coinbase)".into() } else { String::new() }
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

    /// An amount typed privately must not come back out in an error (stderr ends up in logs).
    #[test]
    fn parse_errors_never_repeat_the_text() {
        for (bad, frag) in
            [("1.2497x", "2497"), ("-3.14159", "14159"), ("1.123456789", "123456789"), ("98765432.1", "98765432"), ("abc", "abc")]
        {
            let e = parse_zec(bad).unwrap_err().to_string();
            assert!(!e.is_empty() && !e.contains(frag), "{e}");
        }
    }
}
