//! zcrowd: before you unshield, see how many earlier shields your amount could be matched to.
mod check;
mod report;
mod sandbox;
mod sync;
mod verify;

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use zcrowd_core::crowd::{DEFAULT_FEE_ALLOWANCE, MAX_LOOKBACK_DAYS, MAX_SLACK_ZAT};
use zcrowd_core::source::{DEFAULT_SERVERS, Fixtures, Lwd, require_tls};

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
        /// Days of history to fetch (1 to 3650). About 5% more blocks are fetched (at least 3 hours' worth), so that
        /// `check` with the same lookback does not find the window a hair short.
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=MAX_LOOKBACK_DAYS))]
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
        /// Send every request through Tor (the default; built in, nothing to install). If Tor fails, sync stops; it
        /// never falls back to a direct connection.
        #[arg(long)]
        tor: bool,
        /// Connect directly: the server sees your IP address and that you run zcrowd.
        #[arg(long, conflicts_with = "tor")]
        no_tor: bool,
        /// Where Tor keeps its state (public Tor directory data and guard state only; nothing about your
        /// queries). Default: $XDG_CACHE_HOME/zcrowd/tor, else ~/.cache/zcrowd/tor.
        #[arg(long, conflicts_with = "no_tor")]
        tor_dir: Option<PathBuf>,
        /// Last block to sync instead of the chain tip minus 10 (never later than that), for repeatable runs.
        #[arg(long)]
        end_height: Option<u64>,
    },
    /// How big a crowd does this amount hide in? Offline: cuts its own network access first.
    Check {
        /// Planned unshield amount in ZEC (what the recipient gets). Without a value (or `-`) it is read
        /// privately: no echo on a terminal, one line on stdin.
        #[arg(long, num_args = 0..=1, conflicts_with = "shield")]
        unshield: Option<Option<String>>,
        /// Planned shield amount in ZEC, the amount that arrives in the shielded pool (what you shield minus the fee):
        /// how many other shields of about this amount exist.
        #[arg(long, num_args = 0..=1)]
        shield: Option<Option<String>>,
        /// Shielded pool your funds are in (or will arrive in). `any` scores all three and shows the worst.
        #[arg(long, value_enum, default_value_t = check::PoolArg::Any)]
        pool: check::PoolArg,
        /// Address you plan to unshield to (t1, t3, tex1, or a Unified Address u1 whose transparent receiver is checked), or `-` to
        /// type it privately.
        #[arg(long)]
        to: Option<String>,
        /// How far back an earlier shield still counts as a match, in days (1 to 3650).
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=MAX_LOOKBACK_DAYS))]
        lookback_days: u64,
        /// Also match amounts this many zatoshi off, in both directions (0 to 100000000, which is 1 ZEC).
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u64).range(0..=MAX_SLACK_ZAT))]
        tolerance_zat: u64,
        /// Extra room above the amount for the fee paid out of the shielded note, in zatoshi (0 to 100000000, which is 1 ZEC).
        #[arg(long, default_value_t = DEFAULT_FEE_ALLOWANCE, value_parser = clap::value_parser!(u64).range(0..=MAX_SLACK_ZAT))]
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

/// $XDG_CACHE_HOME/zcrowd/tor, else ~/.cache/zcrowd/tor. Never a relative path: that could land inside a repo.
fn default_tor_dir() -> Result<PathBuf> {
    let abs = |k: &str| std::env::var_os(k).map(PathBuf::from).filter(|p| p.is_absolute());
    let base = abs("XDG_CACHE_HOME").or_else(|| abs("HOME").map(|h| h.join(".cache")));
    Ok(base.ok_or_else(|| anyhow!("no cache directory (XDG_CACHE_HOME and HOME are not set); pass --tor-dir"))?.join("zcrowd").join("tor"))
}

/// Tor is the default: only `--no-tor` connects directly.
async fn open_source(servers: Vec<String>, no_tor: bool, tor_dir: Option<PathBuf>) -> Result<Src> {
    if std::env::var("ZCROWD_OFFLINE").is_ok_and(|v| v == "1") {
        let dir = std::env::var("ZCROWD_FIXTURES").unwrap_or_else(|_| "fixtures".into());
        return Ok(Src::Fix(Fixtures::open(std::path::Path::new(&dir))?));
    }
    let list = if servers.is_empty() { DEFAULT_SERVERS.map(String::from).to_vec() } else { servers };
    list.iter().try_for_each(|u| require_tls(u))?; // a plain-text URL is refused before Tor or anything else starts
    if no_tor {
        eprintln!("warning: --no-tor: connecting directly. The server sees your IP address and that you run zcrowd.");
        return Ok(Src::Net(Lwd::connect(&list, None).await?));
    }
    let stop = |e: anyhow::Error| {
        anyhow!(
            "{e:#}\n\nTor was selected, so zcrowd did not contact the server directly and will not fall back to a direct connection. \
             To sync directly instead (the server then sees your IP address), run again with --no-tor."
        )
    };
    let dir = tor_dir.map_or_else(default_tor_dir, Ok).map_err(stop)?;
    Lwd::connect(&list, Some(&dir)).await.map(Src::Net).map_err(stop)
}

// Not #[tokio::main]: the sandbox must be entered before any thread exists.
fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Sync { days, server, out, candidates_only, concurrency, no_tor, tor_dir, end_height, .. } => {
            let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
            rt.block_on(async {
                sync::run(open_source(server, no_tor, tor_dir).await?, days, &out, candidates_only, concurrency, end_height).await
            })
        }
        Cmd::Check { unshield, shield, pool, to, lookback_days, tolerance_zat, fee_allowance_zat, data, json } => {
            sandbox::enter();
            check::run(check::Args { unshield, shield, pool, to, lookback_days, tol_zat: tolerance_zat, fee_allowance_zat, data, json })
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
