//! zcrowd-core: extract public shield/unshield events and score how big a crowd an amount hides in.
#![allow(async_fn_in_trait)]
//! Heuristic: amount and timing only. Nothing here reads keys, addresses or wallets.
pub mod crowd;
pub mod decode;
pub mod ingest;
pub mod model;
pub mod source;
pub mod suggest;

pub use model::{Confidence, Event, Kind, Pool};
