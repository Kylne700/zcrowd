//! Where chain data comes from: a lightwalletd server (TLS verified) or committed fixtures.
use anyhow::{Context, Result, anyhow, bail};
use futures::future::BoxFuture;
use futures::{FutureExt, Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tonic::transport::{Channel, ClientTlsConfig, Uri};
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_client_backend::proto::service::{
    BlockId, BlockRange, ChainSpec, Empty, PoolType, TxFilter, compact_tx_streamer_client::CompactTxStreamerClient,
};
use zcash_client_backend::tor;
use zcash_protocol::consensus::{BlockHeight, BranchId, MainNetwork};

/// The few public facts per transaction that zcrowd needs from a compact block.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TxSummary {
    /// Display-order txid (as block explorers show it).
    pub txid: String,
    pub n_vin: usize,
    pub n_vout: usize,
    /// Any Sapling, Orchard or Ironwood part.
    pub shielded: bool,
    /// Two or more of those pools are touched: a possible move between pools.
    #[serde(default)]
    pub multi_pool: bool,
    /// Which pools it touches, in the order Sapling, Orchard, Ironwood (absent in the committed fixtures: all false).
    #[serde(default)]
    pub pools: [bool; 3],
    pub coinbase: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BlockSummary {
    pub h: u64,
    pub t: u64,
    pub txs: Vec<TxSummary>,
    /// Block hash and previous block hash as the server states them (hex, one byte order for both).
    /// Empty in the committed fixtures, which are single blocks and not a chain.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prev_hash: String,
    /// Note commitments the block adds to the Sapling, Orchard and Ironwood trees (outputs, actions, Ironwood actions),
    /// counted from the compact block. Absent in the committed fixtures: all 0.
    #[serde(default)]
    pub commitments: [u32; 3],
}

pub trait BlockSource {
    async fn tip(&self) -> Result<u64>;
    /// Inclusive range.
    async fn blocks(&self, start: u64, end: u64) -> Result<Vec<BlockSummary>>;
    /// Raw transaction bytes and the height it was mined at.
    async fn raw_tx(&self, txid: &str) -> Result<(Vec<u8>, u64)>;
    /// Size of the note commitment tree of each shielded pool (Sapling, Orchard, Ironwood) after block `h`, as the server's
    /// own tree state says. None when the source has no tree state (fixtures) or the server does not serve it.
    async fn tree_sizes(&self, _h: u64) -> Result<Option<[u64; 3]>> {
        Ok(None)
    }
    /// True for a server: every requested height must come back, linked by block hashes.
    /// False for sparse fixtures.
    fn strict_chain(&self) -> bool {
        false
    }
}

pub fn txid_to_wire(display: &str) -> Result<Vec<u8>> {
    let mut b = hex::decode(display).context("txid is not hex")?;
    if b.len() != 32 {
        bail!("txid must be 32 bytes");
    }
    b.reverse();
    Ok(b)
}

pub const DEFAULT_SERVERS: [&str; 3] = ["https://zec.rocks:443", "https://na.zec.rocks:443", "https://eu.zec.rocks:443"];

/// tonic sends plain text for an `http://` URL even with a TLS config, so refuse that unless the server is this machine.
pub fn require_tls(url: &str) -> Result<()> {
    let uri: tonic::codegen::http::Uri = url.parse().with_context(|| format!("not a URL: {url}"))?;
    let host = uri.host().unwrap_or("").trim_matches(['[', ']']);
    let local = host == "localhost" || host.parse::<std::net::IpAddr>().is_ok_and(|a| a.is_loopback());
    if uri.scheme_str() != Some("https") && !local {
        bail!("refusing a plain-text connection to {url}: use https:// (http:// is only allowed for a lightwalletd on this machine)");
    }
    Ok(())
}

type LwdClient = CompactTxStreamerClient<Channel>;

pub struct Lwd {
    /// The first connection: the tip, the block streams and the tree states go over it (one server state to compare).
    client: LwdClient,
    /// Raw transactions are spread over these connections (the first one is among them).
    pool: Box<Pool<LwdClient>>, // boxed: a Lwd is a variant of an enum in main.rs
    pub url: String,
    /// True when every byte to the server goes through Tor.
    pub tor: bool,
}

/// Raw transactions are fetched over this many connections, each on a circuit of its own in Tor mode (one circuit set the
/// pace of a whole sync: measured 96 to 358 KB/s across runs of the same range, and the transactions of the second half of
/// a day are big enough, 200 KB, to be limited by it). In-flight requests are still limited to `--concurrency` in all.
const CONNECTIONS: usize = 4;
/// A connection older than this is replaced at its next use: S9b saw a circuit slow from 11 to 1.3 transactions a second
/// after 35 minutes, and a fresh process fast again.
const CONNECTION_MAX_AGE: Duration = Duration::from_secs(600);
/// After a connection could not be made, none is tried for this long.
const CONNECT_RETRY_AFTER: Duration = Duration::from_secs(30);

pub type Connect<C> = Box<dyn Fn() -> BoxFuture<'static, Result<C>> + Send + Sync>;

/// A few connections to the server. `get` gives the one with the fewest requests in flight: a slow connection holds its
/// requests longer and so gets fewer. A connection that was given up on (`Lease::give_up`) or is older than `max_age` is
/// replaced by a new one from `connect`, one at a time; requests already running on the old one finish there.
pub struct Pool<C> {
    slots: Mutex<Vec<Slot<C>>>,
    size: usize,
    max_age: Duration,
    connect: Connect<C>,
    /// Only one `get` at a time makes a connection; the others go on with the ones there are.
    connecting: AtomicBool,
    no_connect_before: Mutex<Instant>,
    turn: AtomicU64,
}

struct Slot<C> {
    conn: C,
    born: Instant,
    busy: Arc<AtomicUsize>,
    bad: Arc<AtomicBool>,
    /// When it was last given out, so that equally busy connections take turns.
    last_turn: u64,
}

impl<C> Slot<C> {
    fn new(conn: C, born: Instant) -> Self {
        Slot { conn, born, busy: Arc::default(), bad: Arc::default(), last_turn: 0 }
    }
}

/// One request's use of a connection; it counts as in flight until dropped.
pub struct Lease<C> {
    pub conn: C,
    busy: Arc<AtomicUsize>,
    bad: Arc<AtomicBool>,
}

impl<C> Lease<C> {
    /// The request failed in a way that says the connection is bad: no new request gets it.
    pub fn give_up(&self) {
        self.bad.store(true, Relaxed);
    }
}

impl<C> Drop for Lease<C> {
    fn drop(&mut self) {
        self.busy.fetch_sub(1, Relaxed);
    }
}

struct Unflag<'a>(&'a AtomicBool);

impl Drop for Unflag<'_> {
    fn drop(&mut self) {
        self.0.store(false, Relaxed);
    }
}

impl<C: Clone> Pool<C> {
    /// `first` is a connection already made (and checked). `size - 1` more are made at once; their ages are staggered so that
    /// they are not all replaced together. One that cannot be made is left out and made later by `get`.
    pub async fn start(size: usize, max_age: Duration, first: C, connect: Connect<C>) -> Pool<C> {
        let more = futures::future::join_all((1..size).map(|_| connect())).await;
        let now = Instant::now();
        let slots = std::iter::once(Ok(first))
            .chain(more)
            .enumerate()
            .filter_map(|(i, c)| c.ok().map(|c| Slot::new(c, now.checked_sub(max_age * i as u32 / size as u32).unwrap_or(now))))
            .collect();
        Pool {
            slots: Mutex::new(slots),
            size,
            max_age,
            connect,
            connecting: false.into(),
            no_connect_before: Mutex::new(now),
            turn: 1.into(),
        }
    }

    pub async fn get(&self) -> Result<Lease<C>> {
        // shortcut: the request that makes the connection waits for it (seconds over Tor), a background task would not hold
        // it up; upgrade if the timings show stalls at replacements.
        if !self.connecting.swap(true, Relaxed) {
            let _done = Unflag(&self.connecting);
            self.maintain().await?;
        }
        let mut slots = self.slots.lock().unwrap();
        let slot = slots
            .iter_mut()
            .filter(|x| !x.bad.load(Relaxed))
            .min_by_key(|x| (x.busy.load(Relaxed), x.last_turn))
            .ok_or_else(|| anyhow!("no connection to the server"))?;
        slot.last_turn = self.turn.fetch_add(1, Relaxed);
        slot.busy.fetch_add(1, Relaxed);
        Ok(Lease { conn: slot.conn.clone(), busy: slot.busy.clone(), bad: slot.bad.clone() })
    }

    /// Makes one connection if one is missing (or was given up on), or else replaces one that is too old. Fails only when
    /// there is no connection at all.
    async fn maintain(&self) -> Result<()> {
        // `old`: the connection to replace; `None` when one is missing.
        let (old, empty) = {
            let mut slots = self.slots.lock().unwrap();
            slots.retain(|x| !x.bad.load(Relaxed));
            let old = if slots.len() >= self.size {
                match slots.iter().find(|x| x.born.elapsed() > self.max_age) {
                    Some(x) => Some(x.busy.clone()),
                    None => return Ok(()),
                }
            } else {
                None
            };
            (old, slots.is_empty())
        };
        if !empty && Instant::now() < *self.no_connect_before.lock().unwrap() {
            return Ok(());
        }
        match (self.connect)().await {
            Ok(conn) => {
                let mut slots = self.slots.lock().unwrap();
                let new = Slot::new(conn, Instant::now());
                match old.and_then(|old| slots.iter().position(|x| Arc::ptr_eq(&x.busy, &old))) {
                    Some(i) => slots[i] = new,
                    None => slots.push(new),
                }
                Ok(())
            }
            Err(e) => {
                *self.no_connect_before.lock().unwrap() = Instant::now() + CONNECT_RETRY_AFTER;
                if empty { Err(e) } else { Ok(()) }
            }
        }
    }
}

/// `f` on a connection of the pool, retried like `retry` but each attempt on the connection `get` gives then: a request that
/// failed because of its connection gives that connection up, so the retry is made on another one.
pub async fn call_pooled<C, T, F, Fut>(pool: &Pool<C>, what: &str, f: F) -> Result<T>
where
    C: Clone,
    F: Fn(C) -> Fut,
    Fut: Future<Output = Result<T, tonic::Status>>,
{
    let f = &f;
    retry(what, || async move {
        let lease = pool.get().await.map_err(|e| tonic::Status::unavailable(format!("{e:#}")))?;
        let r = f(lease.conn.clone()).await;
        if r.as_ref().is_err_and(is_connection_error) {
            lease.give_up();
        }
        r
    })
    .await
}

/// Errors that say something about the connection and not about what was asked for: the ones the client made up (they carry
/// the error underneath; a server's own error, like "no such transaction", does not) and the codes meaning "try elsewhere".
fn is_connection_error(e: &tonic::Status) -> bool {
    use std::error::Error;
    use tonic::Code::{Cancelled, DeadlineExceeded, Unavailable};
    e.source().is_some() || matches!(e.code(), Unavailable | DeadlineExceeded | Cancelled)
}

/// A block stream that goes quiet this long counts as failed (tonic's request timeout ends at the response headers).
const STREAM_IDLE: Duration = Duration::from_secs(60);
const STREAM_BACKOFF: Duration = Duration::from_millis(500);
/// A block stream that fails is opened again after the last block received: this many failed attempts in a row (an
/// attempt that delivered a block starts the count again) or this many in all end the read. The in-all cap keeps a
/// server that sends one block per attempt from holding the sync for ever.
const STREAM_FAILS_IN_A_ROW: u32 = 5;
const STREAM_FAILS_IN_ALL: u32 = 20;

/// Blocks `start..=end` through `open(from)`, which starts a stream at height `from`. A stream that fails (an error or
/// `idle` without data) is opened again at the block after the last one received, after `backoff` that doubles with each
/// failure in a row, so a lost connection in the middle of a range costs the blocks not yet received and nothing more.
/// No block is dropped or repeated by this, and the caller still checks the heights and the hash chain over the whole
/// list, seams included. A stream that ends cleanly is not reopened: the caller judges a short answer.
pub async fn read_blocks<St, Fut>(
    start: u64,
    end: u64,
    idle: Duration,
    backoff: Duration,
    mut open: impl FnMut(u64) -> Fut,
) -> Result<Vec<BlockSummary>>
where
    Fut: Future<Output = Result<St, tonic::Status>>,
    St: Stream<Item = Result<BlockSummary, tonic::Status>> + Unpin,
{
    let mut out: Vec<BlockSummary> = vec![];
    let (mut in_a_row, mut in_all) = (0u32, 0u32);
    loop {
        let from = out.last().map_or(start, |b| b.h + 1);
        if from > end {
            return Ok(out);
        }
        let before = out.len();
        let failure = match open(from).await {
            Err(e) => e.to_string(),
            Ok(mut st) => loop {
                match tokio::time::timeout(idle, st.next()).await {
                    Ok(Some(Ok(b))) => out.push(b),
                    Ok(Some(Err(e))) => break e.to_string(),
                    Ok(None) => return Ok(out),
                    Err(_) => break format!("no data for {} s", idle.as_secs()),
                }
            },
        };
        in_a_row = if out.len() > before { 1 } else { in_a_row + 1 };
        in_all += 1;
        if in_a_row >= STREAM_FAILS_IN_A_ROW || in_all >= STREAM_FAILS_IN_ALL {
            bail!("get_block_range: {failure} (gave up at block {from} of {start}..{end} after {in_all} failed attempts)");
        }
        tokio::time::sleep(backoff * 2u32.pow(in_a_row - 1)).await;
    }
}

/// One compact block, reduced to what zcrowd needs.
fn summarize(b: &CompactBlock) -> BlockSummary {
    let txs = b
        .vtx
        .iter()
        .map(|t| {
            let mut id = t.txid.clone();
            id.reverse();
            let pools = [!(t.spends.is_empty() && t.outputs.is_empty()), !t.actions.is_empty(), !t.ironwood_actions.is_empty()];
            TxSummary {
                txid: hex::encode(id),
                n_vin: t.vin.len(),
                n_vout: t.vout.len(),
                shielded: pools.contains(&true),
                multi_pool: pools.iter().filter(|&&p| p).count() > 1,
                pools,
                // The compact `vin` leaves the coinbase input out; the coinbase is the block's first
                // transaction (zcash_client_backend 0.25 proto/compact_formats.rs, `CompactTx.vin`).
                coinbase: t.index == 0,
            }
        })
        .collect();
    let count = |f: fn(&zcash_client_backend::proto::compact_formats::CompactTx) -> usize| b.vtx.iter().map(f).sum::<usize>() as u32;
    let commitments = [count(|t| t.outputs.len()), count(|t| t.actions.len()), count(|t| t.ironwood_actions.len())];
    BlockSummary { h: b.height, t: b.time as u64, txs, hash: hex::encode(&b.hash), prev_hash: hex::encode(&b.prev_hash), commitments }
}

/// Longer than a cold bootstrap needs (about 30 s); a hang is not a loud failure. ZCROWD_TOR_BOOTSTRAP_SECS overrides it.
fn tor_bootstrap_timeout() -> Duration {
    Duration::from_secs(std::env::var("ZCROWD_TOR_BOOTSTRAP_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(120))
}

/// Bootstrap the built-in Tor client (arti). `dir` holds only public Tor directory data and guard state.
async fn tor_client(dir: &Path) -> Result<tor::Client> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir).with_context(|| format!("Tor directory {}", dir.display()))?;
    let limit = tor_bootstrap_timeout();
    tokio::time::timeout(limit, tor::Client::create(dir, |_| {}))
        .await
        .map_err(|_| anyhow!("Tor did not bootstrap within {} s", limit.as_secs()))?
        .context("Tor bootstrap")
}

/// Up to 5 attempts, 0.5 s apart at first and twice as far apart each time; "not implemented" is final at once.
async fn retry_status<T, F, Fut>(mut f: F) -> Result<T, tonic::Status>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, tonic::Status>>,
{
    let mut delay = Duration::from_millis(500);
    for attempt in 1..=5 {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if attempt == 5 || e.code() == tonic::Code::Unimplemented => return Err(e),
            Err(_) => {
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
        }
    }
    unreachable!()
}

async fn retry<T, F, Fut>(what: &str, f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, tonic::Status>>,
{
    retry_status(f).await.map_err(|e| anyhow!("{what}: {e}"))
}

/// The consensus branch id a server reports for its tip must be the one this build expects at that height (hex, with or
/// without `0x`). A server that does not know the network upgrade in force, or a zcrowd that does not, would leave pools
/// out or misread them. A server that leaves the field empty (or the height at 0) is not checked.
pub fn check_branch_id(reported: &str, tip_height: u64) -> Result<()> {
    let r = reported.trim();
    let r = r.strip_prefix("0x").or_else(|| r.strip_prefix("0X")).unwrap_or(r);
    if r.is_empty() || tip_height == 0 {
        return Ok(());
    }
    let h = u32::try_from(tip_height).context("server tip height")?;
    let want = u32::from(BranchId::for_height(&MainNetwork, BlockHeight::from_u32(h)));
    let got = u32::from_str_radix(r, 16).map_err(|_| anyhow!("server reports consensus branch id {reported:?}, which is not hex"))?;
    if got != want {
        bail!(
            "server reports consensus branch id {got:08x} at block {tip_height} but {want:08x} is expected there: outdated lightwalletd or outdated zcrowd (try another --server)"
        );
    }
    Ok(())
}

impl Lwd {
    /// Try servers in order; the first that answers as a mainnet lightwalletd is used.
    /// Only that one server is ever contacted afterwards.
    /// With `tor_dir`, every connection goes through Tor and there is no other path: a Tor failure is an
    /// error, never a direct connection.
    pub async fn connect(servers: &[String], tor_dir: Option<&Path>) -> Result<Lwd> {
        let tor = match tor_dir {
            Some(d) => Some(tor_client(d).await?),
            None => None,
        };
        let mut last = anyhow!("no servers");
        for url in servers {
            match Self::try_one(url, tor.as_ref()).await {
                Ok(l) => return Ok(l),
                Err(e) => last = e.context(format!("server {url}")),
            }
        }
        Err(last)
    }

    async fn try_one(url: &str, tor: Option<&tor::Client>) -> Result<Lwd> {
        require_tls(url)?;
        let uri = url.parse::<Uri>()?;
        // Every connection is made, and checked, the same way; in Tor mode each gets a circuit of its own, and nothing in
        // the closure can reach the server directly.
        let connect: Connect<LwdClient> = match tor {
            Some(t) => {
                let t = t.clone();
                Box::new(move || connect_checked(Via::Tor(t.isolated_client(), uri.clone())).boxed())
            }
            None => {
                let url = url.to_string();
                Box::new(move || connect_checked(Via::Direct(url.clone())).boxed())
            }
        };
        let client = connect().await?;
        let (size, max_age) = if tor.is_some() { (CONNECTIONS, CONNECTION_MAX_AGE) } else { (1, Duration::from_secs(86_400)) };
        let pool = Box::new(Pool::start(size, max_age, client.clone(), connect).await);
        Ok(Lwd { client, pool, url: url.to_string(), tor: tor.is_some() })
    }
}

enum Via {
    Tor(tor::Client, Uri),
    Direct(String),
}

/// A new connection to a mainnet lightwalletd that knows the network upgrade in force.
async fn connect_checked(via: Via) -> Result<LwdClient> {
    let mut client = match via {
        // The channel reconnects through Tor only.
        Via::Tor(t, uri) => t.connect_to_lightwalletd(uri, false).await?,
        Via::Direct(url) => {
            let ep = Channel::from_shared(url)?
                .tls_config(ClientTlsConfig::new().with_webpki_roots())?
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(60));
            CompactTxStreamerClient::new(ep.connect().await?)
        }
    };
    let info = client.get_lightd_info(Empty {}).await?.into_inner();
    if info.chain_name != "main" {
        bail!("server reports chain {:?}, expected main", info.chain_name);
    }
    check_branch_id(&info.consensus_branch_id, info.block_height)?;
    Ok(client)
}

impl BlockSource for Lwd {
    async fn tip(&self) -> Result<u64> {
        let r = retry("get_latest_block", || {
            let mut c = self.client.clone();
            async move { c.get_latest_block(ChainSpec {}).await }
        })
        .await?;
        Ok(r.into_inner().height)
    }

    async fn blocks(&self, start: u64, end: u64) -> Result<Vec<BlockSummary>> {
        read_blocks(start, end, STREAM_IDLE, STREAM_BACKOFF, |from| {
            let mut c = self.client.clone();
            let range = BlockRange {
                start: Some(BlockId { height: from, hash: vec![] }),
                end: Some(BlockId { height: end, hash: vec![] }),
                // Every pool: an Ironwood-only transaction is invisible otherwise.
                pool_types: [PoolType::Transparent, PoolType::Sapling, PoolType::Orchard, PoolType::Ironwood].map(|p| p as i32).to_vec(),
            };
            async move { Ok::<_, tonic::Status>(c.get_block_range(range).await?.into_inner().map(|b| b.map(|b| summarize(&b)))) }
        })
        .await
    }

    async fn raw_tx(&self, txid: &str) -> Result<(Vec<u8>, u64)> {
        let hash = txid_to_wire(txid)?;
        let r = call_pooled(&self.pool, "get_transaction", |mut c| {
            let f = TxFilter { block: None, index: 0, hash: hash.clone() };
            async move { c.get_transaction(f).await }
        })
        .await?
        .into_inner();
        Ok((r.data, r.height))
    }

    async fn tree_sizes(&self, h: u64) -> Result<Option<[u64; 3]>> {
        let ts = match retry_status(|| {
            let mut c = self.client.clone();
            async move { c.get_tree_state(BlockId { height: h, hash: vec![] }).await }
        })
        .await
        {
            Ok(r) => r.into_inner(),
            Err(e) if e.code() == tonic::Code::Unimplemented => return Ok(None),
            Err(e) => bail!("get_tree_state: {e}"),
        };
        if ts.height != h {
            bail!("get_tree_state: asked for block {h}, the server answered with block {}", ts.height);
        }
        let bad = |e: std::io::Error| anyhow!("get_tree_state: block {h}: {e}");
        Ok(Some([
            ts.sapling_tree().map_err(bad)?.size() as u64,
            ts.orchard_tree().map_err(bad)?.size() as u64,
            ts.ironwood_tree().map_err(bad)?.size() as u64,
        ]))
    }

    fn strict_chain(&self) -> bool {
        true
    }
}

/// Offline source: `blocks-*.json` (Vec<BlockSummary>) and `raw/<txid>.hex` files in a directory.
pub struct Fixtures {
    dir: PathBuf,
    blocks: Vec<BlockSummary>,
}

impl Fixtures {
    pub fn open(dir: &Path) -> Result<Fixtures> {
        let mut blocks: Vec<BlockSummary> = vec![];
        for e in std::fs::read_dir(dir).with_context(|| format!("fixtures dir {}", dir.display()))? {
            let p = e?.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with("blocks-") && name.ends_with(".json") {
                blocks.extend(serde_json::from_slice::<Vec<BlockSummary>>(&std::fs::read(&p)?)?);
            }
        }
        blocks.sort_by_key(|b| b.h);
        Ok(Fixtures { dir: dir.to_path_buf(), blocks })
    }
}

impl BlockSource for Fixtures {
    async fn tip(&self) -> Result<u64> {
        self.blocks.last().map(|b| b.h).ok_or_else(|| anyhow!("no fixture blocks"))
    }
    async fn blocks(&self, start: u64, end: u64) -> Result<Vec<BlockSummary>> {
        Ok(self.blocks.iter().filter(|b| (start..=end).contains(&b.h)).cloned().collect())
    }
    async fn raw_tx(&self, txid: &str) -> Result<(Vec<u8>, u64)> {
        // Test hook, offline source only: a slow server, so that a test can stop a sync half-way through its transactions.
        if let Some(ms) = std::env::var("ZCROWD_FIXTURES_DELAY_MS").ok().and_then(|v| v.parse().ok()) {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
        let h = self
            .blocks
            .iter()
            .find(|b| b.txs.iter().any(|t| t.txid == txid))
            .map(|b| b.h)
            .ok_or_else(|| anyhow!("txid not in fixture blocks"))?;
        let text = std::fs::read_to_string(self.dir.join("raw").join(format!("{txid}.hex")))?;
        Ok((hex::decode(text.trim())?, h))
    }
}

/// Test helper: wraps a source and logs every request it makes (kind and arguments).
pub struct Recording<S> {
    pub inner: S,
    pub log: std::sync::Mutex<Vec<String>>,
}

impl<S> Recording<S> {
    pub fn new(inner: S) -> Self {
        Recording { inner, log: Default::default() }
    }
    fn note(&self, s: String) {
        self.log.lock().unwrap().push(s);
    }
}

impl<S: BlockSource> BlockSource for Recording<S> {
    async fn tip(&self) -> Result<u64> {
        self.note("tip".into());
        self.inner.tip().await
    }
    async fn blocks(&self, start: u64, end: u64) -> Result<Vec<BlockSummary>> {
        self.note(format!("blocks {start}-{end}"));
        self.inner.blocks(start, end).await
    }
    async fn raw_tx(&self, txid: &str) -> Result<(Vec<u8>, u64)> {
        self.note(format!("tx {txid}"));
        self.inner.raw_tx(txid).await
    }
    async fn tree_sizes(&self, h: u64) -> Result<Option<[u64; 3]>> {
        self.note(format!("tree {h}"));
        self.inner.tree_sizes(h).await
    }
    fn strict_chain(&self) -> bool {
        self.inner.strict_chain()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_only_to_this_machine() {
        for ok in ["https://zec.rocks:443", "https://example.org", "http://127.0.0.1:9067", "http://localhost:9067", "http://[::1]:9067"] {
            assert!(require_tls(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://zec.rocks:443",
            "http://example.org",
            "http://127.0.0.1.example.org",
            "http://localhost.example.org",
            "http://10.0.0.5:9067",
        ] {
            assert!(require_tls(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_errors_of_the_connection_count_as_errors_of_the_connection() {
        let io = std::io::Error::other("connection reset by peer");
        for bad in [tonic::Status::from_error(Box::new(io)), tonic::Status::unavailable("x"), tonic::Status::cancelled("Timeout expired")] {
            assert!(is_connection_error(&bad), "{bad:?}");
        }
        // lightwalletd answers a transaction it does not have with a plain "unknown" error
        for server in [tonic::Status::unknown("no such transaction"), tonic::Status::not_found("x"), tonic::Status::unimplemented("x")] {
            assert!(!is_connection_error(&server), "{server:?}");
        }
    }

    fn id_at(h: u32) -> String {
        format!("{:08x}", u32::from(BranchId::for_height(&MainNetwork, BlockHeight::from_u32(h))))
    }

    #[test]
    fn the_branch_id_of_the_tip_must_be_the_one_expected_at_that_height() {
        for h in [1, 419_200, 1_687_104, 3_428_142, 3_428_143, 3_600_000] {
            assert!(check_branch_id(&id_at(h), u64::from(h)).is_ok(), "height {h}");
            assert!(check_branch_id(&format!("0x{}", id_at(h).to_uppercase()), u64::from(h)).is_ok(), "0x prefix, capitals, height {h}");
        }
        // Ironwood is in force from block 3428143: a server still on the id of the block before it is outdated
        assert_ne!(id_at(3_428_142), id_at(3_428_143));
        assert_eq!(id_at(3_428_143), "37a5165b");
        let e = check_branch_id(&id_at(3_428_142), 3_428_143).unwrap_err().to_string();
        assert!(e.contains("outdated lightwalletd or outdated zcrowd") && e.contains("37a5165b"), "{e}");
        assert!(check_branch_id("deadbeef", 3_600_000).is_err());
        assert!(check_branch_id("not hex", 3_600_000).is_err());
        // a field the server does not fill in, or a tip it does not give, is not checked
        assert!(check_branch_id("", 3_600_000).is_ok() && check_branch_id("  ", 3_600_000).is_ok());
        assert!(check_branch_id("deadbeef", 0).is_ok());
    }
}
