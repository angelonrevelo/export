/// Literal/length bases for symbols 257..=285.
pub const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];

pub const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];

pub const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];

pub const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

pub const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

#[derive(Clone, Copy)]
pub struct Code {
    pub rev: u16,
    pub len: u8,
}

pub fn fixed_lit_lengths() -> [u8; 288] {
    let mut lengths = [0u8; 288];
    for item in &mut lengths[..144] {
        *item = 8;
    }
    for item in &mut lengths[144..256] {
        *item = 9;
    }
    for item in &mut lengths[256..280] {
        *item = 7;
    }
    for item in &mut lengths[280..] {
        *item = 8;
    }
    lengths
}

pub fn fixed_dist_lengths() -> [u8; 32] {
    [5u8; 32]
}

/// `(symbol, extra_bits, extra_value)` for a match length of 3..=258.
pub fn length_code(len: usize) -> (u16, u32, u32) {
    debug_assert!((3..=258).contains(&len));
    let mut i = 0;
    while i + 1 < LEN_BASE.len() && LEN_BASE[i + 1] as usize <= len {
        i += 1;
    }
    let extra_bits = u32::from(LEN_EXTRA[i]);
    let extra = (len - LEN_BASE[i] as usize) as u32;
    (257 + i as u16, extra_bits, extra)
}

/// `(symbol, extra_bits, extra_value)` for a distance of 1..=32768.
pub fn distance_code(dist: usize) -> (u16, u32, u32) {
    debug_assert!((1..=32768).contains(&dist));
    let mut i = 0;
    while i + 1 < DIST_BASE.len() && DIST_BASE[i + 1] as usize <= dist {
        i += 1;
    }
    let extra_bits = u32::from(DIST_EXTRA[i]);
    let extra = (dist - DIST_BASE[i] as usize) as u32;
    (i as u16, extra_bits, extra)
}

pub fn canonical(lengths: &[u8]) -> Vec<Code> {
    let mut bl_count = [0u16; 16];
    for &len in lengths {
        if len > 0 {
            bl_count[len as usize] += 1;
        }
    }
    let mut next = [0u32; 16];
    let mut code = 0u32;
    for bits in 1..=15 {
        code = (code + u32::from(bl_count[bits - 1])) << 1;
        next[bits] = code;
    }
    let mut out = vec![Code { rev: 0, len: 0 }; lengths.len()];
    for (sym, &len) in lengths.iter().enumerate() {
        if len == 0 {
            continue;
        }
        let assigned = next[len as usize];
        next[len as usize] = assigned + 1;
        out[sym] = Code {
            rev: reverse_bits(assigned as u16, len),
            len,
        };
    }
    out
}

fn reverse_bits(mut code: u16, len: u8) -> u16 {
    let mut rev = 0u16;
    for _ in 0..len {
        rev = (rev << 1) | (code & 1);
        code >>= 1;
    }
    rev
}

pub fn lengths_from_freqs(freqs: &[u32], max_len: u8) -> Vec<u8> {
    let raw = unlimited_lengths(freqs);
    if raw.iter().all(|&len| len <= u16::from(max_len)) && covers(freqs, &raw) {
        let lens: Vec<u8> = raw.iter().map(|&len| len as u8).collect();
        if kraft_ok(&lens) {
            return lens;
        }
    }
    if let Some(lens) = limit_lengths(freqs, &raw, max_len) {
        if kraft_ok(&lens)
            && lens
                .iter()
                .zip(freqs)
                .all(|(&len, &freq)| freq == 0 || len > 0)
        {
            return lens;
        }
    }
    flat_lengths(freqs, max_len)
}

fn covers(freqs: &[u32], lens: &[u16]) -> bool {
    freqs
        .iter()
        .zip(lens)
        .all(|(&freq, &len)| freq == 0 || len > 0)
}

pub fn kraft_ok(lengths: &[u8]) -> bool {
    let mut left = 1i32;
    let mut counts = [0i32; 16];
    for &len in lengths {
        if len > 15 {
            return false;
        }
        if len > 0 {
            counts[len as usize] += 1;
        }
    }
    for count in counts.iter().skip(1) {
        left <<= 1;
        left -= count;
        if left < 0 {
            return false;
        }
    }
    true
}

struct Node {
    freq: u32,
    sym: i32,
    left: i32,
    right: i32,
}

fn unlimited_lengths(freqs: &[u32]) -> Vec<u16> {
    let mut nodes = Vec::new();
    let mut heap = Vec::new();
    for (sym, &freq) in freqs.iter().enumerate() {
        if freq > 0 {
            nodes.push(Node {
                freq,
                sym: sym as i32,
                left: -1,
                right: -1,
            });
            heap.push(nodes.len() - 1);
            let tip = heap.len() - 1;
            sift_up(&nodes, &mut heap, tip);
        }
    }
    let mut lens = vec![0u16; freqs.len()];
    if heap.is_empty() {
        return lens;
    }
    if heap.len() == 1 {
        lens[nodes[heap[0]].sym as usize] = 1;
        return lens;
    }
    while heap.len() > 1 {
        let left = pop(&nodes, &mut heap);
        let right = pop(&nodes, &mut heap);
        let freq = nodes[left].freq.saturating_add(nodes[right].freq);
        nodes.push(Node {
            freq,
            sym: -1,
            left: left as i32,
            right: right as i32,
        });
        heap.push(nodes.len() - 1);
        let tip = heap.len() - 1;
        sift_up(&nodes, &mut heap, tip);
    }
    walk(&nodes, heap[0], 0, &mut lens);
    lens
}

fn walk(nodes: &[Node], idx: usize, depth: u16, lens: &mut [u16]) {
    let node = &nodes[idx];
    if node.sym >= 0 {
        lens[node.sym as usize] = depth.max(1);
        return;
    }
    walk(nodes, node.left as usize, depth + 1, lens);
    walk(nodes, node.right as usize, depth + 1, lens);
}

fn less(nodes: &[Node], a: usize, b: usize) -> bool {
    nodes[a].freq < nodes[b].freq || (nodes[a].freq == nodes[b].freq && a < b)
}

fn sift_up(nodes: &[Node], heap: &mut [usize], mut index: usize) {
    while index > 0 {
        let parent = (index - 1) / 2;
        if less(nodes, heap[index], heap[parent]) {
            heap.swap(index, parent);
            index = parent;
        } else {
            break;
        }
    }
}

fn pop(nodes: &[Node], heap: &mut Vec<usize>) -> usize {
    let out = heap[0];
    let last = heap.pop().expect("heap");
    if !heap.is_empty() {
        heap[0] = last;
        sift_down(nodes, heap);
    }
    out
}

fn sift_down(nodes: &[Node], heap: &mut [usize]) {
    let mut index = 0;
    loop {
        let left = index * 2 + 1;
        let right = left + 1;
        let mut best = index;
        if left < heap.len() && less(nodes, heap[left], heap[best]) {
            best = left;
        }
        if right < heap.len() && less(nodes, heap[right], heap[best]) {
            best = right;
        }
        if best == index {
            break;
        }
        heap.swap(index, best);
        index = best;
    }
}

fn limit_lengths(freqs: &[u32], raw: &[u16], max_len: u8) -> Option<Vec<u8>> {
    let max_len = usize::from(max_len);
    if max_len == 0 || max_len > 15 {
        return None;
    }
    let mut bl_count = vec![0i32; max_len + 1];
    let mut overflow = 0i32;
    let mut used = 0i32;
    for &len in raw {
        if len == 0 {
            continue;
        }
        used += 1;
        if usize::from(len) > max_len {
            overflow += 1;
        } else {
            bl_count[usize::from(len)] += 1;
        }
    }
    if used == 0 {
        return Some(vec![0; freqs.len()]);
    }
    bl_count[max_len] += overflow;
    let mut guard = 0;
    while overflow > 0 {
        guard += 1;
        if guard > 4096 {
            return None;
        }
        let mut bits = max_len - 1;
        while bits > 0 && bl_count[bits] == 0 {
            bits -= 1;
        }
        if bits == 0 {
            return None;
        }
        bl_count[bits] -= 1;
        bl_count[bits + 1] += 2;
        bl_count[max_len] -= 1;
        overflow -= 2;
    }
    let mut order: Vec<usize> = (0..freqs.len()).filter(|&i| freqs[i] > 0).collect();
    order.sort_by(|&a, &b| freqs[a].cmp(&freqs[b]).then(a.cmp(&b)));
    let mut out = vec![0u8; freqs.len()];
    let mut cursor = 0;
    for len in (1..=max_len).rev() {
        for _ in 0..bl_count[len] {
            if cursor >= order.len() {
                return None;
            }
            out[order[cursor]] = len as u8;
            cursor += 1;
        }
    }
    if cursor != order.len() {
        return None;
    }
    Some(out)
}

fn flat_lengths(freqs: &[u32], max_len: u8) -> Vec<u8> {
    let n = freqs.iter().filter(|&&freq| freq > 0).count();
    let mut len = 1u8;
    if n > 1 {
        while usize::from(len) < 15 && (1usize << len) < n && len < max_len {
            len += 1;
        }
    }
    let mut out = vec![0u8; freqs.len()];
    for (i, &freq) in freqs.iter().enumerate() {
        if freq > 0 {
            out[i] = len;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{distance_code, kraft_ok, length_code, lengths_from_freqs, DIST_BASE, LEN_BASE};

    #[test]
    fn length_and_distance_tables_round() {
        for len in 3..=258 {
            let (sym, extra_bits, extra) = length_code(len);
            assert!((257..=285).contains(&sym));
            assert_eq!(
                LEN_BASE[(sym - 257) as usize] as usize + extra as usize,
                len
            );
            if extra_bits == 0 {
                assert_eq!(extra, 0);
            } else {
                assert!(extra < (1u32 << extra_bits));
            }
        }
        for dist in 1..=32768 {
            let (sym, extra_bits, extra) = distance_code(dist);
            assert!((0..30).contains(&sym));
            assert_eq!(DIST_BASE[sym as usize] as usize + extra as usize, dist);
            if extra_bits == 0 {
                assert_eq!(extra, 0);
            } else {
                assert!(extra < (1u32 << extra_bits));
            }
        }
    }

    #[test]
    fn skewed_alphabet_stays_within_fifteen_bits() {
        let mut freqs = vec![1u32; 286];
        freqs[256] = 50_000;
        let lens = lengths_from_freqs(&freqs, 15);
        assert!(kraft_ok(&lens));
        assert!(lens.iter().all(|&len| (1..=15).contains(&len)));
        assert!(lens[256] <= lens[0]);
    }

    #[test]
    fn one_symbol_gets_one_bit() {
        let lens = lengths_from_freqs(&[0, 4, 0], 15);
        assert_eq!(lens, vec![0, 1, 0]);
        assert!(kraft_ok(&lens));
    }
}
