//! Cleartext fetch for Export URL jobs.
//!
//! The hot path is HTTP/1.1 over `std::net`: redirects, ranged reads, a 4 KB
//! coalesce gap, a byte-budget cache, and a stream to a writer. There is no
//! TLS stack, no browser impersonation, and no credential store. HTTPS URLs,
//! YouTube signature ciphers, and DRM are refused.

#![forbid(unsafe_code)]

mod assemble;
mod error;
#[cfg(test)]
mod harness;
mod http;
mod policy;
mod range;
mod select;
mod site;

pub use assemble::{assemble_coalesced, assemble_stream, AssembleStats};
pub use error::{Error, Result};
pub use http::{
    download_file, download_to, head, DownloadOutcome, GetCfg, USER_AGENT, READ_BUF,
};
pub use policy::{extension_of, is_blocked_ip, join_url, parse_url, refuse_ip, refuse_ssrf, Url};
pub use range::{
    apply_batch, coalesce_ranges, collapse_batch, cursor_plan, ByteCache, ByteCursor, ByteRange,
    BatchKind, BatchOp, RangeSpec, COALESCE_GAP, DEFAULT_CACHE_BUDGET,
};
pub use select::{has_audio, has_video, job_kind, select, JobKind, MediaFormat, Pick, Want};
pub use site::{
    fetch_job, sniff, DirectAdapter, GalleryAdapter, JobOutput, ResolvedMedia, SiteAdapter,
    WatchAdapter, YoutubeAdapter,
};
