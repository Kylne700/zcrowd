use anyhow::Result;
use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;
use zcrowd_core::Kind;
use zcrowd_core::model::{Confidence, Event, fmt_zec, read_ndjson};

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
    let evs: Vec<Event> = read_ndjson(data)?.into_iter().filter(|e| e.confidence == Confidence::High).collect();
    std::fs::write(out, render(&evs))?;
    println!("wrote {} ({} events)", out.display(), evs.len());
    Ok(())
}

pub fn render(evs: &[Event]) -> String {
    let mut by_amt: HashMap<u64, (usize, usize)> = HashMap::new();
    for e in evs {
        let c = by_amt.entry(e.zat).or_default();
        if e.kind == Kind::Shield { c.0 += 1 } else { c.1 += 1 }
    }
    let n = |k| evs.iter().filter(|e| e.kind == k).count();
    let unique = evs.iter().filter(|e| {
        let c = by_amt[&e.zat];
        c.0 + c.1 == 1
    });
    let uniq = unique.count();
    let mut hist = vec![(0usize, 0usize); BUCKETS.len()];
    for e in evs {
        let i = BUCKETS.iter().position(|b| e.zat >= b.1 && e.zat < b.2).unwrap_or(0);
        if e.kind == Kind::Shield { hist[i].0 += 1 } else { hist[i].1 += 1 }
    }
    let max = hist.iter().map(|h| h.0.max(h.1)).max().unwrap_or(1).max(1) as f64;
    let mut svg = String::from("<svg viewBox=\"0 0 560 220\" width=\"560\" height=\"220\" role=\"img\" aria-label=\"events per amount bucket\">");
    for (i, (b, h)) in BUCKETS.iter().zip(&hist).enumerate() {
        let x = 20 + i * 76;
        let (hs, hu) = (h.0 as f64 / max * 150.0, h.1 as f64 / max * 150.0);
        let _ = write!(
            svg,
            "<rect x=\"{x}\" y=\"{:.0}\" width=\"30\" height=\"{hs:.0}\" fill=\"#2a6\"><title>{} shields {}</title></rect>\
             <rect x=\"{}\" y=\"{:.0}\" width=\"30\" height=\"{hu:.0}\" fill=\"#c52\"><title>{} unshields {}</title></rect>\
             <text x=\"{x}\" y=\"195\" font-size=\"11\">{}</text>",
            175.0 - hs, b.0, h.0, x + 32, 175.0 - hu, b.0, h.1, b.0
        );
    }
    svg.push_str("<text x=\"20\" y=\"214\" font-size=\"11\">green = shields, red = unshields; amount in ZEC</text></svg>");
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
<p>{} events from public chain data: {} shields (t to z), {} unshields (z to t). Mixed transactions excluded.</p>\
<p><b>{uniq}</b> events ({:.1}%) have an amount that no other event shares: these are the easiest to link.</p>\
<h2>Events by amount</h2>{svg}\
<h2>Most common exact amounts</h2><table><tr><th>ZEC</th><th>shields</th><th>unshields</th></tr>{rows}</table>\
<p>Heuristic covering amount only. A large crowd is not a privacy guarantee.</p></body></html>\n",
        evs.len(),
        n(Kind::Shield),
        n(Kind::Unshield),
        100.0 * uniq as f64 / evs.len().max(1) as f64
    )
}
