//! Byte-range cursor.
//!
//! Spans whose gap is `COALESCE_GAP` (4 KB) or less become one read. The cache
//! has an explicit byte budget; over budget, every entry except the range just
//! read is dropped. A `200` that carries the whole body is valid.

use crate::error::{Error, Result};
use crate::http::{self, GetCfg};
use std::collections::HashMap;

pub const COALESCE_GAP: u64 = 4096;
pub const DEFAULT_CACHE_BUDGET: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

#[allow(clippy::len_without_is_empty)]
impl ByteRange {
    pub fn new(start: u64, end: u64) -> Self {
        Self { start, end }
    }

    pub fn len(self) -> Option<u64> {
        self.end.checked_sub(self.start)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeSpec {
    Bounded(ByteRange),
    From(u64),
}

/// Gaps of `gap` bytes or less collapse into one span. Overlaps collapse too.
pub fn coalesce_ranges(ranges: &[ByteRange], gap: u64) -> Vec<ByteRange> {
    let mut ordered: Vec<ByteRange> = ranges
        .iter()
        .copied()
        .filter(|r| r.end > r.start)
        .collect();
    ordered.sort_by_key(|r| (r.start, r.end));
    let mut out: Vec<ByteRange> = Vec::new();
    for range in ordered {
        if let Some(last) = out.last_mut() {
            if range.start <= last.end.saturating_add(gap) {
                if range.end > last.end {
                    last.end = range.end;
                }
                continue;
            }
        }
        out.push(range);
    }
    out
}

/// How many network reads a cursor would issue, and the original slices.
pub fn cursor_plan(src: &[u8], ranges: &[ByteRange], gap: u64) -> (Vec<Vec<u8>>, usize) {
    let merged = coalesce_ranges(ranges, gap);
    let mut out = Vec::with_capacity(ranges.len());
    for range in ranges {
        if range.end < range.start {
            out.push(Vec::new());
            continue;
        }
        let start = (range.start as usize).min(src.len());
        let end = (range.end as usize).min(src.len());
        out.push(src[start..end].to_vec());
    }
    (out, merged.len())
}

#[derive(Clone, Debug)]
struct Cached {
    start: u64,
    data: Vec<u8>,
}

#[derive(Debug)]
pub struct ByteCache {
    budget: u64,
    used: u64,
    entries: HashMap<String, Vec<Cached>>,
}

impl ByteCache {
    pub fn new(budget: u64) -> Self {
        Self {
            budget,
            used: 0,
            entries: HashMap::new(),
        }
    }

    pub fn budget(&self) -> u64 {
        self.budget
    }

    pub fn used(&self) -> u64 {
        self.used
    }

    pub fn remove(&mut self, url: &str) {
        if let Some(spans) = self.entries.remove(url) {
            for span in spans {
                self.used = self.used.saturating_sub(span.data.len() as u64);
            }
        }
    }

    pub fn get(&self, url: &str, start: u64, end: u64) -> Option<Vec<u8>> {
        let spans = self.entries.get(url)?;
        for span in spans {
            let span_end = span.start.saturating_add(span.data.len() as u64);
            if start >= span.start && end <= span_end {
                let from = (start - span.start) as usize;
                let to = (end - span.start) as usize;
                return Some(span.data[from..to].to_vec());
            }
        }
        None
    }

    /// Store `data` at `start`. If it does not fit, drop every other entry and
    /// keep only this range. A range larger than the budget is not stored.
    pub fn insert(&mut self, url: &str, start: u64, data: Vec<u8>) {
        let len = data.len() as u64;
        if len > self.budget {
            self.entries.clear();
            self.used = 0;
            return;
        }
        if self.used.saturating_add(len) > self.budget {
            self.entries.clear();
            self.used = 0;
        }
        self.used = self.used.saturating_add(len);
        self.entries
            .entry(url.to_string())
            .or_default()
            .push(Cached { start, data });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchKind {
    Fetch,
    Remove,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchOp {
    pub url: String,
    pub kind: BatchKind,
    pub ranges: Vec<ByteRange>,
}

#[derive(Default)]
struct Acc {
    fetch: bool,
    remove: bool,
    whole: bool,
    ranges: Vec<ByteRange>,
}

/// One op per URL. A fetch and a remove in the same batch end as a remove,
/// whichever came first, so the fetch cannot resurrect the URL.
pub fn collapse_batch(ops: &[BatchOp], gap: u64) -> Vec<BatchOp> {
    let mut order = Vec::new();
    let mut grouped: HashMap<String, Acc> = HashMap::new();
    for op in ops {
        if !grouped.contains_key(&op.url) {
            order.push(op.url.clone());
        }
        let acc = grouped.entry(op.url.clone()).or_default();
        match op.kind {
            BatchKind::Remove => acc.remove = true,
            BatchKind::Fetch => {
                acc.fetch = true;
                if op.ranges.is_empty() {
                    acc.whole = true;
                } else {
                    acc.ranges.extend(op.ranges.iter().copied());
                }
            }
        }
    }
    let mut out = Vec::new();
    for url in order {
        let acc = &grouped[&url];
        if acc.remove {
            out.push(BatchOp {
                url,
                kind: BatchKind::Remove,
                ranges: Vec::new(),
            });
        } else if acc.fetch {
            let ranges = if acc.whole {
                Vec::new()
            } else {
                coalesce_ranges(&acc.ranges, gap)
            };
            out.push(BatchOp {
                url,
                kind: BatchKind::Fetch,
                ranges,
            });
        }
    }
    out
}

pub fn apply_batch(ops: &[BatchOp], cache: &mut ByteCache, gap: u64) -> Vec<BatchOp> {
    let collapsed = collapse_batch(ops, gap);
    for op in &collapsed {
        if op.kind == BatchKind::Remove {
            cache.remove(&op.url);
        }
    }
    collapsed
}

pub struct ByteCursor<'a> {
    cfg: &'a GetCfg,
    url: String,
    cache: &'a mut ByteCache,
    gap: u64,
    pos: u64,
}

impl<'a> ByteCursor<'a> {
    pub fn new(cfg: &'a GetCfg, url: impl Into<String>, cache: &'a mut ByteCache) -> Self {
        Self {
            cfg,
            url: url.into(),
            cache,
            gap: COALESCE_GAP,
            pos: 0,
        }
    }

    pub fn position(&self) -> u64 {
        self.pos
    }

    pub fn seek(&mut self, pos: u64) {
        self.pos = pos;
    }

    pub fn read(&mut self, len: u64) -> Result<Vec<u8>> {
        let start = self.pos;
        let end = start.saturating_add(len);
        let bytes = self.read_at(start, end)?;
        self.pos = end;
        Ok(bytes)
    }

    pub fn read_at(&mut self, start: u64, end: u64) -> Result<Vec<u8>> {
        let mut got = self.fill(&[ByteRange::new(start, end)])?;
        Ok(got.pop().unwrap_or_default())
    }

    /// Coalesce, then one HTTP read per merged span. Original slices are returned.
    pub fn fill(&mut self, ranges: &[ByteRange]) -> Result<Vec<Vec<u8>>> {
        if ranges.is_empty() {
            return Ok(Vec::new());
        }
        for range in ranges {
            let Some(len) = range.len() else {
                return Err(Error::Http("range end is before start".into()));
            };
            if self.cache.budget() > 0 && len > self.cache.budget() {
                return Err(Error::TooLarge {
                    limit: self.cache.budget(),
                    got: len,
                });
            }
        }
        if let Some(hit) = cache_all(self.cache, &self.url, ranges) {
            return Ok(hit);
        }
        let merged = coalesce_ranges(ranges, self.gap);
        let mut parts: Vec<(ByteRange, Vec<u8>)> = Vec::new();
        for span in &merged {
            if let Some(hit) = self.cache.get(&self.url, span.start, span.end) {
                parts.push((*span, hit));
                continue;
            }
            match http::fetch_bounded(self.cfg, &self.url, *span, self.cache.budget())? {
                http::Fetched::Whole(body) => {
                    if (body.len() as u64) <= self.cache.budget() {
                        self.cache.insert(&self.url, 0, body.clone());
                    }
                    return Ok(slice_many(&body, ranges));
                }
                http::Fetched::Part(buf) => {
                    if (buf.len() as u64) <= self.cache.budget() {
                        self.cache.insert(&self.url, span.start, buf.clone());
                    }
                    parts.push((*span, buf));
                }
            }
        }
        let mut out = Vec::with_capacity(ranges.len());
        for range in ranges {
            out.push(cut_part(&parts, range)?);
        }
        Ok(out)
    }
}

fn cache_all(cache: &ByteCache, url: &str, ranges: &[ByteRange]) -> Option<Vec<Vec<u8>>> {
    let mut out = Vec::with_capacity(ranges.len());
    for range in ranges {
        out.push(cache.get(url, range.start, range.end)?);
    }
    Some(out)
}

fn slice_many(body: &[u8], ranges: &[ByteRange]) -> Vec<Vec<u8>> {
    ranges
        .iter()
        .map(|range| {
            let start = (range.start as usize).min(body.len());
            let end = (range.end as usize).min(body.len());
            body[start..end].to_vec()
        })
        .collect()
}

fn cut_part(parts: &[(ByteRange, Vec<u8>)], range: &ByteRange) -> Result<Vec<u8>> {
    for (span, data) in parts {
        if range.start >= span.start && range.end <= span.end {
            let from = (range.start - span.start) as usize;
            let to = (range.end - span.start) as usize;
            if to > data.len() || from > data.len() || from > to {
                return Err(Error::Http("short range body".into()));
            }
            return Ok(data[from..to].to_vec());
        }
    }
    Err(Error::Http("range was not covered by a read".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{self, Resp};
    use crate::http::GetCfg;

    #[test]
    fn coalesces_gaps_of_4kib_or_less() {
        let merged = coalesce_ranges(
            &[ByteRange::new(0, 100), ByteRange::new(4196, 4200)],
            COALESCE_GAP,
        );
        assert_eq!(merged, vec![ByteRange::new(0, 4200)]);
        let split = coalesce_ranges(
            &[ByteRange::new(0, 100), ByteRange::new(4197, 4200)],
            COALESCE_GAP,
        );
        assert_eq!(
            split,
            vec![ByteRange::new(0, 100), ByteRange::new(4197, 4200)]
        );
    }

    #[test]
    fn cursor_plan_counts_one_read_for_a_small_gap() {
        let mut src = vec![0u8; 8000];
        src[100] = 7;
        src[200] = 9;
        let (slices, reads) = cursor_plan(
            &src,
            &[ByteRange::new(100, 120), ByteRange::new(200, 210)],
            COALESCE_GAP,
        );
        assert_eq!(reads, 1);
        assert_eq!(slices[0], src[100..120]);
        assert_eq!(slices[1], src[200..210]);
    }

    #[test]
    fn cache_drops_everything_except_the_new_range() {
        let mut cache = ByteCache::new(1000);
        cache.insert("a", 0, vec![1; 600]);
        cache.insert("b", 0, vec![2; 600]);
        assert!(cache.get("a", 0, 4).is_none());
        assert_eq!(cache.get("b", 0, 4).unwrap(), vec![2; 4]);
        cache.insert("b", 0, vec![3; 2000]);
        assert!(cache.get("b", 0, 1).is_none());
        assert_eq!(cache.used(), 0);
    }

    #[test]
    fn batch_remove_cancels_fetch_regardless_of_order() {
        let fetch = BatchOp {
            url: "http://h/a".into(),
            kind: BatchKind::Fetch,
            ranges: vec![ByteRange::new(0, 10), ByteRange::new(20, 30)],
        };
        let remove = BatchOp {
            url: "http://h/a".into(),
            kind: BatchKind::Remove,
            ranges: Vec::new(),
        };
        let other = BatchOp {
            url: "http://h/b".into(),
            kind: BatchKind::Fetch,
            ranges: vec![ByteRange::new(0, 5), ByteRange::new(8, 12)],
        };
        let collapsed = collapse_batch(&[fetch.clone(), remove.clone()], COALESCE_GAP);
        assert_eq!(collapsed.len(), 1);
        assert_eq!(collapsed[0].kind, BatchKind::Remove);
        let flipped = collapse_batch(&[remove, fetch, other], COALESCE_GAP);
        assert_eq!(flipped[0].kind, BatchKind::Remove);
        assert_eq!(flipped[1].kind, BatchKind::Fetch);
        assert_eq!(flipped[1].ranges, vec![ByteRange::new(0, 12)]);
    }

    #[test]
    fn http_cursor_coalesces_and_accepts_whole_body_200() {
        let body: Vec<u8> = (0..8000u32).map(|i| (i % 251) as u8).collect();
        let body2 = body.clone();
        let server = harness::serve(move |req| {
            if req.target.starts_with("/whole") {
                Resp {
                    status: 200,
                    headers: vec![("Content-Type".into(), "application/octet-stream".into())],
                    body: body2.clone(),
                    chunked: false,
                }
            } else {
                harness::with_range(req, &body2)
            }
        });
        let cfg = test_cfg();
        let url = server.url("/file");
        let mut cache = ByteCache::new(64 * 1024);
        let mut cursor = ByteCursor::new(&cfg, &url, &mut cache);
        let got = cursor
            .fill(&[ByteRange::new(100, 120), ByteRange::new(200, 210)])
            .unwrap();
        assert_eq!(server.hits.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(got[0], body[100..120]);
        assert_eq!(got[1], body[200..210]);
        let raw = server.last_raw.lock().unwrap().clone();
        assert!(raw.to_ascii_lowercase().contains("range: bytes=100-209"));
        assert!(!raw.contains("Mozilla"));
        assert!(!raw.contains("Cookie"));

        let far = server.url("/far");
        let mut cache = ByteCache::new(64 * 1024);
        let mut cursor = ByteCursor::new(&cfg, &far, &mut cache);
        cursor
            .fill(&[ByteRange::new(0, 10), ByteRange::new(5000, 5010)])
            .unwrap();
        assert_eq!(server.hits.load(std::sync::atomic::Ordering::SeqCst), 3);

        let whole = server.url("/whole");
        let mut cache = ByteCache::new(64 * 1024);
        let mut cursor = ByteCursor::new(&cfg, &whole, &mut cache);
        let slice = cursor.read_at(5, 8).unwrap();
        assert_eq!(slice, body[5..8]);
        let hits = server.hits.load(std::sync::atomic::Ordering::SeqCst);
        let again = cursor.read_at(10, 14).unwrap();
        assert_eq!(again, body[10..14]);
        assert_eq!(server.hits.load(std::sync::atomic::Ordering::SeqCst), hits);
    }

    #[test]
    fn oversized_whole_body_is_sliced_not_cached() {
        let body = vec![9u8; 5000];
        let server = harness::serve(move |req| Resp {
            status: 200,
            headers: vec![],
            body: body.clone(),
            chunked: req.target.contains("chunk"),
        });
        let cfg = test_cfg();
        let url = server.url("/big");
        let mut cache = ByteCache::new(100);
        let mut cursor = ByteCursor::new(&cfg, &url, &mut cache);
        let got = cursor.read_at(10, 18).unwrap();
        assert_eq!(got, vec![9u8; 8]);
        let hits = server.hits.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(cursor.read_at(10, 18).unwrap(), vec![9u8; 8]);
        assert_eq!(server.hits.load(std::sync::atomic::Ordering::SeqCst), hits);
        assert_eq!(cursor.read_at(0, 4).unwrap(), vec![9u8; 4]);
        assert!(server.hits.load(std::sync::atomic::Ordering::SeqCst) > hits);
    }

    fn test_cfg() -> GetCfg {
        GetCfg {
            timeout: std::time::Duration::from_secs(2),
            max_redirects: 4,
            max_bytes: None,
            check_ssrf: true,
            permit_loopback: true,
        }
    }
}
