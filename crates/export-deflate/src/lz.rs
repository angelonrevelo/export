/// Hash every 4th byte inside a match. On the repeated-text fixture, hashing
/// every byte was about 535 MB/s and 3291 bytes; this stride is about 830 MB/s
/// and 3806 bytes. That gap is outside the run-to-run spread, so the stride stays.
const MATCH_INSERT_STRIDE: usize = 4;

const HASH_BITS: usize = 15;
const HASH_SIZE: usize = 1 << HASH_BITS;
const MAX_CHAIN: usize = 16;
const MAX_MATCH: usize = 258;
const MIN_MATCH: usize = 3;
pub const MAX_WINDOW: usize = 32_768;

#[derive(Clone, Copy)]
pub enum Tok {
    Lit(u8),
    Match { dist: u16, len: u16 },
}

pub fn lz77(data: &[u8]) -> Vec<Tok> {
    let mut out = Vec::new();
    if data.len() < MIN_MATCH {
        out.extend(data.iter().copied().map(Tok::Lit));
        return out;
    }
    let mut head = vec![u32::MAX; HASH_SIZE];
    let mut prev = vec![u32::MAX; data.len()];
    let mut i = 0;
    while i < data.len() {
        if i + MIN_MATCH <= data.len() {
            let hash = hash3(data[i], data[i + 1], data[i + 2]);
            if let Some((dist, len)) = find_match(data, i, head[hash], &prev) {
                let start = i;
                insert(data, i, &mut head, &mut prev);
                out.push(Tok::Match { dist, len });
                let end = i + len as usize;
                i += 1;
                while i < end {
                    if i - start >= MATCH_INSERT_STRIDE && (i - start) % MATCH_INSERT_STRIDE == 0 {
                        insert(data, i, &mut head, &mut prev);
                    }
                    i += 1;
                }
                continue;
            }
            insert(data, i, &mut head, &mut prev);
        }
        out.push(Tok::Lit(data[i]));
        i += 1;
    }
    out
}

fn hash3(a: u8, b: u8, c: u8) -> usize {
    let value = (u32::from(a) << 16) | (u32::from(b) << 8) | u32::from(c);
    (value.wrapping_mul(0x1E35_A7BD) >> (32 - HASH_BITS)) as usize
}

fn insert(data: &[u8], index: usize, head: &mut [u32], prev: &mut [u32]) {
    if index + 2 >= data.len() {
        return;
    }
    let hash = hash3(data[index], data[index + 1], data[index + 2]);
    prev[index] = head[hash];
    head[hash] = index as u32;
}

fn find_match(data: &[u8], index: usize, mut chain: u32, prev: &[u32]) -> Option<(u16, u16)> {
    let max_len = (data.len() - index).min(MAX_MATCH);
    if max_len < MIN_MATCH {
        return None;
    }
    let mut best_len = 0usize;
    let mut best_dist = 0usize;
    for _ in 0..MAX_CHAIN {
        if chain == u32::MAX {
            break;
        }
        let pos = chain as usize;
        if pos >= index {
            break;
        }
        let dist = index - pos;
        if dist > MAX_WINDOW {
            break;
        }
        let len = match_len(data, pos, index, max_len);
        if len > best_len {
            best_len = len;
            best_dist = dist;
            if best_len == MAX_MATCH {
                break;
            }
        }
        chain = prev[pos];
    }
    if best_len >= MIN_MATCH {
        Some((best_dist as u16, best_len as u16))
    } else {
        None
    }
}

fn match_len(data: &[u8], from: usize, to: usize, max: usize) -> usize {
    let mut n = 0;
    while n + 8 <= max {
        let left = load8(data, from + n);
        let right = load8(data, to + n);
        if left != right {
            let diff = (left ^ right).trailing_zeros() as usize;
            return n + diff / 8;
        }
        n += 8;
    }
    while n < max && data[from + n] == data[to + n] {
        n += 1;
    }
    n
}

fn load8(data: &[u8], index: usize) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[index..index + 8]);
    u64::from_le_bytes(buf)
}
