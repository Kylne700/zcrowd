//! zcrowd: before you unshield, see how many earlier shields your amount could be matched to.
mod check;
mod report;
mod sync;
mod verify;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use zcrowd_core::source::{DEFAULT_SERVERS, Fixtures, Lwd};

#[derive(Parser)]
#[command(name = "zcrowd", version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Crawl public chain data from one lightwalletd server and write shield/unshield events.
    Sync {
        #[arg(long, default_value_t = 30)]
        days: u64,
        /// lightwalletd URL (repeatable; first that answers as mainnet is used).
        #[arg(long)]
        server: Vec<String>,
        #[arg(long, default_value = "data/flows.ndjson")]
        out: PathBuf,
        /// Only count shield-like / unshield-like transactions; fetch no full transactions.
        #[arg(long)]
        candidates_only: bool,
        #[arg(long, default_value_t = 8)]
        concurrency: usize,
    },
    /// How big a crowd does this amount hide in? Runs on the local cache, no network.
    Check {
        /// Planned unshield amount in ZEC. Omit the value and use --amount-stdin to keep it out of shell history.
        #[arg(long, num_args = 0..=1, conflicts_with = "shield")]
        unshield: Option<Option<String>>,
        /// Planned shield amount in ZEC (mirror check).
        #[arg(long, num_args = 0..=1)]
        shield: Option<Option<String>>,
        #[arg(long)]
        amount_stdin: bool,
        #[arg(long, default_value_t = 30)]
        lookback_days: u64,
        #[arg(long, default_value_t = 0)]
        tolerance_zat: u64,
        #[arg(long, default_value = "data/flows.ndjson")]
        data: PathBuf,
    },
    /// Write a self-contained HTML report (inline CSS/SVG, no external requests).
    Report {
        #[arg(long, default_value = "report.html")]
        out: PathBuf,
        #[arg(long, default_value = "data/flows.ndjson")]
        data: PathBuf,
    },
    /// Recompute one event from the raw transaction cached by `sync` (offline).
    Verify {
        txid: String,
        #[arg(long, default_value = "data/flows.ndjson")]
        data: PathBuf,
        /// Directory of cached raw transactions (default: `tx/` next to the data file).
        #[arg(long)]
        raw_dir: Option<PathBuf>,
    },
}

/// ZCROWD_OFFLINE=1 reads committed fixtures instead of a server.
pub enum Src {
    Net(Lwd),
    Fix(Fixtures),
}

pub async fn open_source(servers: Vec<String>) -> Result<Src> {
    if std::env::var("ZCROWD_OFFLINE").is_ok_and(|v| v == "1") {
        let dir = std::env::var("ZCROWD_FIXTURES").unwrap_or_else(|_| "fixtures".into());
        return Ok(Src::Fix(Fixtures::open(std::path::Path::new(&dir))?));
    }
    let list = if servers.is_empty() { DEFAULT_SERVERS.map(String::from).to_vec() } else { servers };
    Ok(Src::Net(Lwd::connect(&list).await?))
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Sync { days, server, out, candidates_only, concurrency } => {
            sync::run(open_source(server).await?, days, &out, candidates_only, concurrency).await
        }
        Cmd::Check { unshield, shield, amount_stdin, lookback_days, tolerance_zat, data } => {
            check::run(unshield, shield, amount_stdin, lookback_days, tolerance_zat, &data)
        }
        Cmd::Report { out, data } => report::run(&out, &data),
        Cmd::Verify { txid, data, raw_dir } => verify::run(&txid, &data, raw_dir.as_deref()),
    }
}
