//! FLAC subset decode and encode.
//!
//! The encoder writes STREAMINFO, then fixed-blocksize frames. Each subframe
//! is constant, verbatim, or fixed order 0..=4, with a single Rice partition
//! or an unencoded residual. One LPC candidate (order 11, or the subset cap
//! when that is lower, at precision 12) replaces that fixed predictor only
//! when the written subframe is strictly shorter. A subframe whose samples
//! share low zero bits stores those samples shifted and writes that wasted-bit
//! count. `encode` does not write a seek table. `encode_with_seek_table` adds
//! one seekpoint per block. The decoder also accepts stereo decorrelation,
//! wasted bits, and Rice partitions.

use crate::crc::{crc8, crc16};
use crate::error::{Error, Result};
use crate::md5::Md5;

/// Interleaved samples kept in memory at once.
pub const MAX_SAMPLES: usize = 16 * 1024 * 1024;

/// Signed interleaved PCM. `samples.len()` is `channels` times the
/// per-channel count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pcm {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits: u8,
    pub samples: Vec<i32>,
}

#[derive(Clone, Copy)]
struct Info {
    min_block: u16,
    max_block: u16,
    sample_rate: u32,
    channels: u16,
    bits: u8,
    total_samples: u64,
    md5: [u8; 16],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Assign {
    Indep,
    LeftSide,
    RightSide,
    MidSide,
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    fn at(data: &'a [u8], byte: usize) -> Self {
        Self {
            data,
            pos: byte.saturating_mul(8),
        }
    }

    fn read_bits(&mut self, n: u32) -> Result<u32> {
        if n == 0 {
            return Ok(0);
        }
        if n > 32 {
            return Err(Error::Corrupt("bits"));
        }
        let end = self
            .pos
            .checked_add(n as usize)
            .ok_or(Error::Corrupt("bits"))?;
        let end_byte = (end + 7) / 8;
        if end_byte > self.data.len() {
            return Err(Error::Truncated {
                at: self.data.len(),
                need: end_byte - self.data.len(),
            });
        }
        let start_byte = self.pos / 8;
        let mut acc = 0u64;
        for i in start_byte..end_byte {
            acc = (acc << 8) | u64::from(self.data[i]);
        }
        let loaded = (end_byte - start_byte) * 8;
        let skip = self.pos % 8;
        let shift = loaded - skip - n as usize;
        let mask = if n == 32 {
            u32::MAX
        } else {
            (1u32 << n) - 1
        };
        let v = ((acc >> shift) as u32) & mask;
        self.pos = end;
        Ok(v)
    }

    fn read_unary(&mut self) -> Result<u32> {
        let mut q = 0u32;
        loop {
            let byte_i = self.pos / 8;
            if byte_i >= self.data.len() {
                return Err(Error::Truncated {
                    at: self.data.len(),
                    need: 1,
                });
            }
            let bit_off = (self.pos % 8) as u32;
            let avail = 8 - bit_off;
            let shifted = self.data[byte_i] << bit_off;
            let lz = shifted.leading_zeros();
            if lz >= avail {
                q = q.checked_add(avail).ok_or(Error::Corrupt("rice"))?;
                self.pos += avail as usize;
                continue;
            }
            q = q.checked_add(lz).ok_or(Error::Corrupt("rice"))?;
            self.pos += lz as usize + 1;
            return Ok(q);
        }
    }

    fn align_zero(&mut self) -> Result<()> {
        let rem = (self.pos % 8) as u32;
        if rem != 0 {
            let pad = self.read_bits(8 - rem)?;
            if pad != 0 {
                return Err(Error::Corrupt("padding"));
            }
        }
        Ok(())
    }
}

struct BitWriter {
    buf: Vec<u8>,
    cur: u8,
    nbit: u32,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            cur: 0,
            nbit: 0,
        }
    }

    fn write_bits(&mut self, val: u32, mut n: u32) {
        if n == 0 || n > 32 {
            return;
        }
        let mut val = if n == 32 {
            val
        } else {
            val & ((1u32 << n) - 1)
        };
        while n > 0 {
            let space = 8 - self.nbit;
            let take = n.min(space);
            let shift = n - take;
            let piece = ((val >> shift) & ((1u32 << take) - 1)) as u8;
            self.cur |= piece << (space - take);
            self.nbit += take;
            n -= take;
            if n > 0 && n < 32 {
                val &= (1u32 << n) - 1;
            }
            if self.nbit == 8 {
                self.buf.push(self.cur);
                self.cur = 0;
                self.nbit = 0;
            }
        }
    }

    fn write_signed(&mut self, v: i32, n: u32) {
        if n == 0 || n > 32 {
            return;
        }
        let mask = if n == 32 {
            u32::MAX
        } else {
            (1u32 << n) - 1
        };
        self.write_bits(v as u32 & mask, n);
    }

    fn align(&mut self) {
        if self.nbit > 0 {
            self.buf.push(self.cur);
            self.cur = 0;
            self.nbit = 0;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        self.align();
        self.buf
    }

    fn bit_len(&self) -> usize {
        self.buf.len() * 8 + self.nbit as usize
    }
}

pub fn decode(bytes: &[u8]) -> Result<Pcm> {
    if bytes.len() < 4 {
        return Err(Error::Truncated {
            at: bytes.len(),
            need: 4 - bytes.len(),
        });
    }
    let (info, mut pos) = read_metadata(bytes)?;
    let channels = info.channels as usize;
    let mut samples = Vec::new();
    if info.total_samples > 0 {
        let n = (info.total_samples as usize)
            .checked_mul(channels)
            .ok_or(Error::Corrupt("sample count"))?;
        if n > MAX_SAMPLES {
            return Err(Error::Limit {
                cap: MAX_SAMPLES as u64,
                got: n as u64,
            });
        }
        samples.reserve(n);
    }
    let mut frame_index = 0u64;
    let mut expected_block: Option<u32> = None;
    while pos < bytes.len() {
        let before = pos;
        let frame = read_frame(bytes, &mut pos, &info)?;
        if pos <= before {
            return Err(Error::Corrupt("frame"));
        }
        if let Some(exp) = expected_block {
            if frame.block != exp && pos != bytes.len() {
                return Err(Error::Corrupt("block size"));
            }
        } else {
            expected_block = Some(frame.block);
        }
        if frame.variable {
            let have = (samples.len() / channels) as u64;
            if frame.number != have {
                return Err(Error::Corrupt("frame number"));
            }
        } else if frame.number != frame_index {
            return Err(Error::Corrupt("frame number"));
        }
        let add = frame.block as usize * channels;
        if samples.len().saturating_add(add) > MAX_SAMPLES {
            return Err(Error::Limit {
                cap: MAX_SAMPLES as u64,
                got: samples.len().saturating_add(add) as u64,
            });
        }
        for i in 0..frame.block as usize {
            for ch in &frame.channels {
                samples.push(ch[i]);
            }
        }
        frame_index += 1;
    }
    if samples.is_empty() {
        return Err(Error::Corrupt("no frames"));
    }
    if info.total_samples != 0 {
        let expect = (info.total_samples as usize)
            .checked_mul(channels)
            .ok_or(Error::Corrupt("sample count"))?;
        if samples.len() != expect {
            return Err(Error::Corrupt("sample count"));
        }
    }
    let pcm = Pcm {
        sample_rate: info.sample_rate,
        channels: info.channels,
        bits: info.bits,
        samples,
    };
    if info.md5 != [0u8; 16] {
        let got = audio_md5(&pcm);
        if got != info.md5 {
            return Err(Error::Corrupt("md5"));
        }
    }
    Ok(pcm)
}

struct Frame {
    block: u32,
    number: u64,
    variable: bool,
    channels: Vec<Vec<i32>>,
}

fn read_metadata(data: &[u8]) -> Result<(Info, usize)> {
    if &data[0..4] != b"fLaC" {
        return Err(Error::Corrupt("magic"));
    }
    let mut pos = 4usize;
    let mut info = None;
    let mut first = true;
    loop {
        if pos + 4 > data.len() {
            return Err(Error::Truncated {
                at: data.len(),
                need: pos + 4 - data.len(),
            });
        }
        let last = data[pos] & 0x80 != 0;
        let kind = data[pos] & 0x7f;
        let len = (u32::from(data[pos + 1]) << 16)
            | (u32::from(data[pos + 2]) << 8)
            | u32::from(data[pos + 3]);
        let len = len as usize;
        pos += 4;
        if pos + len > data.len() {
            return Err(Error::Truncated {
                at: data.len(),
                need: pos + len - data.len(),
            });
        }
        let body = &data[pos..pos + len];
        pos += len;
        if first {
            if kind != 0 || len != 34 {
                return Err(Error::Corrupt("streaminfo"));
            }
            info = Some(parse_streaminfo(body)?);
            first = false;
        } else if kind == 0 || kind == 127 {
            return Err(Error::Corrupt("metadata"));
        }
        if last {
            break;
        }
    }
    Ok((info.ok_or(Error::Corrupt("streaminfo"))?, pos))
}

fn parse_streaminfo(body: &[u8]) -> Result<Info> {
    let mut br = BitReader::at(body, 0);
    let min_block = br.read_bits(16)? as u16;
    let max_block = br.read_bits(16)? as u16;
    let _min_frame = br.read_bits(24)?;
    let _max_frame = br.read_bits(24)?;
    let sample_rate = br.read_bits(20)?;
    let channels = br.read_bits(3)? as u16 + 1;
    let bits = br.read_bits(5)? as u8 + 1;
    let hi = br.read_bits(4)? as u64;
    let lo = br.read_bits(32)? as u64;
    let total_samples = (hi << 32) | lo;
    if body.len() < 34 {
        return Err(Error::Corrupt("streaminfo"));
    }
    let mut md5 = [0u8; 16];
    md5.copy_from_slice(&body[18..34]);
    if min_block < 16 || max_block < min_block || sample_rate == 0 || !(4..=32).contains(&bits) {
        return Err(Error::Corrupt("streaminfo"));
    }
    Ok(Info {
        min_block,
        max_block,
        sample_rate,
        channels,
        bits,
        total_samples,
        md5,
    })
}

fn read_frame(data: &[u8], pos: &mut usize, info: &Info) -> Result<Frame> {
    let start = *pos;
    if data.len() < start + 6 {
        return Err(Error::Truncated {
            at: data.len(),
            need: start + 6 - data.len(),
        });
    }
    if data[start] != 0xff || (data[start + 1] & 0xfe) != 0xf8 {
        return Err(Error::Corrupt("sync"));
    }
    let variable = data[start + 1] & 1 == 1;
    let bs_code = data[start + 2] >> 4;
    let sr_code = data[start + 2] & 0x0f;
    let ch_code = data[start + 3] >> 4;
    let bps_code = (data[start + 3] >> 1) & 0x07;
    if data[start + 3] & 1 != 0 {
        return Err(Error::Corrupt("frame reserved"));
    }
    *pos = start + 4;
    let (number, nlen) = read_utf8(&data[*pos..])?;
    *pos += nlen;
    let block = match bs_code {
        1 => 192u32,
        2 => 576,
        3 => 1152,
        4 => 2304,
        5 => 4608,
        6 => u32::from(take_u8(data, pos)?) + 1,
        7 => u32::from(take_u16(data, pos)?) + 1,
        8..=15 => 256u32 << (bs_code - 8),
        _ => return Err(Error::Corrupt("block size")),
    };
    if !(16..=65535).contains(&block) {
        return Err(Error::Corrupt("block size"));
    }
    if block < u32::from(info.min_block) || block > u32::from(info.max_block) {
        return Err(Error::Corrupt("block size"));
    }
    let rate = match sr_code {
        0 => info.sample_rate,
        1 => 88_200,
        2 => 176_400,
        3 => 192_000,
        4 => 8_000,
        5 => 16_000,
        6 => 22_050,
        7 => 24_000,
        8 => 32_000,
        9 => 44_100,
        10 => 48_000,
        11 => 96_000,
        12 => u32::from(take_u8(data, pos)?) * 1000,
        13 => u32::from(take_u16(data, pos)?),
        14 => u32::from(take_u16(data, pos)?) * 10,
        _ => return Err(Error::Corrupt("sample rate")),
    };
    if rate != info.sample_rate || rate == 0 {
        return Err(Error::Corrupt("sample rate"));
    }
    let (channels, assign) = match ch_code {
        0..=7 => (ch_code + 1, Assign::Indep),
        8 => (2, Assign::LeftSide),
        9 => (2, Assign::RightSide),
        10 => (2, Assign::MidSide),
        _ => return Err(Error::Corrupt("channel")),
    };
    if u16::from(channels) != info.channels {
        return Err(Error::Corrupt("channel"));
    }
    let bps = match bps_code {
        0 => u32::from(info.bits),
        1 => 8,
        2 => 12,
        4 => 16,
        5 => 20,
        6 => 24,
        7 => 32,
        _ => return Err(Error::Corrupt("bps")),
    };
    if bps != u32::from(info.bits) {
        return Err(Error::Corrupt("bps"));
    }
    if *pos >= data.len() {
        return Err(Error::Truncated {
            at: data.len(),
            need: 1,
        });
    }
    let expect_crc = data[*pos];
    if crc8(&data[start..*pos]) != expect_crc {
        return Err(Error::Corrupt("crc8"));
    }
    *pos += 1;
    let mut br = BitReader::at(data, *pos);
    let mut chans = Vec::with_capacity(channels as usize);
    for i in 0..channels {
        let mut width = bps;
        if matches!(assign, Assign::LeftSide | Assign::MidSide) && i == 1 {
            width += 1;
        }
        if assign == Assign::RightSide && i == 0 {
            width += 1;
        }
        chans.push(read_subframe(&mut br, block as usize, width)?);
    }
    decorrelate(&mut chans, assign);
    br.align_zero()?;
    if br.pos % 8 != 0 {
        return Err(Error::Corrupt("padding"));
    }
    let crc_at = br.pos / 8;
    if crc_at + 2 > data.len() {
        return Err(Error::Truncated {
            at: data.len(),
            need: crc_at + 2 - data.len(),
        });
    }
    let expect = u16::from_be_bytes([data[crc_at], data[crc_at + 1]]);
    if crc16(&data[start..crc_at]) != expect {
        return Err(Error::Corrupt("crc16"));
    }
    *pos = crc_at + 2;
    Ok(Frame {
        block,
        number,
        variable,
        channels: chans,
    })
}

fn take_u8(data: &[u8], pos: &mut usize) -> Result<u8> {
    if *pos >= data.len() {
        return Err(Error::Truncated {
            at: data.len(),
            need: 1,
        });
    }
    let v = data[*pos];
    *pos += 1;
    Ok(v)
}

fn take_u16(data: &[u8], pos: &mut usize) -> Result<u16> {
    if *pos + 2 > data.len() {
        return Err(Error::Truncated {
            at: data.len(),
            need: *pos + 2 - data.len(),
        });
    }
    let v = u16::from_be_bytes([data[*pos], data[*pos + 1]]);
    *pos += 2;
    Ok(v)
}

fn read_utf8(data: &[u8]) -> Result<(u64, usize)> {
    if data.is_empty() {
        return Err(Error::Truncated { at: 0, need: 1 });
    }
    let b0 = data[0];
    if b0 & 0x80 == 0 {
        return Ok((u64::from(b0), 1));
    }
    let width = b0.leading_ones() as usize;
    if !(2..=7).contains(&width) {
        return Err(Error::Corrupt("utf8"));
    }
    if data.len() < width {
        return Err(Error::Truncated {
            at: data.len(),
            need: width - data.len(),
        });
    }
    let payload = 7 - width;
    let mask = if payload == 0 {
        0
    } else {
        (1u8 << payload) - 1
    };
    let mut v = u64::from(b0 & mask);
    for &b in data.iter().take(width).skip(1) {
        if b & 0xc0 != 0x80 {
            return Err(Error::Corrupt("utf8"));
        }
        v = (v << 6) | u64::from(b & 0x3f);
    }
    Ok((v, width))
}

fn read_signed(br: &mut BitReader<'_>, n: u32) -> Result<i32> {
    if n == 0 {
        return Ok(0);
    }
    if n > 32 {
        return Err(Error::Corrupt("bits"));
    }
    let u = br.read_bits(n)?;
    if n == 32 {
        return Ok(u as i32);
    }
    let sign = 1u32 << (n - 1);
    if u & sign != 0 {
        Ok((u | (!0u32 << n)) as i32)
    } else {
        Ok(u as i32)
    }
}

fn read_subframe(br: &mut BitReader<'_>, block: usize, bps: u32) -> Result<Vec<i32>> {
    let (kind, wasted) = read_subframe_header(br)?;
    read_subframe_body(br, block, bps, kind, wasted)
}

fn read_subframe_header(br: &mut BitReader<'_>) -> Result<(u32, u32)> {
    if br.read_bits(1)? != 0 {
        return Err(Error::Corrupt("subframe pad"));
    }
    let kind = br.read_bits(6)?;
    let wasted = if br.read_bits(1)? == 1 {
        br.read_unary()?.saturating_add(1)
    } else {
        0
    };
    Ok((kind, wasted))
}

fn read_subframe_body(
    br: &mut BitReader<'_>,
    block: usize,
    bps: u32,
    kind: u32,
    wasted: u32,
) -> Result<Vec<i32>> {
    if wasted >= bps {
        return Err(Error::Corrupt("wasted"));
    }
    let stored = bps - wasted;
    let mut samples = match kind {
        0 => {
            let s = read_signed(br, stored)?;
            vec![s; block]
        }
        1 => {
            let mut v = Vec::with_capacity(block);
            for _ in 0..block {
                v.push(read_signed(br, stored)?);
            }
            v
        }
        8..=12 => {
            let order = (kind - 8) as usize;
            if order > block {
                return Err(Error::Corrupt("order"));
            }
            let mut warm = Vec::with_capacity(order);
            for _ in 0..order {
                warm.push(read_signed(br, stored)?);
            }
            let residual = read_residual(br, block, order)?;
            restore_fixed(warm, &residual, order)?
        }
        32..=63 => {
            let order = (kind - 31) as usize;
            if order == 0 || order > block || order > 32 {
                return Err(Error::Corrupt("order"));
            }
            let mut warm = Vec::with_capacity(order);
            for _ in 0..order {
                warm.push(read_signed(br, stored)?);
            }
            let prec_code = br.read_bits(4)?;
            if prec_code == 15 {
                return Err(Error::Corrupt("lpc"));
            }
            let prec = prec_code + 1;
            let shift_u = br.read_bits(5)?;
            let shift = if shift_u & 0x10 != 0 {
                shift_u as i32 - 32
            } else {
                shift_u as i32
            };
            let mut coeff = Vec::with_capacity(order);
            for _ in 0..order {
                coeff.push(read_signed(br, prec)?);
            }
            let residual = read_residual(br, block, order)?;
            restore_lpc(warm, &residual, &coeff, shift)?
        }
        _ => return Err(Error::Corrupt("subframe")),
    };
    if wasted > 0 {
        for s in &mut samples {
            *s = s.checked_shl(wasted).ok_or(Error::Corrupt("wasted"))?;
        }
    }
    if samples.len() != block {
        return Err(Error::Corrupt("block"));
    }
    Ok(samples)
}

fn read_residual(br: &mut BitReader<'_>, block: usize, order: usize) -> Result<Vec<i32>> {
    let method = br.read_bits(2)?;
    if method > 1 {
        return Err(Error::Corrupt("residual"));
    }
    let part_order = br.read_bits(4)? as usize;
    if part_order > 15 {
        return Err(Error::Corrupt("partition"));
    }
    let parts = 1usize << part_order;
    if block % parts != 0 {
        return Err(Error::Corrupt("partition"));
    }
    let part_samples = block / parts;
    if order > part_samples {
        return Err(Error::Corrupt("partition"));
    }
    let param_bits = if method == 0 { 4 } else { 5 };
    let escape = (1u32 << param_bits) - 1;
    let mut out = Vec::with_capacity(block - order);
    for p in 0..parts {
        let count = if p == 0 {
            part_samples - order
        } else {
            part_samples
        };
        let rice = br.read_bits(param_bits)?;
        if rice == escape {
            let width = br.read_bits(5)?;
            for _ in 0..count {
                out.push(read_signed(br, width)?);
            }
        } else {
            for _ in 0..count {
                out.push(read_rice(br, rice)?);
            }
        }
    }
    if out.len() != block - order {
        return Err(Error::Corrupt("residual"));
    }
    Ok(out)
}

fn read_rice(br: &mut BitReader<'_>, param: u32) -> Result<i32> {
    let q = br.read_unary()?;
    let r = if param == 0 {
        0
    } else {
        br.read_bits(param)?
    };
    let u = (u64::from(q))
        .checked_shl(param)
        .ok_or(Error::Corrupt("rice"))?
        | u64::from(r);
    if u > u64::from(u32::MAX) {
        return Err(Error::Corrupt("rice"));
    }
    Ok(zigzag_dec(u as u32))
}

fn restore_fixed(mut data: Vec<i32>, residual: &[i32], order: usize) -> Result<Vec<i32>> {
    if data.len() != order {
        return Err(Error::Corrupt("order"));
    }
    for &r in residual {
        let i = data.len();
        let pred = match order {
            0 => 0i64,
            1 => i64::from(data[i - 1]),
            2 => 2 * i64::from(data[i - 1]) - i64::from(data[i - 2]),
            3 => {
                3 * i64::from(data[i - 1]) - 3 * i64::from(data[i - 2]) + i64::from(data[i - 3])
            }
            4 => {
                4 * i64::from(data[i - 1]) - 6 * i64::from(data[i - 2]) + 4 * i64::from(data[i - 3])
                    - i64::from(data[i - 4])
            }
            _ => return Err(Error::Corrupt("order")),
        };
        data.push(fit_i32(pred + i64::from(r))?);
    }
    Ok(data)
}

fn restore_lpc(mut data: Vec<i32>, residual: &[i32], coeff: &[i32], shift: i32) -> Result<Vec<i32>> {
    let order = coeff.len();
    if data.len() != order {
        return Err(Error::Corrupt("order"));
    }
    for &r in residual {
        let i = data.len();
        let mut sum = 0i64;
        for (j, c) in coeff.iter().enumerate() {
            sum += i64::from(*c) * i64::from(data[i - 1 - j]);
        }
        let pred = if shift >= 0 {
            if shift >= 63 {
                return Err(Error::Corrupt("lpc"));
            }
            sum >> shift
        } else {
            let sh = -shift;
            if sh >= 63 {
                return Err(Error::Corrupt("lpc"));
            }
            sum << sh
        };
        data.push(fit_i32(pred + i64::from(r))?);
    }
    Ok(data)
}

fn fit_i32(v: i64) -> Result<i32> {
    if v < i64::from(i32::MIN) || v > i64::from(i32::MAX) {
        Err(Error::Corrupt("sample"))
    } else {
        Ok(v as i32)
    }
}

fn decorrelate(ch: &mut [Vec<i32>], assign: Assign) {
    if ch.len() != 2 {
        return;
    }
    let n = ch[0].len();
    match assign {
        Assign::Indep => {}
        Assign::LeftSide => {
            for i in 0..n {
                ch[1][i] = ch[0][i].wrapping_sub(ch[1][i]);
            }
        }
        Assign::RightSide => {
            for i in 0..n {
                ch[0][i] = ch[1][i].wrapping_add(ch[0][i]);
            }
        }
        Assign::MidSide => {
            for i in 0..n {
                let mid = i64::from(ch[0][i]);
                let side = i64::from(ch[1][i]);
                let mid = (mid << 1) | (side & 1);
                ch[0][i] = ((mid + side) >> 1) as i32;
                ch[1][i] = ((mid - side) >> 1) as i32;
            }
        }
    }
}

fn zigzag_enc(n: i32) -> u32 {
    if n >= 0 {
        (n as u32) << 1
    } else {
        n.unsigned_abs().wrapping_shl(1).wrapping_sub(1)
    }
}

fn zigzag_dec(u: u32) -> i32 {
    let u = u as i64;
    if u & 1 == 0 {
        (u >> 1) as i32
    } else {
        (-((u >> 1) + 1)) as i32
    }
}

fn write_rice(w: &mut BitWriter, sample: i32, param: u32) {
    let u = zigzag_enc(sample);
    let mut q = if param >= 32 { 0 } else { u >> param };
    while q >= 32 {
        w.write_bits(0, 32);
        q -= 32;
    }
    if q > 0 {
        w.write_bits(0, q);
    }
    w.write_bits(1, 1);
    if param > 0 && param < 32 {
        let mask = (1u32 << param) - 1;
        w.write_bits(u & mask, param);
    }
}

pub fn encode(pcm: &Pcm) -> Result<Vec<u8>> {
    encode_with(pcm, 4096, false)
}

/// Same encoder as [`encode`], plus one seekpoint per block.
///
/// `encode` does not write a seek table. This is the only path that does.
pub fn encode_with_seek_table(pcm: &Pcm) -> Result<Vec<u8>> {
    encode_with(pcm, 4096, true)
}

fn encode_with(pcm: &Pcm, nominal: usize, seek_table: bool) -> Result<Vec<u8>> {
    if pcm.channels == 0 || pcm.channels > 8 {
        return Err(Error::Unsupported("channel count"));
    }
    if !matches!(pcm.bits, 8 | 16 | 24) {
        return Err(Error::Unsupported("bit depth"));
    }
    if !rate_encodable(pcm.sample_rate) {
        return Err(Error::Unsupported("sample rate"));
    }
    let channels = pcm.channels as usize;
    if pcm.samples.len() % channels != 0 {
        return Err(Error::Corrupt("channel alignment"));
    }
    let per = pcm.samples.len() / channels;
    if per < 16 {
        return Err(Error::Unsupported("fewer than 16 samples"));
    }
    if pcm.samples.len() > MAX_SAMPLES {
        return Err(Error::Limit {
            cap: MAX_SAMPLES as u64,
            got: pcm.samples.len() as u64,
        });
    }
    if !pcm.samples.iter().all(|s| sample_fits(*s, pcm.bits)) {
        return Err(Error::Corrupt("sample"));
    }
    let cap = if pcm.sample_rate <= 48_000 { 4608 } else { 16384 };
    let nominal = nominal.clamp(16, cap).min(per);
    let blocks = plan_blocks(per, nominal);
    let mut frames = Vec::new();
    let mut points = Vec::new();
    let mut min_block = u16::MAX;
    let mut max_block = 0u16;
    let mut min_frame = u32::MAX;
    let mut max_frame = 0u32;
    let mut done = 0usize;
    let mut frame_off = 0u64;
    for (idx, block) in blocks.iter().copied().enumerate() {
        let frame = write_frame(pcm, done, block, idx as u64)?;
        let flen = frame.len() as u32;
        min_frame = min_frame.min(flen);
        max_frame = max_frame.max(flen);
        let b = block as u16;
        min_block = min_block.min(b);
        max_block = max_block.max(b);
        if seek_table {
            points.push((done as u64, frame_off, b));
        }
        frame_off += u64::from(flen);
        frames.extend_from_slice(&frame);
        done += block;
    }
    if done != per {
        return Err(Error::Corrupt("block"));
    }
    let md5 = audio_md5(pcm);
    let info = write_streaminfo(
        min_block,
        max_block,
        min_frame,
        max_frame,
        pcm.sample_rate,
        pcm.channels,
        pcm.bits,
        per as u64,
        md5,
    );
    let table = if seek_table {
        Some(seek_table_block(&points))
    } else {
        None
    };
    let extra = table.as_ref().map(|t| t.len()).unwrap_or(0);
    let mut out = Vec::with_capacity(42 + extra + frames.len());
    out.extend_from_slice(b"fLaC");
    out.push(if table.is_some() { 0x00 } else { 0x80 });
    out.push(0);
    out.push(0);
    out.push(34);
    out.extend_from_slice(&info);
    if let Some(table) = table {
        out.extend_from_slice(&table);
    }
    out.extend_from_slice(&frames);
    Ok(out)
}

fn seek_table_block(points: &[(u64, u64, u16)]) -> Vec<u8> {
    let len = points.len() * 18;
    let mut out = Vec::with_capacity(4 + len);
    out.push(0x83);
    out.push((len >> 16) as u8);
    out.push((len >> 8) as u8);
    out.push(len as u8);
    for &(sample, offset, count) in points {
        out.extend_from_slice(&sample.to_be_bytes());
        out.extend_from_slice(&offset.to_be_bytes());
        out.extend_from_slice(&count.to_be_bytes());
    }
    out
}

fn plan_blocks(n: usize, nominal: usize) -> Vec<usize> {
    let mut blocks = Vec::new();
    let mut off = 0usize;
    while off < n {
        let mut take = nominal.min(n - off);
        let rem = n - off - take;
        if rem > 0 && rem < 16 && take > 16 {
            let give = 16 - rem;
            if take - give >= 16 {
                take -= give;
            }
        }
        if take < 16 {
            blocks.push(n - off);
            break;
        }
        blocks.push(take);
        off += take;
    }
    blocks
}

fn sample_fits(s: i32, bits: u8) -> bool {
    if bits == 0 || bits > 32 {
        return false;
    }
    if bits == 32 {
        return true;
    }
    let max = (1i32 << (bits - 1)) - 1;
    let min = -max - 1;
    s >= min && s <= max
}

fn rate_encodable(rate: u32) -> bool {
    rate > 0 && rate <= 655_350 && (rate <= 65_535 || rate % 10 == 0)
}

#[allow(clippy::too_many_arguments)]
fn write_streaminfo(
    min_block: u16,
    max_block: u16,
    min_frame: u32,
    max_frame: u32,
    rate: u32,
    channels: u16,
    bits: u8,
    total: u64,
    md5: [u8; 16],
) -> [u8; 34] {
    let mut w = BitWriter::new();
    w.write_bits(u32::from(min_block), 16);
    w.write_bits(u32::from(max_block), 16);
    w.write_bits(min_frame & 0x00ff_ffff, 24);
    w.write_bits(max_frame & 0x00ff_ffff, 24);
    w.write_bits(rate & 0x000f_ffff, 20);
    w.write_bits(u32::from(channels - 1), 3);
    w.write_bits(u32::from(bits - 1), 5);
    w.write_bits((total >> 32) as u32, 4);
    w.write_bits(total as u32, 32);
    for b in md5 {
        w.write_bits(u32::from(b), 8);
    }
    let bytes = w.finish();
    let mut out = [0u8; 34];
    let n = bytes.len().min(34);
    out[..n].copy_from_slice(&bytes[..n]);
    out
}

fn audio_md5(pcm: &Pcm) -> [u8; 16] {
    let width = (usize::from(pcm.bits) + 7) / 8;
    let mut m = Md5::new();
    for &s in &pcm.samples {
        let tmp = (s as u32).to_le_bytes();
        m.update(&tmp[..width]);
    }
    m.finish()
}

fn write_frame(pcm: &Pcm, start: usize, block: usize, frame_number: u64) -> Result<Vec<u8>> {
    let mut hdr = Vec::with_capacity(16);
    hdr.push(0xff);
    hdr.push(0xf8);
    let (bs_code, bs_extra) = block_size_code(block as u16);
    let (sr_code, sr_extra) = sample_rate_code(pcm.sample_rate)?;
    let bps_code = match pcm.bits {
        8 => 1u8,
        16 => 4,
        24 => 6,
        _ => return Err(Error::Unsupported("bit depth")),
    };
    hdr.push((bs_code << 4) | sr_code);
    let chan = (pcm.channels - 1) as u8;
    hdr.push((chan << 4) | (bps_code << 1));
    push_utf8(&mut hdr, frame_number);
    if let Some(v) = bs_extra {
        if bs_code == 6 {
            hdr.push(v as u8);
        } else {
            hdr.extend_from_slice(&v.to_be_bytes());
        }
    }
    if let Some(v) = sr_extra {
        if sr_code == 12 {
            hdr.push(v as u8);
        } else {
            hdr.extend_from_slice(&v.to_be_bytes());
        }
    }
    let c = crc8(&hdr);
    hdr.push(c);
    let mut body = BitWriter::new();
    let channels = pcm.channels as usize;
    for ch in 0..channels {
        let mut samples = Vec::with_capacity(block);
        for i in 0..block {
            samples.push(pcm.samples[(start + i) * channels + ch]);
        }
        write_subframe(&mut body, &samples, u32::from(pcm.bits), pcm.sample_rate);
    }
    let mut frame = hdr;
    frame.extend_from_slice(&body.finish());
    let sum = crc16(&frame);
    frame.extend_from_slice(&sum.to_be_bytes());
    Ok(frame)
}

fn block_size_code(n: u16) -> (u8, Option<u16>) {
    match n {
        192 => (1, None),
        576 => (2, None),
        1152 => (3, None),
        2304 => (4, None),
        4608 => (5, None),
        256 => (8, None),
        512 => (9, None),
        1024 => (10, None),
        2048 => (11, None),
        4096 => (12, None),
        8192 => (13, None),
        16384 => (14, None),
        32768 => (15, None),
        1..=256 => (6, Some(n - 1)),
        _ => (7, Some(n - 1)),
    }
}

fn sample_rate_code(rate: u32) -> Result<(u8, Option<u16>)> {
    let coded = match rate {
        88_200 => (1, None),
        176_400 => (2, None),
        192_000 => (3, None),
        8_000 => (4, None),
        16_000 => (5, None),
        22_050 => (6, None),
        24_000 => (7, None),
        32_000 => (8, None),
        44_100 => (9, None),
        48_000 => (10, None),
        96_000 => (11, None),
        r if r % 1000 == 0 && r / 1000 <= 255 => (12, Some((r / 1000) as u16)),
        r if r <= 65_535 => (13, Some(r as u16)),
        r if r % 10 == 0 && r / 10 <= 65_535 => (14, Some((r / 10) as u16)),
        _ => return Err(Error::Unsupported("sample rate")),
    };
    Ok(coded)
}

fn push_utf8(out: &mut Vec<u8>, n: u64) {
    if n < 0x80 {
        out.push(n as u8);
    } else if n < 0x800 {
        out.push((0xc0 | (n >> 6)) as u8);
        out.push((0x80 | (n & 0x3f)) as u8);
    } else if n < 0x1_0000 {
        out.push((0xe0 | (n >> 12)) as u8);
        out.push((0x80 | ((n >> 6) & 0x3f)) as u8);
        out.push((0x80 | (n & 0x3f)) as u8);
    } else if n < 0x20_0000 {
        out.push((0xf0 | (n >> 18)) as u8);
        out.push((0x80 | ((n >> 12) & 0x3f)) as u8);
        out.push((0x80 | ((n >> 6) & 0x3f)) as u8);
        out.push((0x80 | (n & 0x3f)) as u8);
    } else if n < 0x400_0000 {
        out.push((0xf8 | (n >> 24)) as u8);
        out.push((0x80 | ((n >> 18) & 0x3f)) as u8);
        out.push((0x80 | ((n >> 12) & 0x3f)) as u8);
        out.push((0x80 | ((n >> 6) & 0x3f)) as u8);
        out.push((0x80 | (n & 0x3f)) as u8);
    } else if n < 0x8000_0000 {
        out.push((0xfc | (n >> 30)) as u8);
        out.push((0x80 | ((n >> 24) & 0x3f)) as u8);
        out.push((0x80 | ((n >> 18) & 0x3f)) as u8);
        out.push((0x80 | ((n >> 12) & 0x3f)) as u8);
        out.push((0x80 | ((n >> 6) & 0x3f)) as u8);
        out.push((0x80 | (n & 0x3f)) as u8);
    } else {
        out.push(0xfe);
        out.push((0x80 | ((n >> 30) & 0x3f)) as u8);
        out.push((0x80 | ((n >> 24) & 0x3f)) as u8);
        out.push((0x80 | ((n >> 18) & 0x3f)) as u8);
        out.push((0x80 | ((n >> 12) & 0x3f)) as u8);
        out.push((0x80 | ((n >> 6) & 0x3f)) as u8);
        out.push((0x80 | (n & 0x3f)) as u8);
    }
}

enum Coded {
    Constant(i32),
    Verbatim,
    Fixed { order: usize, escape: Option<u32>, param: u32 },
    Lpc {
        precision: u32,
        shift: i32,
        coeff: Vec<i32>,
        escape: Option<u32>,
        param: u32,
    },
}

fn common_wasted(samples: &[i32], bps: u32) -> u32 {
    if bps <= 1 {
        return 0;
    }
    let mut k = bps - 1;
    for &s in samples {
        let zeros = (s as u32).trailing_zeros();
        if zeros == 0 {
            return 0;
        }
        if zeros < k {
            k = zeros;
        }
    }
    k
}

fn write_subframe(w: &mut BitWriter, samples: &[i32], bps: u32, rate: u32) {
    let wasted = common_wasted(samples, bps);
    if wasted == 0 {
        write_stored_subframe(w, samples, bps, rate, 0);
        return;
    }
    let shifted: Vec<i32> = samples.iter().map(|s| s >> wasted).collect();
    write_stored_subframe(w, &shifted, bps - wasted, rate, wasted);
}

fn write_stored_subframe(w: &mut BitWriter, samples: &[i32], bps: u32, rate: u32, wasted: u32) {
    let (mut coded, fixed_bits) = choose(samples, bps, wasted);
    if matches!(coded, Coded::Fixed { .. }) {
        debug_assert_eq!(
            coded_bit_len(samples, bps, wasted, &coded) as u64,
            fixed_bits,
            "fixed bit estimate drifted"
        );
        if let Some(lpc) = smaller_lpc(samples, bps, rate, fixed_bits, wasted) {
            coded = lpc;
        }
    }
    write_coded(w, samples, bps, wasted, &coded);
}

fn write_coded(w: &mut BitWriter, samples: &[i32], bps: u32, wasted: u32, coded: &Coded) {
    match coded {
        Coded::Constant(s) => {
            write_sub_header(w, 0, wasted);
            w.write_signed(*s, bps);
        }
        Coded::Verbatim => {
            write_sub_header(w, 1, wasted);
            for &s in samples {
                w.write_signed(s, bps);
            }
        }
        Coded::Fixed {
            order,
            escape,
            param,
        } => {
            let Some(residual) = fixed_residual(samples, *order) else {
                write_verbatim(w, samples, bps, wasted);
                return;
            };
            write_sub_header(w, 8 + *order as u32, wasted);
            for &s in &samples[..*order] {
                w.write_signed(s, bps);
            }
            write_residual(w, &residual, *param, *escape);
        }
        Coded::Lpc {
            precision,
            shift,
            coeff,
            escape,
            param,
        } => {
            let Some(residual) = lpc_residual(samples, coeff, *shift) else {
                write_verbatim(w, samples, bps, wasted);
                return;
            };
            write_sub_header(w, 31 + coeff.len() as u32, wasted);
            for &s in &samples[..coeff.len()] {
                w.write_signed(s, bps);
            }
            w.write_bits(precision.saturating_sub(1), 4);
            w.write_bits((*shift as u32) & 31, 5);
            for &c in coeff {
                w.write_signed(c, *precision);
            }
            write_residual(w, &residual, *param, *escape);
        }
    }
}

fn write_verbatim(w: &mut BitWriter, samples: &[i32], bps: u32, wasted: u32) {
    write_sub_header(w, 1, wasted);
    for &s in samples {
        w.write_signed(s, bps);
    }
}

fn coded_bit_len(samples: &[i32], bps: u32, wasted: u32, coded: &Coded) -> usize {
    let mut w = BitWriter::new();
    write_coded(&mut w, samples, bps, wasted, coded);
    w.bit_len()
}

fn write_sub_header(w: &mut BitWriter, kind: u32, wasted: u32) {
    w.write_bits(0, 1);
    w.write_bits(kind, 6);
    if wasted == 0 {
        w.write_bits(0, 1);
        return;
    }
    w.write_bits(1, 1);
    if wasted > 1 {
        w.write_bits(0, wasted - 1);
    }
    w.write_bits(1, 1);
}

fn choose(samples: &[i32], bps: u32, wasted: u32) -> (Coded, u64) {
    let header = 8 + u64::from(wasted);
    if samples.iter().all(|s| *s == samples[0]) {
        return (Coded::Constant(samples[0]), header + u64::from(bps));
    }
    // Costs include the subframe header, so they match a written subframe.
    let verbatim = header + samples.len() as u64 * u64::from(bps);
    let mut best_cost = verbatim;
    let mut best = Coded::Verbatim;
    let max_order = 4.min(samples.len().saturating_sub(1));
    for order in 0..=max_order {
        let Some(residual) = fixed_residual(samples, order) else {
            continue;
        };
        let (param, rice) = best_rice(&residual);
        let warm = u64::from(bps) * order as u64;
        let rice_cost = 18 + u64::from(wasted) + rice + warm;
        let width = signed_width(&residual);
        let esc_cost = 23 + u64::from(wasted) + u64::from(width) * residual.len() as u64 + warm;
        if esc_cost < rice_cost && esc_cost < best_cost {
            best_cost = esc_cost;
            best = Coded::Fixed {
                order,
                escape: Some(width),
                param: 0,
            };
        } else if rice_cost < best_cost {
            best_cost = rice_cost;
            best = Coded::Fixed {
                order,
                escape: None,
                param,
            };
        }
    }
    (best, best_cost)
}

fn lpc_order_cap(rate: u32, len: usize) -> usize {
    let cap = if rate <= 48_000 { 12 } else { 32 };
    cap.min(len.saturating_sub(1))
}

/// One LPC candidate: order 11 (or the subset cap, when that is lower) at
/// precision 12. It replaces the fixed subframe only when that candidate is
/// strictly shorter. A wider search on this fixture encoded in about 474 ms,
/// past the time budget, so orders and precisions that are not this one are
/// not tried. Constant and verbatim are left alone.
fn smaller_lpc(samples: &[i32], bps: u32, rate: u32, fixed_bits: u64, wasted: u32) -> Option<Coded> {
    let max_order = lpc_order_cap(rate, samples.len());
    let order = 11.min(max_order);
    if order == 0 || samples.len() <= order {
        return None;
    }
    let precision = 12u32;
    let prefix = 17 + u64::from(wasted) + order as u64 * (u64::from(bps) + u64::from(precision));
    if fixed_bits <= prefix + 15 {
        return None;
    }
    let r = hann_autocorr(samples, order);
    if !r[0].is_finite() || r[0] <= 0.0 {
        return None;
    }
    let mut prev = [0.0f64; 33];
    let mut next = [0.0f64; 33];
    let mut err = r[0];
    for i in 1..=order {
        if !err.is_finite() || err <= 0.0 {
            return None;
        }
        let mut acc = r[i];
        for j in 1..i {
            acc -= prev[j] * r[i - j];
        }
        let k = acc / err;
        if !k.is_finite() || k.abs() >= 1.0 {
            return None;
        }
        next[i] = k;
        for j in 1..i {
            next[j] = prev[j] - k * prev[i - j];
        }
        prev[1..=i].copy_from_slice(&next[1..=i]);
        err *= 1.0 - k * k;
    }
    let mut coeff = Vec::with_capacity(order);
    let shift = quantize_into(&prev[1..=order], precision, &mut coeff)?;
    let mut residual = Vec::new();
    if !lpc_residual_into(samples, &coeff, shift, &mut residual) {
        return None;
    }
    let (param, rice) = best_rice(&residual);
    let width = signed_width(&residual);
    let rice_total = prefix + 10 + rice;
    let esc_total = prefix + 15 + u64::from(width) * residual.len() as u64;
    let (total, escape, param) = if esc_total < rice_total {
        (esc_total, Some(width), 0)
    } else {
        (rice_total, None, param)
    };
    if total >= fixed_bits {
        return None;
    }
    let coded = Coded::Lpc {
        precision,
        shift,
        coeff,
        escape,
        param,
    };
    debug_assert_eq!(
        coded_bit_len(samples, bps, wasted, &coded) as u64,
        total,
        "lpc bit estimate drifted"
    );
    Some(coded)
}

fn hann_autocorr(samples: &[i32], max_order: usize) -> Vec<f64> {
    let n = samples.len();
    let mut windowed = vec![0.0f64; n];
    let denom = (n - 1) as f64;
    for (i, slot) in windowed.iter_mut().enumerate() {
        let hann = 0.5 * (1.0 - (2.0 * std::f64::consts::PI * (i as f64) / denom).cos());
        *slot = f64::from(samples[i]) * hann;
    }
    let mut r = vec![0.0; max_order + 1];
    for (lag, slot) in r.iter_mut().enumerate() {
        let mut sum = 0.0;
        for i in lag..n {
            sum += windowed[i] * windowed[i - lag];
        }
        *slot = sum;
    }
    r
}

fn quantize_into(lpc: &[f64], precision: u32, out: &mut Vec<i32>) -> Option<i32> {
    out.clear();
    if precision == 0 || precision > 15 || lpc.is_empty() {
        return None;
    }
    let max_q = (1i32 << (precision - 1)) - 1;
    let mut max_abs = 0.0f64;
    for &c in lpc {
        if !c.is_finite() {
            return None;
        }
        max_abs = max_abs.max(c.abs());
    }
    if max_abs == 0.0 {
        return None;
    }
    let ratio = f64::from(max_q) / max_abs;
    if !ratio.is_finite() || ratio <= 0.0 {
        return None;
    }
    let shift_f = ratio.log2().floor();
    if shift_f < -16.0 {
        return None;
    }
    let mut shift = if shift_f > 15.0 { 15 } else { shift_f as i32 };
    while shift >= -16 {
        match scale_coeffs(lpc, shift, max_q, out) {
            Scale::Ok => return Some(shift),
            Scale::Shrink => shift -= 1,
            Scale::Stop => return None,
        }
    }
    None
}

enum Scale {
    Ok,
    Shrink,
    Stop,
}

fn scale_coeffs(lpc: &[f64], shift: i32, max_q: i32, out: &mut Vec<i32>) -> Scale {
    out.clear();
    let scale = if shift >= 0 {
        f64::from(1u32 << (shift as u32))
    } else {
        let sh = (-shift) as u32;
        if sh >= 31 {
            return Scale::Stop;
        }
        1.0 / f64::from(1u32 << sh)
    };
    let lo = f64::from(-max_q);
    let hi = f64::from(max_q);
    for &c in lpc {
        let q = (c * scale).round();
        if !q.is_finite() || q < lo || q > hi {
            out.clear();
            return Scale::Shrink;
        }
        out.push(q as i32);
    }
    if out.iter().all(|c| *c == 0) {
        out.clear();
        Scale::Stop
    } else {
        Scale::Ok
    }
}

fn lpc_residual(samples: &[i32], coeff: &[i32], shift: i32) -> Option<Vec<i32>> {
    let mut out = Vec::new();
    if lpc_residual_into(samples, coeff, shift, &mut out) {
        Some(out)
    } else {
        None
    }
}

fn lpc_residual_into(samples: &[i32], coeff: &[i32], shift: i32, out: &mut Vec<i32>) -> bool {
    out.clear();
    let order = coeff.len();
    if order == 0 || order >= samples.len() || !(-16..=15).contains(&shift) {
        return false;
    }
    out.reserve(samples.len() - order);
    for i in order..samples.len() {
        let mut sum = 0i64;
        for (j, &c) in coeff.iter().enumerate() {
            sum += i64::from(c) * i64::from(samples[i - 1 - j]);
        }
        let pred = if shift >= 0 {
            sum >> shift
        } else {
            sum << (-shift)
        };
        let Ok(sample) = fit_i32(i64::from(samples[i]) - pred) else {
            out.clear();
            return false;
        };
        out.push(sample);
    }
    true
}

fn fixed_residual(samples: &[i32], order: usize) -> Option<Vec<i32>> {
    let mut out = Vec::with_capacity(samples.len() - order);
    for i in order..samples.len() {
        let pred = match order {
            0 => 0i64,
            1 => i64::from(samples[i - 1]),
            2 => 2 * i64::from(samples[i - 1]) - i64::from(samples[i - 2]),
            3 => {
                3 * i64::from(samples[i - 1]) - 3 * i64::from(samples[i - 2])
                    + i64::from(samples[i - 3])
            }
            4 => {
                4 * i64::from(samples[i - 1]) - 6 * i64::from(samples[i - 2])
                    + 4 * i64::from(samples[i - 3])
                    - i64::from(samples[i - 4])
            }
            _ => return None,
        };
        out.push(fit_i32(i64::from(samples[i]) - pred).ok()?);
    }
    Some(out)
}

fn best_rice(res: &[i32]) -> (u32, u64) {
    let mut bits = [0u64; 15];
    for &s in res {
        let u = u64::from(zigzag_enc(s));
        bits[0] += u + 1;
        bits[1] += (u >> 1) + 2;
        bits[2] += (u >> 2) + 3;
        bits[3] += (u >> 3) + 4;
        bits[4] += (u >> 4) + 5;
        bits[5] += (u >> 5) + 6;
        bits[6] += (u >> 6) + 7;
        bits[7] += (u >> 7) + 8;
        bits[8] += (u >> 8) + 9;
        bits[9] += (u >> 9) + 10;
        bits[10] += (u >> 10) + 11;
        bits[11] += (u >> 11) + 12;
        bits[12] += (u >> 12) + 13;
        bits[13] += (u >> 13) + 14;
        bits[14] += (u >> 14) + 15;
    }
    let mut best_p = 0u32;
    let mut best = u64::MAX;
    for (p, b) in bits.iter().copied().enumerate() {
        if b < best {
            best = b;
            best_p = p as u32;
        }
    }
    (best_p, best)
}

fn signed_width(samples: &[i32]) -> u32 {
    let mut need = 0u32;
    for &s in samples {
        let bits = if s > 0 {
            (32 - (s as u32).leading_zeros()) + 1
        } else if s == 0 {
            0
        } else if s == i32::MIN {
            32
        } else {
            let abs = s.unsigned_abs();
            let mag = 32 - abs.leading_zeros();
            if abs.is_power_of_two() {
                mag
            } else {
                mag + 1
            }
        };
        if bits > need {
            need = bits;
        }
    }
    need
}

fn write_residual(w: &mut BitWriter, res: &[i32], param: u32, escape: Option<u32>) {
    w.write_bits(0, 2);
    w.write_bits(0, 4);
    if let Some(width) = escape {
        w.write_bits(0b1111, 4);
        w.write_bits(width, 5);
        if width > 0 {
            for &s in res {
                w.write_signed(s, width);
            }
        }
    } else {
        w.write_bits(param, 4);
        for &s in res {
            write_rice(w, s, param);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(pcm: &Pcm) {
        let bytes = encode(pcm).unwrap();
        let back = decode(&bytes).unwrap();
        assert_eq!(back.samples, pcm.samples);
        assert_eq!(back.sample_rate, pcm.sample_rate);
        assert_eq!(back.channels, pcm.channels);
        assert_eq!(back.bits, pcm.bits);
    }

    #[test]
    fn sine_round_trip_exact() {
        let samples: Vec<i32> = (0..2048)
            .map(|i| {
                let t = f64::from(i);
                ((t * 0.17).sin() * 9000.0 + (t * 0.04).sin() * 2500.0).round() as i32
            })
            .collect();
        let (picked, bits) = choose(&samples, 16, 0);
        assert_eq!(coded_bit_len(&samples, 16, 0, &picked) as u64, bits);
        let lpc = smaller_lpc(&samples, 16, 44_100, bits, 0);
        assert!(
            lpc.is_some(),
            "a two-tone sine should beat the fixed predictor"
        );
        round_trip(&Pcm {
            sample_rate: 44_100,
            channels: 1,
            bits: 16,
            samples,
        });
    }

    #[test]
    fn silence_is_constant_and_exact() {
        let pcm = Pcm {
            sample_rate: 44_100,
            channels: 1,
            bits: 16,
            samples: vec![0; 4096],
        };
        let bytes = encode(&pcm).unwrap();
        assert!(bytes.len() < 200, "constant frame grew to {}", bytes.len());
        round_trip(&pcm);
    }

    #[test]
    fn ramp_uses_fixed_predictor() {
        let samples: Vec<i32> = (0..5000).map(|i| (i % 1000) - 500).collect();
        let pcm = Pcm {
            sample_rate: 48_000,
            channels: 1,
            bits: 16,
            samples,
        };
        let bytes = encode(&pcm).unwrap();
        assert!(bytes.len() < 2500, "ramp grew to {}", bytes.len());
        round_trip(&pcm);
    }

    #[test]
    fn noise_verbatim_round_trip() {
        let samples: Vec<i32> = (0..4096i32)
            .map(|i| i.wrapping_mul(1_103_515_245).wrapping_add(12_345) >> 8)
            .map(|s| s.clamp(-32_768, 32_767))
            .collect();
        let pcm = Pcm {
            sample_rate: 44_100,
            channels: 1,
            bits: 16,
            samples,
        };
        let bytes = encode(&pcm).unwrap();
        assert!(bytes.len() > 4000, "expected a wide residual, got {}", bytes.len());
        round_trip(&pcm);
    }

    fn stereo_pcm(n: usize) -> Pcm {
        let mut samples = Vec::with_capacity(n * 2);
        for i in 0..n {
            let ramp = ((i % 256) as i32) - 128;
            let slow = ((i / 64) % 512) as i32 - 256;
            let wobble = ((i.wrapping_mul(i) >> 12) & 63) as i32;
            let left = (slow * 40 + ramp * 8 + wobble).clamp(-32_768, 32_767);
            let right = (ramp * 40 + slow * 8 - wobble).clamp(-32_768, 32_767);
            samples.push(left);
            samples.push(right);
        }
        Pcm {
            sample_rate: 44_100,
            channels: 2,
            bits: 16,
            samples,
        }
    }

    fn stereo_block() -> Pcm {
        stereo_pcm(4096)
    }

    fn aligned_stereo() -> Pcm {
        let n = 4096usize;
        let mut samples = Vec::with_capacity(n * 2);
        for i in 0..n {
            let ramp = ((i % 256) as i32) - 128;
            let slow = ((i / 64) % 512) as i32 - 256;
            let wobble = ((i.wrapping_mul(i) >> 12) & 63) as i32;
            let left = (slow * 40 + ramp * 8 + wobble).clamp(-32_768, 32_767) & !3;
            let right = (ramp * 40 + slow * 8 - wobble).clamp(-32_768, 32_767) & !3;
            samples.push(left);
            samples.push(right);
        }
        Pcm {
            sample_rate: 44_100,
            channels: 2,
            bits: 16,
            samples,
        }
    }

    #[test]
    fn aligned_stereo_writes_wasted_bits_on_both_channels() {
        let pcm = aligned_stereo();
        assert_eq!(pcm.samples.len(), 8192);
        let mut left = Vec::with_capacity(4096);
        let mut right = Vec::with_capacity(4096);
        for i in 0..4096 {
            let l = pcm.samples[i * 2];
            let r = pcm.samples[i * 2 + 1];
            assert_eq!(l % 4, 0);
            assert_eq!(r % 4, 0);
            left.push(l);
            right.push(r);
        }
        assert!(left.iter().any(|s| s % 8 != 0));
        assert!(right.iter().any(|s| s % 8 != 0));
        assert_ne!(left, right);
        let bytes = encode(&pcm).unwrap();
        assert!(
            bytes.len() < 7_601,
            "aligned stereo was not shorter than 7601: {}",
            bytes.len()
        );
        let (left_wasted, right_wasted) = both_subframe_wasted(&bytes);
        assert_eq!(left_wasted, 2);
        assert_eq!(right_wasted, 2);
        let back = decode(&bytes).unwrap();
        assert_eq!(back.channels, 2);
        assert_eq!(back.bits, 16);
        assert_eq!(back.sample_rate, 44_100);
        assert_eq!(back.samples, pcm.samples);
    }

    fn both_subframe_wasted(bytes: &[u8]) -> (u32, u32) {
        assert_eq!(&bytes[0..4], b"fLaC");
        assert_eq!(bytes[42], 0xff);
        assert_eq!(bytes[43], 0xf8);
        assert_eq!(bytes[45], 0x18);
        let mut br = BitReader::at(bytes, 48);
        let (kind, left) = read_subframe_header(&mut br).unwrap();
        let _ = read_subframe_body(&mut br, 4096, 16, kind, left).unwrap();
        let (kind, right) = read_subframe_header(&mut br).unwrap();
        let _ = read_subframe_body(&mut br, 4096, 16, kind, right).unwrap();
        (left, right)
    }

    fn aligned_block() -> Pcm {
        let n = 4096usize;
        let mut samples = Vec::with_capacity(n);
        for i in 0..n {
            let ramp = ((i % 256) as i32) - 128;
            let slow = ((i / 64) % 512) as i32 - 256;
            let wobble = ((i.wrapping_mul(i) >> 12) & 63) as i32;
            let sample = (slow * 40 + ramp * 8 + wobble).clamp(-32_768, 32_767);
            samples.push(sample & !3);
        }
        Pcm {
            sample_rate: 44_100,
            channels: 1,
            bits: 16,
            samples,
        }
    }

    #[test]
    fn aligned_block_round_trips() {
        let pcm = aligned_block();
        assert_eq!(pcm.samples.len(), 4096);
        assert!(pcm.samples.iter().all(|s| s % 4 == 0));
        assert!(pcm.samples.iter().any(|s| s % 8 != 0));
        let bytes = encode(&pcm).unwrap();
        assert_eq!(bytes.len(), 2_215);
        assert_eq!(first_subframe_wasted(&bytes), 2);
        let back = decode(&bytes).unwrap();
        assert_eq!(back.channels, 1);
        assert_eq!(back.bits, 16);
        assert_eq!(back.sample_rate, 44_100);
        assert_eq!(back.samples, pcm.samples);
    }

    fn first_subframe_wasted(bytes: &[u8]) -> u32 {
        assert_eq!(&bytes[0..4], b"fLaC");
        assert_eq!(bytes[42], 0xff);
        assert_eq!(bytes[43], 0xf8);
        let mut br = BitReader::at(bytes, 48);
        assert_eq!(br.read_bits(1).unwrap(), 0);
        let _kind = br.read_bits(6).unwrap();
        assert_eq!(br.read_bits(1).unwrap(), 1, "wasted-bits flag");
        br.read_unary().unwrap() + 1
    }

    fn mono_wide() -> Pcm {
        let n = 4096usize;
        let mut samples = Vec::with_capacity(n);
        for i in 0..n {
            let ramp = ((i % 256) as i32) - 128;
            let slow = ((i / 64) % 512) as i32 - 256;
            let wobble = ((i.wrapping_mul(i) >> 12) & 63) as i32;
            let narrow = (slow * 40 + ramp * 8 + wobble).clamp(-32_768, 32_767);
            let low = (i as i32) & 255;
            samples.push((narrow << 8) + low);
        }
        Pcm {
            sample_rate: 44_100,
            channels: 1,
            bits: 24,
            samples,
        }
    }

    fn stereo_wide() -> Pcm {
        let n = 4096usize;
        let mut samples = Vec::with_capacity(n * 2);
        for i in 0..n {
            let ramp = ((i % 256) as i32) - 128;
            let slow = ((i / 64) % 512) as i32 - 256;
            let wobble = ((i.wrapping_mul(i) >> 12) & 63) as i32;
            let left = (slow * 40 + ramp * 8 + wobble).clamp(-32_768, 32_767);
            let right = (ramp * 40 + slow * 8 - wobble).clamp(-32_768, 32_767);
            let low = (i as i32) & 255;
            samples.push((left << 8) + low);
            samples.push((right << 8) + (255 - low));
        }
        Pcm {
            sample_rate: 44_100,
            channels: 2,
            bits: 24,
            samples,
        }
    }

    #[test]
    fn stereo_wide_block_round_trips_both_channels() {
        let pcm = stereo_wide();
        assert_eq!(pcm.samples.len(), 8192);
        assert_eq!(pcm.sample_rate, 44_100);
        let mut wider_than_16 = false;
        for i in 0..4096 {
            let left = pcm.samples[i * 2];
            let right = pcm.samples[i * 2 + 1];
            assert_ne!(left, right);
            assert!(sample_fits(left, 24) && sample_fits(right, 24));
            if !sample_fits(left, 16) || !sample_fits(right, 16) {
                wider_than_16 = true;
            }
        }
        assert!(wider_than_16);
        let bytes = encode(&pcm).unwrap();
        assert_eq!(bytes.len(), 16_025);
        let back = decode(&bytes).unwrap();
        assert_eq!(back.channels, 2);
        assert_eq!(back.bits, 24);
        assert_eq!(back.sample_rate, 44_100);
        assert_eq!(back.samples, pcm.samples);
    }

    #[test]
    fn mono_wide_block_round_trips() {
        let pcm = mono_wide();
        assert_eq!(pcm.samples.len(), 4096);
        assert!(pcm.samples.iter().any(|s| !sample_fits(*s, 16)));
        assert!(pcm.samples.iter().all(|s| sample_fits(*s, 24)));
        let bytes = encode(&pcm).unwrap();
        let back = decode(&bytes).unwrap();
        assert_eq!(back.channels, 1);
        assert_eq!(back.bits, 24);
        assert_eq!(back.sample_rate, 44_100);
        assert_eq!(back.samples, pcm.samples);
    }

    #[test]
    fn stereo_block_round_trips_both_channels() {
        let pcm = stereo_block();
        assert_eq!(pcm.samples.len(), 8192);
        let bytes = encode(&pcm).unwrap();
        let back = decode(&bytes).unwrap();
        assert_eq!(back.channels, 2);
        assert_eq!(back.bits, 16);
        assert_eq!(back.sample_rate, 44_100);
        assert_eq!(back.samples.len(), pcm.samples.len());
        let mut left = Vec::with_capacity(4096);
        let mut right = Vec::with_capacity(4096);
        let mut back_left = Vec::with_capacity(4096);
        let mut back_right = Vec::with_capacity(4096);
        for i in 0..4096 {
            left.push(pcm.samples[i * 2]);
            right.push(pcm.samples[i * 2 + 1]);
            back_left.push(back.samples[i * 2]);
            back_right.push(back.samples[i * 2 + 1]);
        }
        assert_eq!(back_left, left);
        assert_eq!(back_right, right);
        assert_ne!(left, right);
    }

    #[test]
    fn stereo_second_round_trips_both_channels() {
        let pcm = stereo_pcm(44_100);
        assert_eq!(pcm.samples.len(), 88_200);
        let bytes = encode(&pcm).unwrap();
        assert_eq!(bytes.len(), 87_272);
        let back = decode(&bytes).unwrap();
        assert_eq!(back.channels, 2);
        assert_eq!(back.bits, 16);
        assert_eq!(back.sample_rate, 44_100);
        let mut left = Vec::with_capacity(44_100);
        let mut right = Vec::with_capacity(44_100);
        for i in 0..44_100 {
            assert_eq!(back.samples[i * 2], pcm.samples[i * 2]);
            assert_eq!(back.samples[i * 2 + 1], pcm.samples[i * 2 + 1]);
            left.push(pcm.samples[i * 2]);
            right.push(pcm.samples[i * 2 + 1]);
        }
        assert_ne!(left, right);
    }

    #[test]
    fn stereo_second_seek_table_one_point_per_block() {
        let pcm = stereo_pcm(44_100);
        let plain = encode(&pcm).unwrap();
        assert_eq!(plain.len(), 87_272);
        let bytes = encode_with_seek_table(&pcm).unwrap();
        assert_eq!(bytes.len(), 87_474);
        let back = decode(&bytes).unwrap();
        assert_eq!(back.channels, 2);
        assert_eq!(back.bits, 16);
        assert_eq!(back.sample_rate, 44_100);
        let mut left = Vec::with_capacity(44_100);
        let mut right = Vec::with_capacity(44_100);
        for i in 0..44_100 {
            assert_eq!(back.samples[i * 2], pcm.samples[i * 2]);
            assert_eq!(back.samples[i * 2 + 1], pcm.samples[i * 2 + 1]);
            left.push(pcm.samples[i * 2]);
            right.push(pcm.samples[i * 2 + 1]);
        }
        assert_ne!(left, right);
        let points = seek_points(&bytes).unwrap();
        assert_eq!(points.len(), 11);
        let mut sample = 0u64;
        let mut offset = 0u64;
        let audio = seek_audio_start(&bytes).unwrap();
        for (i, (sample_n, off, count)) in points.iter().copied().enumerate() {
            assert_eq!(sample_n, sample);
            assert_eq!(off, offset);
            let next = if i + 1 < points.len() {
                audio + points[i + 1].1 as usize
            } else {
                bytes.len()
            };
            let frame_len = next - (audio + off as usize);
            offset += frame_len as u64;
            sample += u64::from(count);
        }
        assert_eq!(sample, 44_100);
        assert_eq!(offset, (bytes.len() - audio) as u64);
    }

    fn mono_pcm(n: usize) -> Pcm {
        let mut samples = Vec::with_capacity(n);
        for i in 0..n {
            let ramp = ((i % 256) as i32) - 128;
            let slow = ((i / 64) % 512) as i32 - 256;
            let wobble = ((i.wrapping_mul(i) >> 12) & 63) as i32;
            samples.push((slow * 40 + ramp * 8 + wobble).clamp(-32_768, 32_767));
        }
        Pcm {
            sample_rate: 44_100,
            channels: 1,
            bits: 16,
            samples,
        }
    }

    #[test]
    fn mono_long_seek_table_one_point_per_block() {
        let pcm = mono_pcm(1_048_576);
        let plain = encode(&pcm).unwrap();
        assert_eq!(plain.len(), 948_202);
        let bytes = encode_with_seek_table(&pcm).unwrap();
        assert_eq!(bytes.len(), 952_814);
        let back = decode(&bytes).unwrap();
        assert_eq!(back.channels, 1);
        assert_eq!(back.bits, 16);
        assert_eq!(back.sample_rate, 44_100);
        assert_eq!(back.samples, pcm.samples);
        let points = seek_points(&bytes).unwrap();
        assert_eq!(points.len(), 256);
        let mut sample = 0u64;
        let mut offset = 0u64;
        let audio = seek_audio_start(&bytes).unwrap();
        for (i, (sample_n, off, count)) in points.iter().copied().enumerate() {
            assert_eq!(sample_n, sample);
            assert_eq!(off, offset);
            assert_eq!(count, 4096);
            let next = if i + 1 < points.len() {
                audio + points[i + 1].1 as usize
            } else {
                bytes.len()
            };
            let frame_len = next - (audio + off as usize);
            offset += frame_len as u64;
            sample += u64::from(count);
        }
        assert_eq!(sample, 1_048_576);
        assert_eq!(offset, (bytes.len() - audio) as u64);
    }

    fn seek_audio_start(bytes: &[u8]) -> Option<usize> {
        if bytes.len() < 8 || &bytes[0..4] != b"fLaC" {
            return None;
        }
        let mut pos = 4usize;
        loop {
            if pos + 4 > bytes.len() {
                return None;
            }
            let last = bytes[pos] & 0x80 != 0;
            let len = (usize::from(bytes[pos + 1]) << 16)
                | (usize::from(bytes[pos + 2]) << 8)
                | usize::from(bytes[pos + 3]);
            pos += 4 + len;
            if last {
                return Some(pos);
            }
        }
    }

    fn seek_points(bytes: &[u8]) -> Option<Vec<(u64, u64, u16)>> {
        if bytes.len() < 42 || &bytes[0..4] != b"fLaC" || bytes[4] & 0x80 != 0 {
            return None;
        }
        let pos = 42usize;
        if bytes[pos] != 0x83 {
            return None;
        }
        let len = (usize::from(bytes[pos + 1]) << 16)
            | (usize::from(bytes[pos + 2]) << 8)
            | usize::from(bytes[pos + 3]);
        if len % 18 != 0 || pos + 4 + len > bytes.len() {
            return None;
        }
        let mut out = Vec::with_capacity(len / 18);
        let mut at = pos + 4;
        while at < pos + 4 + len {
            let sample = u64::from_be_bytes(bytes[at..at + 8].try_into().ok()?);
            let offset = u64::from_be_bytes(bytes[at + 8..at + 16].try_into().ok()?);
            let count = u16::from_be_bytes(bytes[at + 16..at + 18].try_into().ok()?);
            out.push((sample, offset, count));
            at += 18;
        }
        Some(out)
    }

    #[test]
    fn stereo_eight_and_twentyfour() {
        let stereo: Vec<i32> = (0..8000)
            .map(|i| {
                if i % 2 == 0 {
                    (i / 2) % 200 - 100
                } else {
                    1000 - (i / 2) % 50
                }
            })
            .collect();
        round_trip(&Pcm {
            sample_rate: 44_100,
            channels: 2,
            bits: 16,
            samples: stereo,
        });
        round_trip(&Pcm {
            sample_rate: 8_000,
            channels: 1,
            bits: 8,
            samples: (0..64).map(|i| (i % 17) - 8).collect(),
        });
        round_trip(&Pcm {
            sample_rate: 96_000,
            channels: 1,
            bits: 24,
            samples: (0..64).map(|i| i * 1000 - 30_000).collect(),
        });
    }

    #[test]
    fn many_frames_utf8_frame_number() {
        let samples: Vec<i32> = (0..3200).map(|i| (i % 64) - 32).collect();
        let pcm = Pcm {
            sample_rate: 16_000,
            channels: 1,
            bits: 16,
            samples,
        };
        let bytes = encode_with(&pcm, 16, false).unwrap();
        let back = decode(&bytes).unwrap();
        assert_eq!(back.samples, pcm.samples);
    }

    #[test]
    fn rice_values_survive_both_readers() {
        let values = [-4000, -2, -1, 0, 1, 2, 40, 9_000];
        for param in [0u32, 1, 4, 7, 14] {
            let mut w = BitWriter::new();
            for &v in &values {
                write_rice(&mut w, v, param);
            }
            let bytes = w.finish();
            let mut br = BitReader::at(&bytes, 0);
            for &v in &values {
                assert_eq!(read_rice(&mut br, param).unwrap(), v, "param {param}");
            }
        }
    }

    #[test]
    fn escape_residual_round_trip() {
        let samples: Vec<i32> = (0..32).map(|i| if i % 2 == 0 { 30_000 } else { -30_000 }).collect();
        round_trip(&Pcm {
            sample_rate: 44_100,
            channels: 1,
            bits: 16,
            samples,
        });
    }

    #[test]
    fn truncated_and_flipped_crc() {
        let pcm = Pcm {
            sample_rate: 44_100,
            channels: 1,
            bits: 16,
            samples: (0..64).map(|i| i - 32).collect(),
        };
        let bytes = encode(&pcm).unwrap();
        let mut cut = bytes.clone();
        cut.pop();
        assert!(matches!(
            decode(&cut),
            Err(Error::Truncated { .. }) | Err(Error::Corrupt(_))
        ));
        let mut half = bytes.clone();
        half.truncate(half.len() / 2);
        assert!(decode(&half).is_err());
        let mut flip = bytes.clone();
        let last = flip.len() - 1;
        flip[last] ^= 0xff;
        assert!(matches!(decode(&flip), Err(Error::Corrupt("crc16"))));
        let mut mid = bytes.clone();
        let at = 50.min(mid.len() - 3);
        mid[at] ^= 0x01;
        assert!(decode(&mid).is_err());
    }

    #[test]
    fn hand_built_constant_verbatim_and_fixed() {
        let constant = build_mono(16, &[7; 16], |w, _| {
            write_sub_header(w, 0, 0);
            w.write_signed(7, 16);
        });
        assert_eq!(decode(&constant).unwrap().samples, vec![7; 16]);

        let verb: Vec<i32> = (0..16).map(|i| i - 8).collect();
        let verbatim = build_mono(16, &verb, |w, samples| {
            write_sub_header(w, 1, 0);
            for &s in samples {
                w.write_signed(s, 16);
            }
        });
        assert_eq!(decode(&verbatim).unwrap().samples, verb);

        let fixed: Vec<i32> = (0..16).collect();
        let framed = build_mono(16, &fixed, |w, samples| {
            write_sub_header(w, 9, 0);
            w.write_signed(samples[0], 16);
            let residual = fixed_residual(samples, 1).unwrap();
            write_residual(w, &residual, 0, None);
        });
        assert_eq!(decode(&framed).unwrap().samples, fixed);

        let linear: Vec<i32> = (0..16).collect();
        let lpc = build_mono(16, &linear, |w, samples| {
            // LPC order 2, coeffs 4 and -2, shift 1, same as fixed order 2.
            write_sub_header(w, 33, 0);
            w.write_signed(samples[0], 16);
            w.write_signed(samples[1], 16);
            w.write_bits(3, 4);
            w.write_bits(1, 5);
            w.write_signed(4, 4);
            w.write_signed(-2, 4);
            write_residual(w, &[0; 14], 0, None);
        });
        assert_eq!(decode(&lpc).unwrap().samples, linear);
    }

    fn build_mono(block: u16, samples: &[i32], body: impl Fn(&mut BitWriter, &[i32])) -> Vec<u8> {
        let pcm = Pcm {
            sample_rate: 44_100,
            channels: 1,
            bits: 16,
            samples: samples.to_vec(),
        };
        let md5 = audio_md5(&pcm);
        let mut hdr = vec![0xff, 0xf8];
        let (bs, extra) = block_size_code(block);
        hdr.push((bs << 4) | 9);
        hdr.push(4 << 1);
        hdr.push(0);
        if let Some(v) = extra {
            if bs == 6 {
                hdr.push(v as u8);
            } else {
                hdr.extend_from_slice(&v.to_be_bytes());
            }
        }
        let c = crc8(&hdr);
        hdr.push(c);
        let mut w = BitWriter::new();
        body(&mut w, samples);
        let mut frame = hdr;
        frame.extend_from_slice(&w.finish());
        let sum = crc16(&frame);
        frame.extend_from_slice(&sum.to_be_bytes());
        let info = write_streaminfo(block, block, frame.len() as u32, frame.len() as u32, 44_100, 1, 16, samples.len() as u64, md5);
        let mut out = Vec::new();
        out.extend_from_slice(b"fLaC");
        out.push(0x80);
        out.extend_from_slice(&34u32.to_be_bytes()[1..]);
        out.extend_from_slice(&info);
        out.extend_from_slice(&frame);
        out
    }
}
