use anyhow::{Result, bail};
use serde_json::json;
use std::collections::BTreeMap;
use std::io::{BufRead, IsTerminal};
use std::path::PathBuf;
use zcrowd_core::Kind;
use zcrowd_core::crowd::{AddrFinding, Crowd, DAY, Params, Verdict, address_check, score_pools, worse, worst};
use zcrowd_core::decode::{canonical_taddr, hides_crossing};
use zcrowd_core::model::{Event, coinbase_tag, fmt_utc, fmt_zec, load, parse_zec};
use zcrowd_core::suggest::popular;

/// Which shielded pool the funds are in (unshield) or will arrive in (shield). `any` scores all three and shows the worst.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum PoolArg {
    Any,
    Sapling,
    Orchard,
    Ironwood,
}

impl PoolArg {
    /// The crowd to show out of `score_pools`, which lists the pools in `Pool::SINGLE` order.
    fn pick(self, crowds: &[Crowd; 3]) -> &Crowd {
        match self {
            PoolArg::Any => worst(crowds),
            PoolArg::Sapling => &crowds[0],
            PoolArg::Orchard => &crowds[1],
            PoolArg::Ironwood => &crowds[2],
        }
    }
}

pub struct Args {
    pub unshield: Option<Option<String>>,
    pub shield: Option<Option<String>>,
    pub pool: PoolArg,
    pub to: Option<String>,
    pub lookback_days: u64,
    pub tol_zat: u64,
    pub fee_allowance_zat: u64,
    pub data: PathBuf,
    pub json: bool,
}

/// A value that should not sit in shell history: read from the terminal without echo, or one line of stdin.
fn secret_line(prompt: &str) -> Result<String> {
    if std::io::stdin().is_terminal() {
        return Ok(rpassword::prompt_password(prompt)?);
    }
    let mut s = String::new();
    std::io::stdin().lock().read_line(&mut s)?;
    Ok(s)
}

/// `--flag VALUE` uses VALUE (and warns); `--flag` or `--flag -` reads it privately.
fn take(v: Option<String>, prompt: &str) -> Result<String> {
    match v.as_deref() {
        None | Some("-") => secret_line(prompt),
        Some(x) => {
            eprintln!(
                "note: values given as arguments are visible in shell history and the process list; use the flag without a value (or `-`) to type it privately"
            );
            Ok(x.to_string())
        }
    }
}

const FOOTER: &str = "Covers amount + timing within one pool (+ address reuse with --to). Not covered: wallet/fee fingerprints, your IP when your wallet broadcasts (use Tor in your wallet), the z->z graph, links through pool moves beyond their amounts, off-chain records.";

pub fn run(a: Args) -> Result<()> {
    let (user, val) = match (a.unshield, a.shield) {
        (Some(v), None) => (Kind::Unshield, v),
        (None, Some(v)) => (Kind::Shield, v),
        _ => bail!("give exactly one of --unshield or --shield"),
    };
    if a.to.is_some() && user == Kind::Shield {
        bail!("--to applies to --unshield (the address you plan to unshield to)");
    }
    let prompt = match user {
        Kind::Shield => "Amount that arrives in the pool in ZEC, what you shield minus the fee (not shown): ",
        _ => "Amount in ZEC (not shown): ",
    };
    let zat = parse_zec(&take(val, prompt)?)?;
    let to = match a.to {
        Some(t) => Some(canonical_taddr(&take(Some(t).filter(|t| t != "-"), "Destination address (not shown): ")?)?),
        None => None,
    };
    let (events, meta) = load(&a.data)?;
    if events.is_empty() {
        bail!("{} has no events", a.data.display());
    }
    let cov = meta.cov;
    let now = cov.last_t;
    let p = Params { lookback_days: a.lookback_days, tol_zat: a.tol_zat, fee_allowance_zat: a.fee_allowance_zat, now };
    let crowds = score_pools(&events, user, zat, &p);
    let c = a.pool.pick(&crowds);
    let sugg = if matches!(c.verdict, Verdict::Green) { vec![] } else { popular(&events, user, c.pool, zat, &p, 5) };
    let addr = to.as_deref().map(|t| address_check(&events, t, now));
    let overall = addr.as_ref().map_or(c.verdict, |f| worse(c.verdict, addr_verdict(f)));
    // entries into the other pools: an observer reaches them only through a visible move between pools
    let elsewhere: usize = crowds.iter().filter(|o| o.pool != c.pool).map(|o| o.k).sum();

    // Lines about the data and the clock, printed after `data:`; --json carries the same lines under "notes".
    let wall = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let stale_secs = wall.saturating_sub(cov.last_t);
    let span = cov.last_t.saturating_sub(cov.first_t);
    let short_window = span < a.lookback_days.saturating_mul(DAY);
    let mut notes: Vec<String> = skipped_line(&meta.skipped).into_iter().collect();
    if stale_secs > STALE_SECS {
        notes.push(format!(
            "warning: the newest block in this data is {} old; shields since {} are not counted (run `zcrowd sync`)",
            fmt_age(stale_secs),
            fmt_utc(cov.last_t)
        ));
    }
    if short_window {
        notes.push(format!(
            "warning: the data covers {} but the lookback is {} days: a shield of about this amount before {} is not counted (sync more days)",
            fmt_span(span),
            a.lookback_days,
            fmt_utc(cov.first_t)
        ));
    }
    notes.extend(clock_warning(wall, cov.last_t));
    // The newest block is where the data ends: a verdict that rests on finding nothing (NO MATCH) or one entry (RED) can
    // still be overturned by shields after it, so these two always say so, however fresh the data is.
    let after_end = matches!(overall, Verdict::NoMatch | Verdict::Red)
        .then(|| format!("shields after {} (last synced block) are not counted", fmt_utc(cov.last_t)));

    if a.json {
        let v = |x: Verdict| x.to_string();
        // `verdict` is the overall one, exactly as in the text output (the worse of amount and address)
        let out = json!({
            "pool": c.pool.name(), "k": c.k, "weighted": c.weighted, "verdict": v(overall), "odd_amount": c.odd_amount,
            "pools": crowds.iter().map(|o| json!({"pool": o.pool.name(), "k": o.k, "weighted": o.weighted, "verdict": v(o.verdict)})).collect::<Vec<_>>(),
            "other_pools_k": elsewhere, "skipped": meta.skipped,
            "data": {"first_h": cov.first_h, "last_h": cov.last_h, "first_t": cov.first_t, "last_t": cov.last_t, "events": events.len()},
            "stale_secs": stale_secs, "short_window": short_window, "not_covered": FOOTER,
            "notes": notes.iter().chain(&after_end).collect::<Vec<_>>(),
            "amount_verdict": v(c.verdict), "address_verdict": addr.as_ref().map(|f| v(addr_verdict(f))),
            "matches": c.matches.iter().map(|e| &e.txid).collect::<Vec<_>>(),
            "suggestions": sugg.iter().map(|s| json!({"zat": s.zat, "k": s.k, "weighted": s.weighted, "verdict": v(s.verdict)})).collect::<Vec<_>>(),
        });
        println!("{out}");
        return Ok(());
    }

    let (what, other) = if user == Kind::Unshield {
        ("unshield (recipient gets this amount)", "earlier shields and moves into it of about this amount, allowing for the fee")
    } else {
        (
            "shield (amount arriving in the pool)",
            "other shields and moves into it of about this amount, the decoys for a later unshield of the whole note",
        )
    };
    // The amount typed is never printed back: it may sit in a terminal's scrollback or a CI log.
    println!(
        "check: {what}; pool {}; fee allowance {} zat, tolerance {} zat, lookback {} days",
        if a.pool == PoolArg::Any { "any (worst of sapling, orchard, ironwood)" } else { c.pool.name() },
        if user == Kind::Unshield { a.fee_allowance_zat } else { 0 },
        a.tol_zat,
        a.lookback_days
    );
    println!("data: blocks {}..{} ({} .. {}), {} events", cov.first_h, cov.last_h, fmt_utc(cov.first_t), fmt_utc(cov.last_t), events.len());
    for n in &notes {
        println!("{n}");
    }
    if a.pool == PoolArg::Any {
        for o in &crowds {
            println!("pool {}: k={} weighted={:.1} -> {}", o.pool.name(), o.k, o.weighted, o.verdict);
        }
    }
    // stable line, read by scripts and the web parity test: `crowd: k=<n> weighted=<w>`, explanation after it
    println!(
        "crowd: k={} weighted={:.1} (pool {}{}: {other}; entries older than 14 days count half)",
        c.k,
        c.weighted,
        c.pool.name(),
        if a.pool == PoolArg::Any { ", the worst of the three" } else { "" }
    );
    println!(
        "other pools: {elsewhere} entries of about this amount went into the other pools: linkable only through a visible pool move; not counted"
    );
    if !c.matches.is_empty() {
        println!("matches (newest first, up to 10):");
        for e in &c.matches {
            print_match(e, now);
        }
    }
    let why = match (user, c.verdict) {
        (Kind::Unshield, Verdict::NoMatch) => format!(
            "no shield of about this amount in the last {} days; amount matching cannot tie this unshield to a shield in that window",
            a.lookback_days
        ),
        (Kind::Unshield, Verdict::Red) => "one match: if it is yours you are linked; if not, observers may wrongly link you to it".into(),
        (Kind::Shield, Verdict::Red) => "your future unshield of this whole note would be unique".into(),
        (_, Verdict::Amber) => "small crowd".into(),
        (_, Verdict::Green) => "big crowd for amount + timing only, not \"private\"".into(),
        _ => String::new(),
    };
    match &addr {
        None => println!("verdict: {overall} - {why}"),
        Some(f) => {
            println!("verdict: {overall} (worst of amount {} and address {})", c.verdict, addr_verdict(f));
            println!("amount: {} - {why}", c.verdict);
        }
    }
    if let Some(l) = &after_end {
        println!("{l}");
    }
    if a.pool == PoolArg::Any {
        println!("funds in another pool? use --pool for a precise answer");
    }
    if c.odd_amount {
        println!("note: the exact amount is easy to recognise later, e.g. at an exchange; round amounts blend in.");
    }
    match &addr {
        None if user == Kind::Unshield => {
            println!("Not checked: address reuse. Never unshield to an address you shielded from (add --to -).")
        }
        None => {}
        Some(AddrFinding::Clear) => println!(
            "address: clear - not seen as a shield source or unshield destination in this data (addresses used before the window are not known)"
        ),
        Some(AddrFinding::ShieldSource { t, .. }) => println!(
            "address: RED - this address shielded on {}; unshielding back to it links you directly, whatever the amount",
            fmt_utc(*t)
        ),
        Some(AddrFinding::UsedAsDestination { n, first_t, last_t }) => println!(
            "address: AMBER - it received {n} earlier unshield(s) ({} .. {}); these are linked to each other by this address",
            fmt_utc(*first_t),
            fmt_utc(*last_t)
        ),
    }
    if !sugg.is_empty() {
        println!("popular nearby amounts (within 10%, bigger crowd):");
        for s in &sugg {
            println!("  {} ZEC -> crowd {} (weighted {:.1}, {})", fmt_zec(s.zat), s.k, s.weighted, s.verdict);
        }
        println!(
            "  do not send several chunks at the same time or to the same address; do not unshield the exact amount you shielded; waiting helps."
        );
    } else if c.verdict != Verdict::Green {
        println!("popular nearby amounts: none within 10% with a bigger crowd");
    }
    println!("{FOOTER}");
    Ok(())
}

/// After a sync the newest block is minutes old; older than this gets the stale warning.
const STALE_SECS: u64 = 7200;
/// A clock more than this behind the newest block is wrong (that block is already on the chain).
const CLOCK_BEHIND_SECS: u64 = 3 * 3600;
/// A clock more than this ahead means old data or a wrong clock.
const CLOCK_AHEAD_SECS: u64 = 365 * DAY;

/// How old the data is, is judged by this computer's clock, so a clock far off makes every age here wrong.
fn clock_warning(wall: u64, last_t: u64) -> Option<String> {
    let (clock, newest) = (fmt_utc(wall), fmt_utc(last_t));
    if last_t > wall.saturating_add(CLOCK_BEHIND_SECS) {
        Some(format!(
            "warning: this computer's clock ({clock}) is more than 3 h behind the newest block in this data ({newest}): the clock is wrong, so how old the data is cannot be judged"
        ))
    } else if wall > last_t.saturating_add(CLOCK_AHEAD_SECS) {
        Some(format!(
            "warning: this computer's clock ({clock}) is more than a year after the newest block in this data ({newest}): the data is out of date (run `zcrowd sync`) or the clock is wrong"
        ))
    } else {
        None
    }
}

/// One line about the transactions the sync could not turn into events; nothing when the data does not record them.
fn skipped_line(skipped: &Option<BTreeMap<String, usize>>) -> Option<String> {
    let s = skipped.as_ref()?;
    let list = |hides: bool| {
        s.iter().filter(|(k, n)| **n > 0 && hides_crossing(k) == hides).map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", ")
    };
    let (hide, benign) = (list(true), list(false));
    Some(match (hide.is_empty(), benign.is_empty()) {
        (true, true) => "skipped: none".into(),
        (true, false) => format!("skipped: {benign} (not shields or unshields: nothing crossed the turnstile)"),
        (false, true) => format!("warning: skipped {hide}: these could hide a shield or unshield and are missing from the crowd"),
        (false, false) => format!(
            "warning: skipped {hide}: these could hide a shield or unshield and are missing from the crowd (also skipped, harmless: {benign})"
        ),
    })
}

fn print_match(e: &Event, now: u64) {
    println!("  {}  {} ago  {} ZEC  {}{}", fmt_utc(e.t), fmt_age(now - e.t), fmt_zec(e.zat), e.txid, coinbase_tag(e));
}

/// Like `fmt_age`, but days to two decimals: a window an hour short of a 30-day lookback must not read "30.0 days".
fn fmt_span(secs: u64) -> String {
    if secs >= 2 * DAY { format!("{:.2} days", secs as f64 / DAY as f64) } else { fmt_age(secs) }
}

fn fmt_age(secs: u64) -> String {
    if secs >= 2 * DAY { format!("{:.1} days", secs as f64 / DAY as f64) } else { format!("{:.1} h", secs as f64 / 3600.0) }
}

fn addr_verdict(f: &AddrFinding) -> Verdict {
    match f {
        AddrFinding::Clear => Verdict::NoMatch,
        AddrFinding::UsedAsDestination { .. } => Verdict::Amber,
        AddrFinding::ShieldSource { .. } => Verdict::Red,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_is_wrong_beyond_three_hours_behind_or_a_year_ahead_of_the_newest_block() {
        let last = 1_790_000_000;
        let w = |wall| clock_warning(wall, last).map(|m| m.contains("behind"));
        assert_eq!(w(last), None);
        assert_eq!(w(last - 3 * 3600), None, "exactly 3 h behind is still fine");
        assert_eq!(w(last - 3 * 3600 - 1), Some(true));
        assert_eq!(w(last + 365 * DAY), None);
        assert_eq!(w(last + 365 * DAY + 1), Some(false));
        // wild values neither panic nor wrap
        assert!(clock_warning(0, u64::MAX).is_some() && clock_warning(u64::MAX, 0).is_some());
        assert!(clock_warning(u64::MAX, u64::MAX).is_none());
    }

    #[test]
    fn a_span_an_hour_short_of_30_days_does_not_read_as_30_days() {
        assert_eq!(fmt_span(30 * DAY - 3600), "29.96 days");
        assert_eq!(fmt_span(DAY + 3600), "25.0 h");
    }
}
