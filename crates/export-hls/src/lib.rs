//! Cleartext HLS media playlists.
//!
//! This crate parses generated media playlists (`#EXTINF` segments, no
//! encryption). On hosts other than `wasm32-unknown-unknown` it can also fetch
//! them with cleartext HTTP/1.1 on `std::net`. It does not download YouTube
//! and it does not transcode. `https` URLs, `#EXT-X-KEY`, cookies,
//! credentials, and userinfo are unsupported.

#![forbid(unsafe_code)]

mod error;
mod index;
mod model;
mod parse;
mod policy;
mod urlcheck;

#[cfg(not(target_arch = "wasm32"))]
mod http;

pub use error::Error;
pub use index::{coalesce_byte_ranges, index_segments, COALESCE_GAP_BYTES};
pub use model::{ByteRange, IndexedSpan, Map, MediaPlaylist, PlaylistType, Segment};
pub use parse::parse_media_playlist;
pub use policy::ConnectPolicy;
pub use urlcheck::{check_media_uri, parse_http_url, HttpUrl};

#[cfg(not(target_arch = "wasm32"))]
pub use http::{
    check_hop, read_http_message, resolve_redirect, write_http_get, Fetcher, Hop, HttpMessage,
};
