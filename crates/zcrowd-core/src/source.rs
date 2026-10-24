//! Where chain data comes from: a lightwalletd server (TLS verified) or committed fixtures.
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tonic::transport::{Channel, ClientTlsConfig, Uri};
use zcash_client_backend::proto::service::{
    BlockId, BlockRange, ChainSpec, Empty, PoolType, TxFilter, compact_tx_streamer_client::CompactTxStreamerClient,
};
use zcash_client_backend::tor;

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
}

pub trait BlockSource {
    async fn tip(&self) -> Result<u64>;
    /// Inclusive range.
    async fn blocks(&self, start: u64, end: u64) -> Result<Vec<BlockSummary>>;
    /// Raw transaction bytes and the height it was mined at.
    async fn raw_tx(&self, txid: &str) -> Result<(Vec<u8>, u64)>;
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

pub struct Lwd {
    client: CompactTxStreamerClient<Channel>,
    pub url: String,
    /// True when every byte to the server goes through Tor.
    pub tor: bool,
}

const STREAM_IDLE: Duration = Duration::from_secs(60);

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

async fn retry<T, F, Fut>(what: &str, mut f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, tonic::Status>>,
{
    let mut delay = Duration::from_millis(500);
    for attempt in 1..=5 {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) if attempt == 5 => return Err(anyhow!("{what}: {e}")),
            Err(_) => {
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
        }
    }
    unreachable!()
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
        let mut client = match tor {
            // The Tor branch has no direct connection in it; the channel reconnects through Tor only.
            Some(t) => t.connect_to_lightwalletd(url.parse::<Uri>()?, false).await?,
            None => {
                let ep = Channel::from_shared(url.to_string())?
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
        Ok(Lwd { client, url: url.to_string(), tor: tor.is_some() })
    }
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
        let range = BlockRange {
            start: Some(BlockId { height: start, hash: vec![] }),
            end: Some(BlockId { height: end, hash: vec![] }),
            // Every pool: an Ironwood-only transaction is invisible otherwise.
            pool_types: [PoolType::Transparent, PoolType::Sapling, PoolType::Orchard, PoolType::Ironwood].map(|p| p as i32).to_vec(),
        };
        let mut stream = retry("get_block_range", || {
            let mut c = self.client.clone();
            let r = range.clone();
            async move { c.get_block_range(r).await }
        })
        .await?
        .into_inner();
        let mut out = vec![];
        // tonic's request timeout ends at the response headers: a server that goes quiet mid-stream must fail, not hang.
        while let Some(b) = tokio::time::timeout(STREAM_IDLE, stream.message())
            .await
            .map_err(|_| anyhow!("block stream: no data for {} s", STREAM_IDLE.as_secs()))?
            .context("block stream")?
        {
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
                        // The compact `vin` leaves the coinbase input out; the coinbase is the block's first
                        // transaction (zcash_client_backend 0.25 proto/compact_formats.rs, `CompactTx.vin`).
                        coinbase: t.index == 0,
                    }
                })
                .collect();
            out.push(BlockSummary { h: b.height, t: b.time as u64, txs, hash: hex::encode(&b.hash), prev_hash: hex::encode(&b.prev_hash) });
        }
        Ok(out)
    }

    async fn raw_tx(&self, txid: &str) -> Result<(Vec<u8>, u64)> {
        let hash = txid_to_wire(txid)?;
        let r = retry("get_transaction", || {
            let mut c = self.client.clone();
            let f = TxFilter { block: None, index: 0, hash: hash.clone() };
            async move { c.get_transaction(f).await }
        })
        .await?
        .into_inner();
        Ok((r.data, r.height))
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
    fn strict_chain(&self) -> bool {
        self.inner.strict_chain()
    }
}

#[cfg(test)]
mod tests {
    use super::require_tls;

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
}
