use anyhow::Result;
use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;
use zcrowd_core::crowd::{DAY, DEFAULT_FEE_ALLOWANCE, Params, Verdict, bounds, score};
use zcrowd_core::model::{Confidence, Coverage, Event, coverage_of, fmt_utc, fmt_zec, read_ndjson};
use zcrowd_core::{Kind, Pool};

const BUCKETS: [(&str, u64, u64); 7] = [
    ("<0.01", 0, 1_000_000),
    ("0.01-0.1", 1_000_000, 10_000_000),
    ("0.1-1", 10_000_000, 100_000_000),
    ("1-10", 100_000_000, 1_000_000_000),
    ("10-100", 1_000_000_000, 10_000_000_000),
    ("100-1000", 10_000_000_000, 100_000_000_000),
    (">=1000", 100_000_000_000, u64::MAX),
];

pub fn run(out: &Path, data: &Path) -> Result<()> {
    let all = read_ndjson(data)?;
    let cov = coverage_of(data, &all).map(|c| c.0);
    let evs: Vec<Event> = all.into_iter().filter(|e| e.confidence == Confidence::High).collect();
    std::fs::write(out, render(&evs, cov))?;
    println!("wrote {} ({} events)", out.display(), evs.len());
    Ok(())
}

/// How the unshields of the last 14 days looked to an observer: each scored against the 14 days of entries (shields and
/// moves) into its own pool before it.
/// Counts in the order NO MATCH, RED, AMBER, GREEN.
fn profile(evs: &[Event], last_t: u64) -> [usize; 4] {
    // Entries per pool sorted by amount: an unshield then scores only the entries of about its own amount, so a month of
    // data takes seconds instead of minutes. Same result as scoring against every event.
    let mut by_pool: [Vec<Event>; 3] = Default::default();
    for e in evs.iter().filter(|e| matches!(e.kind, Kind::Shield | Kind::Move)) {
        if let Some(i) = Pool::SINGLE.iter().position(|p| *p == e.pool) {
            by_pool[i].push(e.clone());
        }
    }
    by_pool.iter_mut().for_each(|v| v.sort_by_key(|e| e.zat));
    let mut n = [0; 4];
    for e in evs.iter().filter(|e| e.kind == Kind::Unshield && e.t + 14 * DAY >= last_t) {
        let p = Params { lookback_days: 14, tol_zat: 0, fee_allowance_zat: DEFAULT_FEE_ALLOWANCE, now: e.t };
        // the recipient gets the pool amount minus the fee (the fee is unknown for a few: their pool amount is used)
        let x = e.zat.saturating_sub(e.fee_zat.unwrap_or(0)).max(1);
        let (lo, hi) = bounds(Kind::Unshield, x, &p);
        let all: &[Event] = Pool::SINGLE.iter().position(|q| *q == e.pool).map_or(&[], |i| &by_pool[i]);
        let near = &all[all.partition_point(|c| c.zat < lo)..all.partition_point(|c| c.zat <= hi)];
        n[match score(near, Kind::Unshield, e.pool, x, &p).verdict {
            Verdict::NoMatch => 0,
            Verdict::Red => 1,
            Verdict::Amber => 2,
            Verdict::Green => 3,
        }] += 1;
    }
    n
}

pub fn render(evs: &[Event], cov: Option<Coverage>) -> String {
    let window = cov.map_or(String::new(), |c| {
        format!(" Window: blocks {}-{} ({} to {}).", c.first_h, c.last_h, fmt_utc(c.first_t), fmt_utc(c.last_t))
    });
    // moves between shielded pools are neither shields nor unshields: counted in the headline, not in the tables
    let flows: Vec<&Event> = evs.iter().filter(|e| e.kind != Kind::Move).collect();
    let mut by_amt: HashMap<u64, (usize, usize)> = HashMap::new();
    for e in &flows {
        let c = by_amt.entry(e.zat).or_default();
        if e.kind == Kind::Shield { c.0 += 1 } else { c.1 += 1 }
    }
    let n = |k| evs.iter().filter(|e| e.kind == k).count();
    let uniq = flows.iter().filter(|e| by_amt[&e.zat] == (1, 0) || by_amt[&e.zat] == (0, 1)).count();
    let mut hist = vec![(0usize, 0usize); BUCKETS.len()];
    for e in &flows {
        let i = BUCKETS.iter().position(|b| e.zat >= b.1 && e.zat < b.2).unwrap_or(0);
        if e.kind == Kind::Shield { hist[i].0 += 1 } else { hist[i].1 += 1 }
    }
    let max = hist.iter().map(|h| h.0.max(h.1)).max().unwrap_or(1).max(1) as f64;
    let mut svg =
        String::from("<svg viewBox=\"0 0 560 220\" width=\"560\" height=\"220\" role=\"img\" aria-label=\"events per amount bucket\">");
    for (i, (b, h)) in BUCKETS.iter().zip(&hist).enumerate() {
        let x = 20 + i * 76;
        let name = b.0.replace('<', "&lt;").replace('>', "&gt;");
        let (hs, hu) = (h.0 as f64 / max * 150.0, h.1 as f64 / max * 150.0);
        let _ = write!(
            svg,
            "<rect x=\"{x}\" y=\"{:.0}\" width=\"30\" height=\"{hs:.0}\" fill=\"#2a6\"><title>{} shields {}</title></rect>\
             <rect x=\"{}\" y=\"{:.0}\" width=\"30\" height=\"{hu:.0}\" fill=\"#c52\"><title>{} unshields {}</title></rect>\
             <text x=\"{x}\" y=\"195\" font-size=\"11\">{}</text>",
            175.0 - hs,
            name,
            h.0,
            x + 32,
            175.0 - hu,
            name,
            h.1,
            name
        );
    }
    svg.push_str("<text x=\"20\" y=\"214\" font-size=\"11\">green = shields, red = unshields; amount in ZEC</text></svg>");
    let last_t = cov.map_or_else(|| evs.iter().map(|e| e.t).max().unwrap_or(0), |c| c.last_t);
    let first_t = cov.map_or_else(|| evs.iter().map(|e| e.t).min().unwrap_or(0), |c| c.first_t);
    let prof = profile(evs, last_t);
    let total = prof.iter().sum::<usize>().max(1);
    let mut prof_rows = String::new();
    for (name, c) in ["NO MATCH", "RED", "AMBER", "GREEN"].iter().zip(prof) {
        let _ = write!(prof_rows, "<tr><td>{name}</td><td>{c}</td><td>{:.1}%</td></tr>", 100.0 * c as f64 / total as f64);
    }
    let short = if last_t.saturating_sub(first_t) < 28 * DAY {
        "<p><b>This data covers less than 28 days, so the earlier shields of many unshields are missing and NO MATCH is overstated.</b></p>"
    } else {
        ""
    };
    let mut top: Vec<_> = by_amt.iter().collect();
    top.sort_by(|a, b| (b.1.0 + b.1.1).cmp(&(a.1.0 + a.1.1)).then(a.0.cmp(b.0)));
    let mut rows = String::new();
    for (z, (s, u)) in top.into_iter().take(15) {
        let _ = write!(rows, "<tr><td>{}</td><td>{s}</td><td>{u}</td></tr>", fmt_zec(*z));
    }
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>zcrowd report</title>\
<style>body{{font:15px/1.5 sans-serif;max-width:640px;margin:2em auto;padding:0 1em}}td,th{{padding:2px 12px;text-align:right}}</style></head><body>\
<h1>zcrowd report</h1>\
<p>{} events from public chain data: {} shields (t to z), {} unshields (z to t), {} moves between shielded pools.{window} Low-confidence multi-pool transactions excluded.</p>\
<p><b>{uniq}</b> events ({:.1}%) have an amount that no other event shares: these are the easiest to link.</p>\
<h2>Unshields of the last 14 days as an observer would score them</h2>\
<p>Each unshield (amount the recipient got) against the shields and moves into its own pool in the 14 days before it, fee allowance 50000 zat. RED: exactly one earlier entry of about that amount, so an observer has one candidate. GREEN: ten or more.</p>{short}\
<table><tr><th>verdict</th><th>unshields</th><th>share</th></tr>{prof_rows}</table>\
<h2>Events by amount</h2>{svg}\
<h2>Most common exact amounts</h2><table><tr><th>ZEC</th><th>shields</th><th>unshields</th></tr>{rows}</table>\
<p>Heuristic covering amount only. A large crowd is not a privacy guarantee.</p></body></html>\n",
        evs.len(),
        n(Kind::Shield),
        n(Kind::Unshield),
        n(Kind::Move),
        100.0 * uniq as f64 / flows.len().max(1) as f64
    )
}
