use anyhow::{Result, bail};
use std::io::Read;
use std::path::Path;
use zcrowd_core::Kind;
use zcrowd_core::crowd::{DAY, Params, Verdict, score};
use zcrowd_core::model::{fmt_zec, parse_zec, read_ndjson};
use zcrowd_core::suggest::popular;

pub fn run(
    unshield: Option<Option<String>>,
    shield: Option<Option<String>>,
    amount_stdin: bool,
    lookback_days: u64,
    tol_zat: u64,
    data: &Path,
) -> Result<()> {
    let (kind, val) = match (unshield, shield) {
        (Some(v), None) => (Kind::Unshield, v),
        (None, Some(v)) => (Kind::Shield, v),
        _ => bail!("give exactly one of --unshield or --shield"),
    };
    let text = match (val, amount_stdin) {
        (Some(v), false) => v,
        (None, true) => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            s
        }
        _ => bail!("give the amount as a value, or use --amount-stdin without a value"),
    };
    let zat = parse_zec(&text)?;
    let events = read_ndjson(data).map_err(|e| anyhow::anyhow!("cannot read {} (run `zcrowd sync` first): {e}", data.display()))?;
    let Some(now) = events.iter().map(|e| e.t).max() else { bail!("{} has no events", data.display()) };
    let p = Params { lookback_days, tol_zat, now };
    let c = score(&events, kind, zat, &p);
    let (what, other) = match kind {
        Kind::Unshield => ("unshield", "earlier shields"),
        Kind::Shield => ("shield", "unshields"),
    };
    println!("amount: {} ZEC ({what}), tolerance {} zat, lookback {} days", fmt_zec(zat), tol_zat, lookback_days);
    println!("data: {} events, newest {} days old", events.len(), (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs().saturating_sub(now)) / DAY);
    println!("crowd: {} matching {other} (weighted {:.1}; events older than 14 days count half)", c.k, c.weighted);
    println!("verdict: {}", c.verdict);
    if c.odd_amount {
        println!("note: 5 or more significant decimals make this amount nearly unique (RED regardless of crowd).");
    }
    if c.verdict != Verdict::Green {
        let s = popular(&events, kind, zat, &p, 5);
        if s.is_empty() {
            println!("nearby: no amount within +-10% has a bigger crowd");
        }
        for (a, sc) in s {
            println!("nearby: {} ZEC -> crowd {} (weighted {:.1}, {})", fmt_zec(a), sc.k, sc.weighted, sc.verdict);
        }
    }
    println!("scope: amount and timing only. Not a privacy guarantee; GREEN means \"not obviously linkable by amount\".");
    Ok(())
}
