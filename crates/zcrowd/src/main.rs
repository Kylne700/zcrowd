//! zcrowd: before you unshield, see how many earlier shields your amount could be matched to.
mod check;
mod report;
mod sandbox;
mod sync;
mod verify;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use zcrowd_core::crowd::DEFAULT_FEE_ALLOWANCE;
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
        /// Parallel transaction fetches (at most 8).
        #[arg(long, default_value_t = 8)]
        concurrency: usize,
    },
    /// How big a crowd does this amount hide in? Offline: cuts its own network access first.
    Check {
        /// Planned unshield amount in ZEC (what the recipient gets). Without a value (or `-`) it is read
        /// privately: no echo on a terminal, one line on stdin.
        #[arg(long, num_args = 0..=1, conflicts_with = "shield")]
        unshield: Option<Option<String>>,
        /// Planned shield amount in ZEC: how many other shields of about this amount exist.
        #[arg(long, num_args = 0..=1)]
        shield: Option<Option<String>>,
        /// Transparent address you plan to unshield to (t1, t3 or tex1), or `-` to type it privately.
        #[arg(long)]
        to: Option<String>,
        #[arg(long, default_value_t = 30)]
        lookback_days: u64,
        #[arg(long, default_value_t = 0)]
        tolerance_zat: u64,
        /// Extra room above the amount for the fee paid out of the shielded note.
        #[arg(long, default_value_t = DEFAULT_FEE_ALLOWANCE)]
        fee_allowance_zat: u64,
        #[arg(long, default_value = "data/flows.ndjson")]
        data: PathBuf,
        /// Machine-readable result (never includes the amount or address typed).
        #[arg(long)]
        json: bool,
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
    /// Test helper: try to connect to a TCP address, optionally after entering the sandbox.
    #[command(hide = true)]
    SandboxProbe {
        addr: std::net::SocketAddr,
        #[arg(long)]
        sandbox: bool,
    },
}

/// ZCROWD_OFFLINE=1 reads committed fixtures instead of a server.
pub enum Src {
    Net(Lwd),
    Fix(Fixtures),
}

async fn open_source(servers: Vec<String>) -> Result<Src> {
    if std::env::var("ZCROWD_OFFLINE").is_ok_and(|v| v == "1") {
        let dir = std::env::var("ZCROWD_FIXTURES").unwrap_or_else(|_| "fixtures".into());
        return Ok(Src::Fix(Fixtures::open(std::path::Path::new(&dir))?));
    }
    let list = if servers.is_empty() { DEFAULT_SERVERS.map(String::from).to_vec() } else { servers };
    Ok(Src::Net(Lwd::connect(&list).await?))
}

// Not #[tokio::main]: the sandbox must be entered before any thread exists.
fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Sync { days, server, out, candidates_only, concurrency } => {
            let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
            rt.block_on(async { sync::run(open_source(server).await?, days, &out, candidates_only, concurrency).await })
        }
        Cmd::Check { unshield, shield, to, lookback_days, tolerance_zat, fee_allowance_zat, data, json } => {
            sandbox::enter();
            check::run(check::Args { unshield, shield, to, lookback_days, tol_zat: tolerance_zat, fee_allowance_zat, data, json })
        }
        Cmd::Report { out, data } => {
            sandbox::enter();
            report::run(&out, &data)
        }
        Cmd::Verify { txid, data, raw_dir } => {
            sandbox::enter();
            verify::run(&txid, &data, raw_dir.as_deref())
        }
        Cmd::SandboxProbe { addr, sandbox } => {
            if sandbox {
                sandbox::enter();
            }
            match std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(2)) {
                Ok(_) => println!("connected"),
                Err(e) => println!("blocked: {e}"),
            }
            Ok(())
        }
    }
}
