//! The library in five lines (the README shows the same). Run: cargo run -q -p zcrowd-core --example five_lines
use zcrowd_core::Kind;
use zcrowd_core::crowd::{DEFAULT_FEE_ALLOWANCE, Params, score};
use zcrowd_core::model::{parse_zec, read_coverage, read_ndjson};

fn main() -> anyhow::Result<()> {
    let data = std::path::Path::new("fixtures/flows-sample.ndjson");
    let events = read_ndjson(data)?;
    let now = read_coverage(data).unwrap().last_t; // the data's newest block time
    let p = Params { lookback_days: 30, tol_zat: 0, fee_allowance_zat: DEFAULT_FEE_ALLOWANCE, now };
    let c = score(&events, Kind::Unshield, parse_zec("1.2497")?, &p);
    println!("crowd {} -> {}", c.k, c.verdict);
    Ok(())
}
