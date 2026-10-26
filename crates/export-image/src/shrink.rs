//! Measured JPEG shrink.
//!
//! There is no single quality. For each image the encoder is run at the
//! qualities a binary search visits, the file size of each finished encode is
//! recorded, and the highest quality that landed at or under the cap is kept.
//! If quality 1 is still over the cap the image is area-resized by 0.7 and the
//! search repeats, at most six times. A source already at or under the cap is
//! copied byte for byte.
//!
//! That first resize rounds each side and uses the standard Huffman tables.
//! When six of those scales still miss the cap, the search starts over from
//! the original pixels with tables counted from the image. Each side is then
//! truncated to 70 percent, and at least 2 pixels, which is how the Python
//! app scales. The file is still a baseline JPEG. The cap is not raised.

use std::collections::HashMap;

use crate::error::{Error, INVALID, OVER_BUDGET, UNSUPPORTED};
use crate::exif;
use crate::jpeg::{decode_jpeg, encode_jpeg_limited, encode_jpeg_optimized_limited, Limited};
use crate::png::{decode_png, png_header};
use crate::raster::Raster;
use crate::resize::{resize, Filter};
use crate::sha256::Sha256;
use crate::sniff::{sniff, Format};
use crate::SHRINK_QUALITY_MAX;

const MAX_SCALES: u32 = 6;
const DEFAULT_CACHE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    pub quality: u8,
    pub width: u32,
    pub height: u32,
    /// `Some` when the encode finished. `None` when it was stopped because the
    /// file had already passed the cap. A stopped encode has no measured size.
    pub bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShrinkOutcome {
    pub bytes: Vec<u8>,
    pub digest: [u8; 32],
    pub dedup_hit: bool,
    pub copied: bool,
    pub quality: Option<u8>,
    pub probes: Vec<Probe>,
    pub width: u32,
    pub height: u32,
    pub scales: u32,
}

struct Slot {
    target: u64,
    outcome: ShrinkOutcome,
}

/// Remembers one encode per content hash so a repeat does not encode again.
pub struct ShrinkCache {
    slots: HashMap<[u8; 32], Slot>,
    pub encodes: u64,
    pub hash_passes: u64,
    pub hits: u64,
    held: usize,
    budget: usize,
}

impl ShrinkCache {
    pub fn new() -> Self {
        Self::with_budget(DEFAULT_CACHE_BYTES)
    }

    pub fn with_budget(budget: usize) -> Self {
        Self {
            slots: HashMap::new(),
            encodes: 0,
            hash_passes: 0,
            hits: 0,
            held: 0,
            budget,
        }
    }
}

impl Default for ShrinkCache {
    fn default() -> Self {
        Self::new()
    }
}

pub fn shrink_png(
    bytes: &[u8],
    target: u64,
    cache: &mut ShrinkCache,
) -> Result<ShrinkOutcome, Error> {
    if target == 0 {
        return Err(INVALID);
    }
    if sniff(bytes) != Format::Png {
        return Err(UNSUPPORTED);
    }
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finish();
    cache.hash_passes += 1;
    if let Some(slot) = cache.slots.get(&digest) {
        if slot.target == target {
            cache.hits += 1;
            let mut hit = slot.outcome.clone();
            hit.dedup_hit = true;
            return Ok(hit);
        }
    }
    let info = png_header(bytes)?;
    if (bytes.len() as u64) <= target {
        let outcome = ShrinkOutcome {
            bytes: bytes.to_vec(),
            digest,
            dedup_hit: false,
            copied: true,
            quality: None,
            probes: Vec::new(),
            width: info.width,
            height: info.height,
            scales: 0,
        };
        remember(cache, target, &outcome);
        return Ok(outcome);
    }
    let decoded = decode_png(bytes)?;
    let mut image = into_rgb(decoded)?;
    let mut probes = Vec::new();
    let mut scales = 0u32;
    loop {
        let found = search(&image, target, cache, &mut probes)?;
        if let Some((quality, encoded)) = found {
            return Ok(publish(
                encoded,
                Fit {
                    quality,
                    width: image.width,
                    height: image.height,
                    scales,
                },
                digest,
                probes,
                target,
                cache,
            ));
        }
        if scales >= MAX_SCALES {
            break;
        }
        let nw = ((f64::from(image.width) * 0.7).round() as u32).max(1);
        let nh = ((f64::from(image.height) * 0.7).round() as u32).max(1);
        if nw == image.width && nh == image.height {
            break;
        }
        image = resize(&image, nw, nh, Filter::Area)?;
        scales += 1;
    }
    // The rounded standard-table search is unchanged for images that fit.
    // Re-decode so that path does not pay for a copy of the pixels.
    let decoded = decode_png(bytes)?;
    let mut image = into_rgb(decoded)?;
    let mut scales = 0u32;
    loop {
        let found = search_counted(&image, target, cache, &mut probes)?;
        if let Some((quality, encoded)) = found {
            return Ok(publish(
                encoded,
                Fit {
                    quality,
                    width: image.width,
                    height: image.height,
                    scales,
                },
                digest,
                probes,
                target,
                cache,
            ));
        }
        if scales >= MAX_SCALES {
            return Err(OVER_BUDGET);
        }
        let nw = truncated_scale(image.width);
        let nh = truncated_scale(image.height);
        if nw == image.width && nh == image.height {
            return Err(OVER_BUDGET);
        }
        image = resize(&image, nw, nh, Filter::Area)?;
        scales += 1;
    }
}

/// Measured JPEG shrink from a JPEG or HEIC source, one cap.
///
/// This is the phone-photo path and it mirrors the app's algorithm step for
/// step: a source at or under the cap is copied byte for byte; otherwise the
/// encoder runs a binary search over qualities 1-95 at the source
/// dimensions, each side is truncated to 70 percent (at least 2 pixels) and
/// the search repeats, at most six times; the encode that landed under the
/// cap is grown back to the cap with COM markers. The EXIF orientation is
/// applied first, so a portrait photo keeps its upright pixel count. There
/// is no second pass with counted Huffman tables here: the app runs the
/// standard tables only.
pub fn shrink_jpeg(
    bytes: &[u8],
    target: u64,
    cache: &mut ShrinkCache,
) -> Result<ShrinkOutcome, Error> {
    if target == 0 {
        return Err(INVALID);
    }
    if sniff(bytes) != Format::Jpeg {
        return Err(UNSUPPORTED);
    }
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finish();
    cache.hash_passes += 1;
    if let Some(slot) = cache.slots.get(&digest) {
        if slot.target == target {
            cache.hits += 1;
            let mut hit = slot.outcome.clone();
            hit.dedup_hit = true;
            return Ok(hit);
        }
    }
    let (source_width, source_height) = jpeg_size(bytes)?;
    if (bytes.len() as u64) <= target {
        let outcome = ShrinkOutcome {
            bytes: bytes.to_vec(),
            digest,
            dedup_hit: false,
            copied: true,
            quality: None,
            probes: Vec::new(),
            width: source_width,
            height: source_height,
            scales: 0,
        };
        remember(cache, target, &outcome);
        return Ok(outcome);
    }
    let decoded = decode_jpeg(bytes)?;
    let mut image = into_rgb(decoded)?;
    image = exif::transpose(&image, exif::orientation(bytes))?;
    let mut probes = Vec::new();
    let mut scales = 0u32;
    loop {
        // The app saves with optimize=True: Huffman tables counted from the
        // image. A quality-1 miss means every higher quality misses too.
        if let Some((quality, encoded)) = search_counted(&image, target, cache, &mut probes)? {
            return Ok(publish(
                encoded,
                Fit {
                    quality,
                    width: image.width,
                    height: image.height,
                    scales,
                },
                digest,
                probes,
                target,
                cache,
            ));
        }
        if scales >= MAX_SCALES {
            return Err(OVER_BUDGET);
        }
        let nw = truncated_scale(image.width);
        let nh = truncated_scale(image.height);
        if nw == image.width && nh == image.height {
            return Err(OVER_BUDGET);
        }
        image = resize(&image, nw, nh, Filter::Area)?;
        scales += 1;
    }
}

/// Width and height straight from the frame header, without decoding.
fn jpeg_size(bytes: &[u8]) -> Result<(u32, u32), Error> {
    let mut i = 2usize;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xff {
            return Err(INVALID);
        }
        let marker = bytes[i + 1];
        if marker == 0xd8 || (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        if marker == 0xda || marker == 0xd9 {
            return Err(UNSUPPORTED);
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if len < 2 || i + 2 + len > bytes.len() {
            return Err(INVALID);
        }
        // SOF0-SOF15 except DHT (C4), JPG (C8), and DAC (CC).
        if (0xc0..=0xcf).contains(&marker) && marker != 0xc4 && marker != 0xc8 && marker != 0xcc {
            if len < 7 {
                return Err(INVALID);
            }
            let height = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]);
            let width = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]);
            return Ok((u32::from(width), u32::from(height)));
        }
        i += 2 + len;
    }
    Err(UNSUPPORTED)
}

fn truncated_scale(n: u32) -> u32 {
    ((f64::from(n) * 0.7) as u32).max(2)
}

struct Fit {
    quality: u8,
    width: u32,
    height: u32,
    scales: u32,
}

fn publish(
    mut encoded: Vec<u8>,
    fit: Fit,
    digest: [u8; 32],
    probes: Vec<Probe>,
    target: u64,
    cache: &mut ShrinkCache,
) -> ShrinkOutcome {
    if (encoded.len() as u64) < target {
        pad_jpeg(&mut encoded, target as usize);
    }
    let outcome = ShrinkOutcome {
        bytes: encoded,
        digest,
        dedup_hit: false,
        copied: false,
        quality: Some(fit.quality),
        probes,
        width: fit.width,
        height: fit.height,
        scales: fit.scales,
    };
    remember(cache, target, &outcome);
    outcome
}

fn search(
    image: &Raster,
    target: u64,
    cache: &mut ShrinkCache,
    probes: &mut Vec<Probe>,
) -> Result<Option<(u8, Vec<u8>)>, Error> {
    let mut lo: u8 = 1;
    let mut hi: u8 = SHRINK_QUALITY_MAX;
    let mut best: Option<(u8, Vec<u8>)> = None;
    loop {
        let mid = lo + (hi - lo) / 2;
        cache.encodes += 1;
        match encode_jpeg_limited(image, mid, target as usize)? {
            Limited::Bytes(buf) => {
                probes.push(Probe {
                    quality: mid,
                    width: image.width,
                    height: image.height,
                    bytes: Some(buf.len() as u64),
                });
                best = Some((mid, buf));
                if mid == SHRINK_QUALITY_MAX {
                    break;
                }
                lo = mid + 1;
            }
            Limited::Exceeded => {
                probes.push(Probe {
                    quality: mid,
                    width: image.width,
                    height: image.height,
                    bytes: None,
                });
                if mid == 1 {
                    break;
                }
                hi = mid - 1;
            }
        }
        if lo > hi {
            break;
        }
    }
    Ok(best)
}

/// Same quality search as [`search`], with tables counted from `image`.
/// Quality 1 is tried first: a miss means every higher quality misses too,
/// so a scale that cannot fit does one full count pass.
fn search_counted(
    image: &Raster,
    target: u64,
    cache: &mut ShrinkCache,
    probes: &mut Vec<Probe>,
) -> Result<Option<(u8, Vec<u8>)>, Error> {
    cache.encodes += 1;
    let mut best = match encode_jpeg_optimized_limited(image, 1, target as usize)? {
        Limited::Bytes(buf) => {
            probes.push(Probe {
                quality: 1,
                width: image.width,
                height: image.height,
                bytes: Some(buf.len() as u64),
            });
            Some((1u8, buf))
        }
        Limited::Exceeded => {
            probes.push(Probe {
                quality: 1,
                width: image.width,
                height: image.height,
                bytes: None,
            });
            return Ok(None);
        }
    };
    let mut lo: u8 = 2;
    let mut hi: u8 = SHRINK_QUALITY_MAX;
    loop {
        if lo > hi {
            break;
        }
        let mid = lo + (hi - lo) / 2;
        cache.encodes += 1;
        match encode_jpeg_optimized_limited(image, mid, target as usize)? {
            Limited::Bytes(buf) => {
                probes.push(Probe {
                    quality: mid,
                    width: image.width,
                    height: image.height,
                    bytes: Some(buf.len() as u64),
                });
                best = Some((mid, buf));
                if mid == SHRINK_QUALITY_MAX {
                    break;
                }
                lo = mid + 1;
            }
            Limited::Exceeded => {
                probes.push(Probe {
                    quality: mid,
                    width: image.width,
                    height: image.height,
                    bytes: None,
                });
                if mid == 1 {
                    break;
                }
                hi = mid - 1;
            }
        }
    }
    Ok(best)
}

fn remember(cache: &mut ShrinkCache, target: u64, outcome: &ShrinkOutcome) {
    if outcome.bytes.len() > cache.budget {
        return;
    }
    if cache.held + outcome.bytes.len() > cache.budget {
        cache.slots.clear();
        cache.held = 0;
    }
    cache.held += outcome.bytes.len();
    let mut stored = outcome.clone();
    stored.dedup_hit = false;
    cache.slots.insert(
        outcome.digest,
        Slot {
            target,
            outcome: stored,
        },
    );
}

pub(crate) fn into_rgb(src: Raster) -> Result<Raster, Error> {
    match src.channels {
        3 => Ok(src),
        1 => {
            let mut pixels = Vec::with_capacity(src.pixels.len() * 3);
            for &y in &src.pixels {
                pixels.extend_from_slice(&[y, y, y]);
            }
            Raster::new(src.width, src.height, 3, pixels)
        }
        2 => {
            let mut pixels = Vec::with_capacity((src.pixels.len() / 2) * 3);
            for px in src.pixels.chunks_exact(2) {
                let y = over_white(px[0], px[1]);
                pixels.extend_from_slice(&[y, y, y]);
            }
            Raster::new(src.width, src.height, 3, pixels)
        }
        4 => {
            let mut pixels = Vec::with_capacity((src.pixels.len() / 4) * 3);
            for px in src.pixels.chunks_exact(4) {
                pixels.push(over_white(px[0], px[3]));
                pixels.push(over_white(px[1], px[3]));
                pixels.push(over_white(px[2], px[3]));
            }
            Raster::new(src.width, src.height, 3, pixels)
        }
        _ => Err(INVALID),
    }
}

fn over_white(fg: u8, alpha: u8) -> u8 {
    let a = u32::from(alpha);
    let fg = u32::from(fg);
    ((fg * a + 255 * (255 - a) + 127) / 255) as u8
}

/// Grow a baseline JPEG to `target` with COM markers before EOI.
pub(crate) fn pad_jpeg(buf: &mut Vec<u8>, target: usize) {
    if buf.len() >= target {
        return;
    }
    let need = target - buf.len();
    if buf.len() >= 2 && buf[buf.len() - 2] == 0xff && buf[buf.len() - 1] == 0xd9 {
        buf.truncate(buf.len() - 2);
    }
    let mut remaining = need;
    while remaining >= 4 {
        let added = remaining.min(2 + 65535);
        let length = (added - 2) as u16;
        let payload = (length - 2) as usize;
        buf.push(0xff);
        buf.push(0xfe);
        buf.extend_from_slice(&length.to_be_bytes());
        buf.extend(std::iter::repeat(0u8).take(payload));
        remaining -= added;
    }
    buf.extend_from_slice(&[0xff, 0xd9]);
    buf.extend(std::iter::repeat(0u8).take(remaining));
}

#[cfg(test)]
mod tests {
    use super::truncated_scale;
    use super::*;
    use crate::jpeg::encode_jpeg;

    #[test]
    fn truncated_scale_matches_the_python_chain() {
        let mut w = 6144u32;
        let mut h = 4608u32;
        let expect = [
            (4300u32, 3225u32),
            (3010, 2257),
            (2107, 1579),
            (1474, 1105),
            (1031, 773),
            (721, 541),
        ];
        for (ew, eh) in expect {
            w = truncated_scale(w);
            h = truncated_scale(h);
            assert_eq!((w, h), (ew, eh));
        }
    }

    /// A noisy 64×48 JPEG (noise keeps the encode well above the test caps).
    fn noisy_jpeg() -> Vec<u8> {
        let mut pixels = Vec::with_capacity(64 * 48 * 3);
        let mut state = 0x1234_5678u32;
        for _ in 0..64 * 48 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            pixels.push((state >> 24) as u8);
            pixels.push((state >> 16) as u8);
            pixels.push((state >> 8) as u8);
        }
        let raster = Raster::new(64, 48, 3, pixels).unwrap();
        encode_jpeg(&raster, 95).unwrap()
    }

    /// Splice an EXIF APP1 carrying the given orientation after the SOI.
    fn with_orientation(mut jpeg: Vec<u8>, orientation: u16) -> Vec<u8> {
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II\x2a\x00");
        tiff.extend_from_slice(&8u32.to_le_bytes());
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&0x0112u16.to_le_bytes());
        tiff.extend_from_slice(&3u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&orientation.to_le_bytes());
        tiff.extend_from_slice(&0u16.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes());
        let mut app1 = Vec::new();
        app1.extend_from_slice(&((6 + tiff.len() + 2) as u16).to_be_bytes());
        app1.extend_from_slice(b"Exif\x00\x00");
        app1.extend_from_slice(&tiff);
        let mut out = jpeg.drain(..2).collect::<Vec<u8>>();
        out.extend_from_slice(&[0xff, 0xe1]);
        out.extend_from_slice(&app1);
        out.extend_from_slice(&jpeg);
        out
    }

    #[test]
    fn a_jpeg_source_shrinks_to_exactly_the_cap() {
        let bytes = noisy_jpeg();
        assert!(bytes.len() > 2_000, "the fixture encodes past the test cap");
        let mut cache = ShrinkCache::new();
        let outcome = shrink_jpeg(&bytes, 2_000, &mut cache).unwrap();
        assert_eq!(outcome.bytes.len(), 2_000);
        assert_eq!(&outcome.bytes[..2], &[0xff, 0xd8]);
        let round = decode_jpeg(&outcome.bytes).unwrap();
        assert_eq!((round.width, round.height), (64, 48));
        assert!(!outcome.copied);
        assert!(outcome.quality.is_some());
    }

    #[test]
    fn an_under_cap_jpeg_is_copied() {
        let bytes = noisy_jpeg();
        let mut cache = ShrinkCache::new();
        let outcome = shrink_jpeg(&bytes, u64::from(bytes.len() as u32) + 10, &mut cache).unwrap();
        assert!(outcome.copied);
        assert_eq!(outcome.bytes, bytes);
    }

    #[test]
    fn the_exif_orientation_turns_the_output() {
        let bytes = with_orientation(noisy_jpeg(), 6);
        let mut cache = ShrinkCache::new();
        let outcome = shrink_jpeg(&bytes, 2_000, &mut cache).unwrap();
        let round = decode_jpeg(&outcome.bytes).unwrap();
        // Orientation 6 is stored sideways: the output is 48 wide, 64 tall.
        assert_eq!((round.width, round.height), (48, 64));
    }

    #[test]
    fn a_non_jpeg_source_is_refused() {
        let mut cache = ShrinkCache::new();
        let err = shrink_jpeg(b"not a jpeg", 1_000, &mut cache).unwrap_err();
        assert_eq!(err.code, crate::error::ErrorCode::Unsupported);
    }
}
