use crate::bits::BitWriter;
use crate::huff::{
    canonical, distance_code, fixed_dist_lengths, fixed_lit_lengths, length_code,
    lengths_from_freqs, Code, CL_ORDER,
};
use crate::lz::{lz77, Tok};

/// How a block is coded. `Auto` picks the smallest of the three for each chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Auto,
    Stored,
    Fixed,
    Dynamic,
}

const BLOCK: usize = 256 * 1024;

pub fn compress(data: &[u8]) -> Vec<u8> {
    compress_with(data, Method::Auto)
}

pub fn compress_with(data: &[u8], method: Method) -> Vec<u8> {
    let mut writer = BitWriter::new();
    if data.is_empty() {
        emit_block(&mut writer, data, method, true);
        return writer.finish();
    }
    let mut offset = 0;
    while offset < data.len() {
        let end = (offset + BLOCK).min(data.len());
        let last = end == data.len();
        emit_block(&mut writer, &data[offset..end], method, last);
        offset = end;
    }
    writer.finish()
}

fn emit_block(writer: &mut BitWriter, data: &[u8], method: Method, last: bool) {
    match method {
        Method::Stored => emit_stored(writer, data, last),
        Method::Fixed => {
            let toks = lz77(data);
            emit_fixed(writer, &toks, last);
        }
        Method::Dynamic => {
            let toks = lz77(data);
            emit_dynamic(writer, &toks, last);
        }
        Method::Auto => {
            let offset = writer.pending() % 8;
            let toks = lz77(data);
            let stored_cost = cost(offset, |w| emit_stored(w, data, last));
            let fixed_cost = cost(offset, |w| emit_fixed(w, &toks, last));
            let dynamic_cost = cost(offset, |w| emit_dynamic(w, &toks, last));
            if stored_cost <= fixed_cost && stored_cost <= dynamic_cost {
                emit_stored(writer, data, last);
            } else if fixed_cost <= dynamic_cost {
                emit_fixed(writer, &toks, last);
            } else {
                emit_dynamic(writer, &toks, last);
            }
        }
    }
}

fn cost(offset: u32, emit: impl FnOnce(&mut BitWriter)) -> usize {
    let mut scratch = BitWriter::with_bit_offset(offset);
    let before = scratch.bit_len();
    emit(&mut scratch);
    scratch.bit_len() - before
}

fn emit_stored(writer: &mut BitWriter, data: &[u8], last: bool) {
    if data.is_empty() {
        write_stored(writer, data, true);
        return;
    }
    let mut offset = 0;
    while offset < data.len() {
        let n = (data.len() - offset).min(65535);
        let sub_last = last && offset + n == data.len();
        write_stored(writer, &data[offset..offset + n], sub_last);
        offset += n;
    }
}

fn write_stored(writer: &mut BitWriter, data: &[u8], last: bool) {
    writer.write_bits(u32::from(last), 1);
    writer.write_bits(0, 2);
    writer.align();
    let n = data.len() as u16;
    writer.write_bits(u32::from(n), 16);
    writer.write_bits(u32::from(!n), 16);
    writer.write_bytes(data);
}

fn emit_fixed(writer: &mut BitWriter, toks: &[Tok], last: bool) {
    let lit = canonical(&fixed_lit_lengths());
    let dist = canonical(&fixed_dist_lengths());
    writer.write_bits(u32::from(last), 1);
    writer.write_bits(1, 2);
    emit_tokens(writer, toks, &lit, &dist);
}

fn emit_dynamic(writer: &mut BitWriter, toks: &[Tok], last: bool) {
    let (lit_freq, dist_freq) = frequencies(toks);
    let lit_len = lengths_from_freqs(&lit_freq, 15);
    let dist_len = lengths_from_freqs(&dist_freq, 15);
    let nlit = trim_end(&lit_len, 257);
    let ndist = trim_end(&dist_len, 1);
    let mut all = Vec::with_capacity(nlit + ndist);
    all.extend_from_slice(&lit_len[..nlit]);
    all.extend_from_slice(&dist_len[..ndist]);
    let rle = rle_lengths(&all);
    let mut cl_freq = [0u32; 19];
    for tok in &rle {
        cl_freq[tok.sym as usize] += 1;
    }
    let cl_len = lengths_from_freqs(&cl_freq, 7);
    let mut hclen = 19usize;
    while hclen > 4 && cl_len[CL_ORDER[hclen - 1]] == 0 {
        hclen -= 1;
    }
    let cl_codes = canonical(&cl_len);
    let lit_codes = canonical(&lit_len);
    let dist_codes = canonical(&dist_len);

    writer.write_bits(u32::from(last), 1);
    writer.write_bits(2, 2);
    writer.write_bits((nlit - 257) as u32, 5);
    writer.write_bits((ndist - 1) as u32, 5);
    writer.write_bits((hclen - 4) as u32, 4);
    for i in 0..hclen {
        writer.write_bits(u32::from(cl_len[CL_ORDER[i]]), 3);
    }
    for tok in &rle {
        write_code(writer, &cl_codes[tok.sym as usize]);
        if tok.extra_bits > 0 {
            writer.write_bits(u32::from(tok.extra), u32::from(tok.extra_bits));
        }
    }
    emit_tokens(writer, toks, &lit_codes, &dist_codes);
}

fn trim_end(lengths: &[u8], min_keep: usize) -> usize {
    let mut n = lengths.len();
    while n > min_keep && lengths[n - 1] == 0 {
        n -= 1;
    }
    n
}

struct Rle {
    sym: u8,
    extra: u16,
    extra_bits: u8,
}

fn rle_lengths(lengths: &[u8]) -> Vec<Rle> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lengths.len() {
        let value = lengths[i];
        let mut run = 1usize;
        if value == 0 {
            while i + run < lengths.len() && lengths[i + run] == 0 && run < 138 {
                run += 1;
            }
            let mut left = run;
            while left > 0 {
                if left < 3 {
                    for _ in 0..left {
                        out.push(Rle {
                            sym: 0,
                            extra: 0,
                            extra_bits: 0,
                        });
                    }
                    left = 0;
                } else if left <= 10 {
                    out.push(Rle {
                        sym: 17,
                        extra: (left - 3) as u16,
                        extra_bits: 3,
                    });
                    left = 0;
                } else {
                    let n = left.min(138);
                    out.push(Rle {
                        sym: 18,
                        extra: (n - 11) as u16,
                        extra_bits: 7,
                    });
                    left -= n;
                }
            }
        } else {
            while i + run < lengths.len() && lengths[i + run] == value {
                run += 1;
            }
            out.push(Rle {
                sym: value,
                extra: 0,
                extra_bits: 0,
            });
            let mut left = run - 1;
            while left > 0 {
                if left < 3 {
                    for _ in 0..left {
                        out.push(Rle {
                            sym: value,
                            extra: 0,
                            extra_bits: 0,
                        });
                    }
                    left = 0;
                } else {
                    let n = left.min(6);
                    out.push(Rle {
                        sym: 16,
                        extra: (n - 3) as u16,
                        extra_bits: 2,
                    });
                    left -= n;
                }
            }
        }
        i += run;
    }
    out
}

fn frequencies(toks: &[Tok]) -> ([u32; 286], [u32; 30]) {
    let mut lit = [0u32; 286];
    let mut dist = [0u32; 30];
    lit[256] = 1;
    for tok in toks {
        match *tok {
            Tok::Lit(byte) => lit[byte as usize] += 1,
            Tok::Match {
                len,
                dist: distance,
            } => {
                let (sym, _, _) = length_code(len as usize);
                lit[sym as usize] += 1;
                let (dsym, _, _) = distance_code(distance as usize);
                dist[dsym as usize] += 1;
            }
        }
    }
    (lit, dist)
}

fn emit_tokens(writer: &mut BitWriter, toks: &[Tok], lit: &[Code], dist: &[Code]) {
    for tok in toks {
        match *tok {
            Tok::Lit(byte) => write_code(writer, &lit[byte as usize]),
            Tok::Match {
                len,
                dist: distance,
            } => {
                let (sym, extra_bits, extra) = length_code(len as usize);
                write_code(writer, &lit[sym as usize]);
                if extra_bits > 0 {
                    writer.write_bits(extra, extra_bits);
                }
                let (dsym, dextra_bits, dextra) = distance_code(distance as usize);
                write_code(writer, &dist[dsym as usize]);
                if dextra_bits > 0 {
                    writer.write_bits(dextra, dextra_bits);
                }
            }
        }
    }
    write_code(writer, &lit[256]);
}

fn write_code(writer: &mut BitWriter, code: &Code) {
    debug_assert!(code.len > 0, "used symbol has no Huffman code");
    if code.len > 0 {
        writer.write_bits(u32::from(code.rev), u32::from(code.len));
    }
}
