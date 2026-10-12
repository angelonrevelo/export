//! Ingest order is fixed: SHA-256 once, then the dedup map, then the codec.
//! A second call with the same bytes hashes once and does not decode again.

use std::collections::HashMap;

use crate::error::Error;
use crate::gif::{self, Gif};
use crate::sha256::{sha256, Sha256};

const DEFAULT_BUDGET: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadGif {
    pub gif: Gif,
    pub digest: [u8; 32],
    pub dedup_hit: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrittenGif {
    pub bytes: Vec<u8>,
    pub digest: [u8; 32],
    pub dedup_hit: bool,
}

pub struct Cache {
    decoded: HashMap<[u8; 32], Gif>,
    encoded: HashMap<[u8; 32], Vec<u8>>,
    pub hash_passes: u64,
    pub decodes: u64,
    pub encodes: u64,
    pub hits: u64,
    held: usize,
    budget: usize,
}

impl Cache {
    pub fn new() -> Self {
        Self::with_budget(DEFAULT_BUDGET)
    }

    pub fn with_budget(budget: usize) -> Self {
        Self {
            decoded: HashMap::new(),
            encoded: HashMap::new(),
            hash_passes: 0,
            decodes: 0,
            encodes: 0,
            hits: 0,
            held: 0,
            budget,
        }
    }
}

impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}

/// Hash the file once, return a cached decode when the digest is known, and
/// only then run LZW.
pub fn read_gif(bytes: &[u8], cache: &mut Cache) -> Result<ReadGif, Error> {
    let digest = sha256(bytes);
    cache.hash_passes += 1;
    if let Some(gif) = cache.decoded.get(&digest) {
        cache.hits += 1;
        return Ok(ReadGif {
            gif: gif.clone(),
            digest,
            dedup_hit: true,
        });
    }
    let gif = gif::decode_gif(bytes)?;
    cache.decodes += 1;
    remember_decode(cache, digest, &gif);
    Ok(ReadGif {
        gif,
        digest,
        dedup_hit: false,
    })
}

/// Hash the animation once, return a cached file when that digest is known,
/// and only then run LZW.
pub fn write_gif(gif: &Gif, cache: &mut Cache) -> Result<WrittenGif, Error> {
    let digest = digest_gif(gif);
    cache.hash_passes += 1;
    if let Some(bytes) = cache.encoded.get(&digest) {
        cache.hits += 1;
        return Ok(WrittenGif {
            bytes: bytes.clone(),
            digest,
            dedup_hit: true,
        });
    }
    let bytes = gif::encode_gif(gif)?;
    cache.encodes += 1;
    remember_encode(cache, digest, &bytes);
    Ok(WrittenGif {
        bytes,
        digest,
        dedup_hit: false,
    })
}

fn digest_gif(gif: &Gif) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(match gif.version {
        gif::Version::Gif87a => b"87",
        gif::Version::Gif89a => b"89",
    });
    h.update(&gif.width.to_le_bytes());
    h.update(&gif.height.to_le_bytes());
    h.update(&[gif.background, gif.aspect]);
    match &gif.global_table {
        Some(table) => {
            h.update(&[1]);
            for rgb in table {
                h.update(rgb);
            }
        }
        None => h.update(&[0]),
    }
    match gif.loop_count {
        Some(n) => {
            h.update(&[1]);
            h.update(&n.to_le_bytes());
        }
        None => h.update(&[0]),
    }
    for frame in &gif.frames {
        h.update(&frame.left.to_le_bytes());
        h.update(&frame.top.to_le_bytes());
        h.update(&frame.width.to_le_bytes());
        h.update(&frame.height.to_le_bytes());
        h.update(&frame.delay_cs.to_le_bytes());
        h.update(&[frame.disposal]);
        match frame.transparent {
            Some(t) => h.update(&[1, t]),
            None => h.update(&[0]),
        }
        match &frame.local_table {
            Some(table) => {
                h.update(&[1]);
                for rgb in table {
                    h.update(rgb);
                }
            }
            None => h.update(&[0]),
        }
        h.update(&frame.indices);
    }
    h.finish()
}

fn remember_decode(cache: &mut Cache, digest: [u8; 32], gif: &Gif) {
    let weight = gif
        .frames
        .iter()
        .map(|f| f.indices.len())
        .fold(0usize, usize::saturating_add);
    if weight > cache.budget {
        return;
    }
    if cache.held.saturating_add(weight) > cache.budget {
        cache.decoded.clear();
        cache.encoded.clear();
        cache.held = 0;
    }
    cache.held = cache.held.saturating_add(weight);
    cache.decoded.insert(digest, gif.clone());
}

fn remember_encode(cache: &mut Cache, digest: [u8; 32], bytes: &[u8]) {
    if bytes.len() > cache.budget {
        return;
    }
    if cache.held.saturating_add(bytes.len()) > cache.budget {
        cache.decoded.clear();
        cache.encoded.clear();
        cache.held = 0;
    }
    cache.held = cache.held.saturating_add(bytes.len());
    cache.encoded.insert(digest, bytes.to_vec());
}
