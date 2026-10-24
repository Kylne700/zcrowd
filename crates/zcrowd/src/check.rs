use anyhow::{Result, bail};
use serde_json::json;
use std::io::{BufRead, IsTerminal};
use std::path::PathBuf;
use zcrowd_core::Kind;
use zcrowd_core::crowd::{AddrFinding, DAY, Params, Verdict, address_check, score};
use zcrowd_core::decode::canonical_taddr;
use zcrowd_core::model::{Event, coinbase_tag, coverage_of, fmt_utc, fmt_zec, parse_zec, read_ndjson};
use zcrowd_core::suggest::popular;

pub struct Args {
    pub unshield: Option<Option<String>>,
    pub shield: Option<Option<String>>,
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

const FOOTER: &str = "Covers amount + timing (+ address reuse with --to). Not covered: wallet/fee fingerprints, your IP when your wallet broadcasts (use Tor in your wallet), z->z graph, cross-pool (Sapling<->Orchard) moves, off-chain records.";

pub fn run(a: Args) -> Result<()> {
    let (user, val) = match (a.unshield, a.shield) {
        (Some(v), None) => (Kind::Unshield, v),
        (None, Some(v)) => (Kind::Shield, v),
        _ => bail!("give exactly one of --unshield or --shield"),
    };
    if a.to.is_some() && user == Kind::Shield {
        bail!("--to applies to --unshield (the address you plan to unshield to)");
    }
    let zat = parse_zec(&take(val, "Amount in ZEC (not shown): ")?)?;
    let to = match a.to {
        Some(t) => Some(canonical_taddr(&take(Some(t).filter(|t| t != "-"), "Destination address (not shown): ")?)?),
        None => None,
    };
    let events = read_ndjson(&a.data)
        .map_err(|e| anyhow::anyhow!("cannot read {} (run `zcrowd sync` first; check never fetches anything): {e}", a.data.display()))?;
    if events.is_empty() {
        bail!("{} has no events", a.data.display());
    }
    let (cov, cov_derived) = coverage_of(&a.data, &events).expect("events is non-empty");
    let now = cov.last_t;
    let p = Params { lookback_days: a.lookback_days, tol_zat: a.tol_zat, fee_allowance_zat: a.fee_allowance_zat, now };
    let c = score(&events, user, zat, &p);
    let sugg = if matches!(c.verdict, Verdict::Green) { vec![] } else { popular(&events, user, zat, &p, 5) };
    let addr = to.as_deref().map(|t| address_check(&events, t, now));
    let overall = addr.as_ref().map_or(c.verdict, |f| worse(c.verdict, addr_verdict(f)));

    if a.json {
        let v = |x: Verdict| x.to_string();
        // `verdict` is the overall one, exactly as in the text output (the worse of amount and address)
        let out = json!({
            "k": c.k, "weighted": c.weighted, "verdict": v(overall), "odd_amount": c.odd_amount,
            "amount_verdict": v(c.verdict), "address_verdict": addr.as_ref().map(|f| v(addr_verdict(f))),
            "matches": c.matches.iter().map(|e| &e.txid).collect::<Vec<_>>(),
            "suggestions": sugg.iter().map(|s| json!({"zat": s.zat, "k": s.k, "weighted": s.weighted, "verdict": v(s.verdict)})).collect::<Vec<_>>(),
        });
        println!("{out}");
        return Ok(());
    }

    let (what, other) = match user {
        Kind::Unshield => ("unshield (recipient gets this amount)", "earlier shields of about this amount, allowing for the fee"),
        Kind::Shield | Kind::Move => ("shield", "other shields of about this amount, the decoys for a later unshield of the whole note"),
    };
    println!(
        "check: {what} {} ZEC; fee allowance {} zat, tolerance {} zat, lookback {} days",
        fmt_zec(zat),
        if user == Kind::Unshield { a.fee_allowance_zat } else { 0 },
        a.tol_zat,
        a.lookback_days
    );
    let wall = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    println!(
        "data: blocks {}..{} ({} .. {}), {} events{}",
        cov.first_h,
        cov.last_h,
        fmt_utc(cov.first_t),
        fmt_utc(cov.last_t),
        events.len(),
        if cov_derived { " (coverage file missing: window estimated from event times)" } else { "" }
    );
    if wall.saturating_sub(cov.last_t) > 7200 {
        println!(
            "warning: the newest block in this data is {} old; shields since {} are not counted (run `zcrowd sync`)",
            fmt_age(wall - cov.last_t),
            fmt_utc(cov.last_t)
        );
    }
    if cov.last_t.saturating_sub(cov.first_t) + 6 * 3600 < a.lookback_days * DAY {
        println!(
            "warning: the data covers {} but the lookback is {} days: a shield of about this amount before {} is not counted (sync more days)",
            fmt_age(cov.last_t - cov.first_t),
            a.lookback_days,
            fmt_utc(cov.first_t)
        );
    }
    // stable line (spec 41 A10): `crowd: k=<n> weighted=<w>`, explanation after it
    println!("crowd: k={} weighted={:.1} ({other}; shields older than 14 days count half)", c.k, c.weighted);
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

fn print_match(e: &Event, now: u64) {
    println!("  {}  {} ago  {} ZEC  {}{}", fmt_utc(e.t), fmt_age(now - e.t), fmt_zec(e.zat), e.txid, coinbase_tag(e));
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

fn worse(a: Verdict, b: Verdict) -> Verdict {
    let rank = |v| match v {
        Verdict::NoMatch => 0,
        Verdict::Green => 1,
        Verdict::Amber => 2,
        Verdict::Red => 3,
    };
    if rank(b) > rank(a) { b } else { a }
}
