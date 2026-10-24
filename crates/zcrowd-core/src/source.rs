//! Where chain data comes from: a lightwalletd server (TLS verified) or committed fixtures.
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tonic::transport::{Channel, ClientTlsConfig};
use zcash_client_backend::proto::service::{
    BlockId, BlockRange, ChainSpec, Empty, PoolType, TxFilter, compact_tx_streamer_client::CompactTxStreamerClient,
};

/// The few public facts per transaction that zcrowd needs from a compact block.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TxSummary {
    /// Display-order txid (as block explorers show it).
    pub txid: String,
    pub n_vin: usize,
    pub n_vout: usize,
    pub shielded: bool,
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
    /// True for a server: every requested height must come back, linked by block hashes (spec 41 A9).
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
fn require_tls(url: &str) -> Result<()> {
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
    pub async fn connect(servers: &[String]) -> Result<Lwd> {
        let mut last = anyhow!("no servers");
        for url in servers {
            match Self::try_one(url).await {
                Ok(l) => return Ok(l),
                Err(e) => last = e.context(format!("server {url}")),
            }
        }
        Err(last)
    }

    async fn try_one(url: &str) -> Result<Lwd> {
        require_tls(url)?;
        let ep = Channel::from_shared(url.to_string())?
            .tls_config(ClientTlsConfig::new().with_webpki_roots())?
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60));
        let ch = ep.connect().await?;
        let mut client = CompactTxStreamerClient::new(ch);
        let info = client.get_lightd_info(Empty {}).await?.into_inner();
        if info.chain_name != "main" {
            bail!("server reports chain {:?}, expected main", info.chain_name);
        }
        Ok(Lwd { client, url: url.to_string() })
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
            pool_types: vec![PoolType::Transparent as i32, PoolType::Sapling as i32, PoolType::Orchard as i32],
        };
        let mut stream = retry("get_block_range", || {
            let mut c = self.client.clone();
            let r = range.clone();
            async move { c.get_block_range(r).await }
        })
        .await?
        .into_inner();
        let mut out = vec![];
        while let Some(b) = stream.message().await.context("block stream")? {
            let txs = b
                .vtx
                .iter()
                .map(|t| {
                    let mut id = t.txid.clone();
                    id.reverse();
                    TxSummary {
                        txid: hex::encode(id),
                        n_vin: t.vin.len(),
                        n_vout: t.vout.len(),
                        shielded: !(t.spends.is_empty() && t.outputs.is_empty() && t.actions.is_empty()),
                        coinbase: t.vin.len() == 1 && t.vin[0].prevout_txid.iter().all(|&x| x == 0),
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
