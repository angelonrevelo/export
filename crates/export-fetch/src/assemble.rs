//! Ordered byte assembly. One fixed buffer, no full-file copy.

use crate::error::Result;
use crate::range::{coalesce_ranges, ByteRange};
use std::io::Write;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssembleStats {
    pub bytes: u64,
    pub peak_buffered: u64,
    pub reads: u64,
}

pub fn assemble_stream<W: Write>(src: &[u8], chunk: usize, writer: &mut W) -> Result<AssembleStats> {
    let chunk = chunk.max(1);
    let mut buf = vec![0u8; chunk];
    let mut off = 0;
    while off < src.len() {
        let n = (src.len() - off).min(chunk);
        buf[..n].copy_from_slice(&src[off..off + n]);
        writer.write_all(&buf[..n])?;
        off += n;
    }
    Ok(AssembleStats {
        bytes: src.len() as u64,
        peak_buffered: chunk as u64,
        reads: 1,
    })
}

/// Coalesce first. Filler inside a merged span is read and not written.
pub fn assemble_coalesced<W: Write>(
    src: &[u8],
    ranges: &[ByteRange],
    gap: u64,
    chunk: usize,
    writer: &mut W,
) -> Result<AssembleStats> {
    let merged = coalesce_ranges(ranges, gap);
    let chunk = chunk.max(1);
    let mut buf = vec![0u8; chunk];
    let mut written = 0u64;
    for range in ranges {
        if range.end <= range.start {
            continue;
        }
        let start = (range.start as usize).min(src.len());
        let end = (range.end as usize).min(src.len());
        let mut off = start;
        while off < end {
            let n = (end - off).min(chunk);
            buf[..n].copy_from_slice(&src[off..off + n]);
            writer.write_all(&buf[..n])?;
            written += n as u64;
            off += n;
        }
    }
    Ok(AssembleStats {
        bytes: written,
        peak_buffered: chunk as u64,
        reads: merged.len() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::range::COALESCE_GAP;
    use std::io::Write;

    struct Stat {
        max: usize,
        data: Vec<u8>,
    }
    impl Write for Stat {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.max = self.max.max(buf.len());
            self.data.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn stream_keeps_a_fixed_buffer() {
        let src: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let mut out = Stat {
            max: 0,
            data: Vec::new(),
        };
        let stats = assemble_stream(&src, 64 * 1024, &mut out).unwrap();
        assert_eq!(out.data, src);
        assert!(out.max <= 64 * 1024);
        assert_eq!(stats.peak_buffered, 64 * 1024);
        assert!(stats.peak_buffered < src.len() as u64);
    }

    #[test]
    fn coalesced_write_skips_a_wide_gap_and_counts_reads() {
        let src: Vec<u8> = (0..8000u32).map(|i| i as u8).collect();
        let close = [
            ByteRange::new(0, 10),
            ByteRange::new(20, 30),
        ];
        let mut out = Stat {
            max: 0,
            data: Vec::new(),
        };
        let stats = assemble_coalesced(&src, &close, COALESCE_GAP, 64, &mut out).unwrap();
        assert_eq!(stats.reads, 1);
        assert_eq!(&out.data[..10], &src[..10]);
        assert_eq!(&out.data[10..], &src[20..30]);
        let far = [ByteRange::new(0, 10), ByteRange::new(5000, 5010)];
        let stats = assemble_coalesced(&src, &far, COALESCE_GAP, 64, &mut Stat {
            max: 0,
            data: Vec::new(),
        })
        .unwrap();
        assert_eq!(stats.reads, 2);
    }
}
