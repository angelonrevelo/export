//! Content-addressed dedup. The key is SHA-256 of the file bytes.
//!
//! A non-cryptographic hash is refused: a collision would forge "already
//! ingested". The digest is computed once, before any container parse.

use crate::sha256;
use crate::sniff::MediaKind;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prior {
    pub kind: MediaKind,
    pub len: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Dedup {
    map: HashMap<[u8; 32], Prior>,
}

impl Dedup {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn get(&self, digest: &[u8; 32]) -> Option<&Prior> {
        self.map.get(digest)
    }

    pub fn insert(&mut self, digest: [u8; 32], prior: Prior) -> Option<Prior> {
        self.map.insert(digest, prior)
    }

    pub fn forget(&mut self, digest: &[u8; 32]) -> Option<Prior> {
        self.map.remove(digest)
    }
}

/// Hash `bytes` in one streaming pass over the slice.
pub fn digest_of(bytes: &[u8]) -> [u8; 32] {
    let mut s = sha256::Sha256::new();
    // Fixed chunk so the hot path is the streaming updater, not a special case.
    for chunk in bytes.chunks(64 * 1024) {
        s.update(chunk);
    }
    s.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_matches_oneshot() {
        let data = vec![7u8; 200_000];
        assert_eq!(digest_of(&data), sha256::hash(&data));
    }
}
