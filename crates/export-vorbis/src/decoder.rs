//! Decode the one short mono Vorbis packet this crate writes.

use crate::dsp::{self, unpack_vorbis_float};
use crate::{ogg_crc, Error, BLOCKSIZE, SAMPLE_RATE_HZ};

/// Decode one Ogg Vorbis bitstream that holds a single short mono audio packet.
pub fn decode_packet(bytes: &[u8]) -> Result<Vec<f32>, Error> {
    if bytes.is_empty() {
        return Err(Error::Truncated);
    }
    let packets = split_packets(bytes)?;
    if packets.is_empty() {
        return Err(Error::Truncated);
    }
    let id = parse_identification(&packets[0])?;
    if id.channels != 1 || id.sample_rate != SAMPLE_RATE_HZ {
        return Err(Error::Unsupported);
    }
    if packets.len() < 4 {
        return Err(Error::Truncated);
    }
    parse_comment(&packets[1])?;
    if packets.len() == 4 {
        let setup = parse_setup(&packets[2], &id)?;
        return decode_audio(&packets[3], &id, &setup);
    }
    if packets.len() != 5 && packets.len() != 6 {
        return Err(Error::BadHeader);
    }
    if id.blocksize_0 != BLOCKSIZE || id.blocksize_1 != BLOCKSIZE {
        return Err(Error::Unsupported);
    }
    let values = parse_tone_setup(&packets[2])?;
    let mut saved = [0.0f64; 16];
    let mut pcm = [0.0f32; dsp::BINS];
    for packet in &packets[3..] {
        let (frame, next) = decode_tone_frame(packet, &values, &saved)?;
        saved = next;
        pcm = frame;
    }
    Ok(pcm.to_vec())
}

struct Identification {
    channels: usize,
    sample_rate: u32,
    blocksize_0: usize,
    blocksize_1: usize,
}

fn parse_identification(packet: &[u8]) -> Result<Identification, Error> {
    if packet.len() < 30 {
        return Err(Error::Truncated);
    }
    if packet[0] != 1 || &packet[1..7] != b"vorbis" {
        return Err(Error::BadHeader);
    }
    if packet[7..11] != [0, 0, 0, 0] {
        return Err(Error::BadHeader);
    }
    let channels = packet[11] as usize;
    if channels == 0 {
        return Err(Error::BadHeader);
    }
    let sample_rate = u32::from_le_bytes([packet[12], packet[13], packet[14], packet[15]]);
    if sample_rate == 0 {
        return Err(Error::BadHeader);
    }
    let bs0 = (packet[28] & 0x0f) as usize;
    let bs1 = (packet[28] >> 4) as usize;
    if !(6..=13).contains(&bs0) || !(6..=13).contains(&bs1) || bs0 > bs1 {
        return Err(Error::BadHeader);
    }
    if packet[29] != 1 {
        return Err(Error::BadHeader);
    }
    Ok(Identification {
        channels,
        sample_rate,
        blocksize_0: 1usize << bs0,
        blocksize_1: 1usize << bs1,
    })
}

fn parse_comment(packet: &[u8]) -> Result<(), Error> {
    if packet.len() < 7 {
        return Err(Error::Truncated);
    }
    if packet[0] != 3 || &packet[1..7] != b"vorbis" {
        return Err(Error::BadHeader);
    }
    let mut r = BitReader::new(&packet[7..]);
    let vendor_len = r.read(32)? as usize;
    if vendor_len > 1024 {
        return Err(Error::BadHeader);
    }
    for _ in 0..vendor_len {
        let _ = r.read(8)?;
    }
    let comments = r.read(32)? as usize;
    if comments > 1024 {
        return Err(Error::BadHeader);
    }
    for _ in 0..comments {
        let len = r.read(32)? as usize;
        if len > 4096 {
            return Err(Error::BadHeader);
        }
        for _ in 0..len {
            let _ = r.read(8)?;
        }
    }
    let framing = r.read(1)?;
    if framing != 1 {
        return Err(Error::BadHeader);
    }
    Ok(())
}

struct Setup {
    modes: Vec<Mode>,
    mappings: Vec<Mapping>,
}

struct Mode {
    blockflag: u8,
    mapping: usize,
}

struct Mapping {
    coupling: bool,
}

fn parse_setup(packet: &[u8], id: &Identification) -> Result<Setup, Error> {
    if packet.len() < 7 {
        return Err(Error::Truncated);
    }
    if packet[0] != 5 || &packet[1..7] != b"vorbis" {
        return Err(Error::BadHeader);
    }
    let mut r = BitReader::new(&packet[7..]);

    let codebook_count = r.read(8)? as usize + 1;
    for _ in 0..codebook_count {
        read_codebook(&mut r)?;
    }

    let time_count = r.read(6)? as usize + 1;
    for _ in 0..time_count {
        if r.read(16)? != 0 {
            return Err(Error::BadHeader);
        }
    }

    let floor_count = r.read(6)? as usize + 1;
    let mut floors = Vec::with_capacity(floor_count);
    for _ in 0..floor_count {
        floors.push(read_floor(&mut r, codebook_count)?);
    }

    let residue_count = r.read(6)? as usize + 1;
    let mut residues = Vec::with_capacity(residue_count);
    for _ in 0..residue_count {
        residues.push(read_residue(&mut r, codebook_count)?);
    }

    let mapping_count = r.read(6)? as usize + 1;
    let mut mappings = Vec::with_capacity(mapping_count);
    for _ in 0..mapping_count {
        mappings.push(read_mapping(
            &mut r,
            id.channels,
            floor_count,
            residue_count,
        )?);
    }

    let mode_count = r.read(6)? as usize + 1;
    let mut modes = Vec::with_capacity(mode_count);
    for _ in 0..mode_count {
        let blockflag = r.read(1)? as u8;
        if r.read(16)? != 0 || r.read(16)? != 0 {
            return Err(Error::BadHeader);
        }
        let mapping = r.read(8)? as usize;
        if mapping >= mappings.len() {
            return Err(Error::BadHeader);
        }
        modes.push(Mode { blockflag, mapping });
    }
    if r.read(1)? != 1 {
        return Err(Error::BadHeader);
    }

    validate_setup(id, &modes, &mappings, &floors, &residues)?;
    let mappings = mappings
        .into_iter()
        .map(|mapping| Mapping {
            coupling: mapping.coupling,
        })
        .collect();
    Ok(Setup { modes, mappings })
}

fn read_codebook(r: &mut BitReader<'_>) -> Result<(), Error> {
    if r.read(24)? != 0x0056_4342 {
        return Err(Error::BadHeader);
    }
    let dimensions = r.read(16)? as usize;
    let entries = r.read(24)? as usize;
    if dimensions == 0 || dimensions > 64 || entries == 0 || entries > 4096 {
        return Err(Error::BadHeader);
    }
    let ordered = r.read(1)?;
    let mut lengths = vec![0u8; entries];
    if ordered == 0 {
        let sparse = r.read(1)?;
        for length in &mut lengths {
            if sparse == 1 && r.read(1)? == 0 {
                continue;
            }
            *length = r.read(5)? as u8 + 1;
        }
    } else {
        let mut current_len = r.read(5)? as usize + 1;
        let mut current_entry = 0usize;
        while current_entry < entries {
            if current_len > 32 {
                return Err(Error::BadHeader);
            }
            let bits = ilog(entries - current_entry);
            let number = r.read(bits)? as usize;
            if number == 0 || number > entries - current_entry {
                return Err(Error::BadHeader);
            }
            for slot in lengths.iter_mut().skip(current_entry).take(number) {
                *slot = current_len as u8;
            }
            current_entry += number;
            current_len += 1;
        }
    }
    assign_codewords(&lengths)?;

    let lookup = r.read(4)?;
    if lookup > 2 {
        return Err(Error::BadHeader);
    }
    if lookup >= 1 {
        let _minimum = r.read(32)?;
        let _delta = r.read(32)?;
        let value_bits = r.read(4)? as usize + 1;
        let _sequence_p = r.read(1)?;
        let count = if lookup == 1 {
            lookup1_values(entries, dimensions)?
        } else {
            entries.checked_mul(dimensions).ok_or(Error::BadHeader)?
        };
        if count > 8192 {
            return Err(Error::BadHeader);
        }
        for _ in 0..count {
            let _ = r.read(value_bits)?;
        }
    }
    Ok(())
}

fn assign_codewords(lengths: &[u8]) -> Result<(), Error> {
    let mut count = [0u64; 33];
    let mut used = 0usize;
    for &len in lengths {
        if len > 32 {
            return Err(Error::BadHeader);
        }
        if len > 0 {
            count[len as usize] += 1;
            used += 1;
        }
    }
    if used == 0 {
        return Err(Error::BadHeader);
    }
    let mut available = [0u64; 33];
    for i in 1..=32 {
        let sum = available[i - 1]
            .checked_add(count[i - 1])
            .ok_or(Error::BadHeader)?;
        available[i] = sum.checked_shl(1).ok_or(Error::BadHeader)?;
    }
    for &len in lengths {
        if len == 0 {
            continue;
        }
        let len = len as usize;
        if available[len] >= (1u64 << len) {
            return Err(Error::BadHeader);
        }
        available[len] += 1;
    }
    Ok(())
}

fn lookup1_values(entries: usize, dimensions: usize) -> Result<usize, Error> {
    if dimensions == 1 {
        return Ok(entries);
    }
    let mut vals = 0usize;
    loop {
        let next = vals + 1;
        let mut acc = 1usize;
        for _ in 0..dimensions {
            acc = match acc.checked_mul(next) {
                Some(v) => v,
                None => return Ok(vals),
            };
        }
        if acc > entries {
            break;
        }
        vals = next;
    }
    if vals == 0 {
        return Err(Error::BadHeader);
    }
    Ok(vals)
}

struct Floor1 {
    x_list: Vec<usize>,
}

fn read_floor(r: &mut BitReader<'_>, codebook_count: usize) -> Result<Floor1, Error> {
    let floor_type = r.read(16)?;
    if floor_type != 1 {
        return Err(Error::BadHeader);
    }
    let partitions = r.read(5)? as usize;
    let mut partition_class = vec![0usize; partitions];
    let mut max_class = -1i32;
    for class in &mut partition_class {
        let value = r.read(4)? as i32;
        *class = value as usize;
        if value > max_class {
            max_class = value;
        }
    }
    let mut classes = Vec::new();
    for _ in 0..(max_class + 1) {
        let dim = r.read(3)? as usize + 1;
        let subclasses = r.read(2)? as usize;
        let master = if subclasses > 0 {
            Some(r.read(8)? as usize)
        } else {
            None
        };
        let mut books = Vec::with_capacity(1 << subclasses);
        for _ in 0..(1usize << subclasses) {
            let raw = r.read(8)? as i32;
            books.push(raw - 1);
        }
        if let Some(master) = master {
            if master >= codebook_count {
                return Err(Error::BadHeader);
            }
        }
        for book in books {
            if book >= 0 && book as usize >= codebook_count {
                return Err(Error::BadHeader);
            }
        }
        classes.push(dim);
    }
    let _multiplier = r.read(2)? + 1;
    let rangebits = r.read(4)? as usize;
    if rangebits > 15 {
        return Err(Error::BadHeader);
    }
    let mut values = 2usize;
    for &class in &partition_class {
        values = values.checked_add(classes[class]).ok_or(Error::BadHeader)?;
    }
    if values > 258 {
        return Err(Error::BadHeader);
    }
    let mut x_list = vec![0usize; values];
    x_list[1] = 1usize << rangebits;
    for slot in x_list.iter_mut().skip(2) {
        *slot = r.read(rangebits)? as usize;
    }
    Ok(Floor1 { x_list })
}

struct Residue {
    begin: usize,
    end: usize,
    grouping: usize,
}

fn read_residue(r: &mut BitReader<'_>, codebook_count: usize) -> Result<Residue, Error> {
    let residue_type = r.read(16)?;
    if residue_type > 2 {
        return Err(Error::BadHeader);
    }
    let begin = r.read(24)? as usize;
    let end = r.read(24)? as usize;
    let grouping = r.read(24)? as usize + 1;
    let classifications = r.read(6)? as usize + 1;
    let classbook = r.read(8)? as usize;
    if end < begin || classbook >= codebook_count {
        return Err(Error::BadHeader);
    }
    let mut cascade = vec![0u8; classifications];
    for value in &mut cascade {
        let low = r.read(3)?;
        let flag = r.read(1)?;
        let high = if flag == 1 { r.read(5)? } else { 0 };
        *value = (high * 8 + low) as u8;
    }
    for value in cascade {
        for stage in 0..8 {
            if value & (1 << stage) != 0 {
                let book = r.read(8)? as usize;
                if book >= codebook_count {
                    return Err(Error::BadHeader);
                }
            }
        }
    }
    Ok(Residue {
        begin,
        end,
        grouping,
    })
}

struct MappingIn {
    coupling: bool,
    floors: Vec<usize>,
    residues: Vec<usize>,
}

fn read_mapping(
    r: &mut BitReader<'_>,
    channels: usize,
    floor_count: usize,
    residue_count: usize,
) -> Result<MappingIn, Error> {
    if r.read(16)? != 0 {
        return Err(Error::BadHeader);
    }
    let submaps = if r.read(1)? == 1 {
        r.read(4)? as usize + 1
    } else {
        1
    };
    let coupling = if r.read(1)? == 1 {
        let steps = r.read(8)? as usize + 1;
        let bits = ilog(channels.saturating_sub(1));
        for _ in 0..steps {
            let magnitude = if bits == 0 { 0 } else { r.read(bits)? as usize };
            let angle = if bits == 0 { 0 } else { r.read(bits)? as usize };
            if channels < 2 || magnitude == angle || magnitude >= channels || angle >= channels {
                return Err(Error::BadHeader);
            }
        }
        true
    } else {
        false
    };
    if r.read(2)? != 0 {
        return Err(Error::BadHeader);
    }
    if submaps > 1 {
        for _ in 0..channels {
            if r.read(4)? as usize >= submaps {
                return Err(Error::BadHeader);
            }
        }
    }
    let mut floors = Vec::with_capacity(submaps);
    let mut residues = Vec::with_capacity(submaps);
    for _ in 0..submaps {
        let _reserved = r.read(8)?;
        let floor = r.read(8)? as usize;
        let residue = r.read(8)? as usize;
        if floor >= floor_count || residue >= residue_count {
            return Err(Error::BadHeader);
        }
        floors.push(floor);
        residues.push(residue);
    }
    Ok(MappingIn {
        coupling,
        floors,
        residues,
    })
}

fn validate_setup(
    id: &Identification,
    modes: &[Mode],
    mappings: &[MappingIn],
    floors: &[Floor1],
    residues: &[Residue],
) -> Result<(), Error> {
    if modes.is_empty() {
        return Err(Error::BadHeader);
    }
    for mode in modes {
        let blocksize = if mode.blockflag == 0 {
            id.blocksize_0
        } else {
            id.blocksize_1
        };
        let mapping = &mappings[mode.mapping];
        for (&floor_index, &residue_index) in mapping.floors.iter().zip(mapping.residues.iter()) {
            let floor = &floors[floor_index];
            let mut seen = vec![false; blocksize / 2 + 1];
            for &x in &floor.x_list {
                if x > blocksize / 2 || seen[x] {
                    return Err(Error::BadHeader);
                }
                seen[x] = true;
            }
            let residue = &residues[residue_index];
            if residue.begin > residue.end || residue.end > blocksize / 2 {
                return Err(Error::BadHeader);
            }
            if (residue.end - residue.begin) % residue.grouping != 0 {
                return Err(Error::BadHeader);
            }
        }
    }
    Ok(())
}

fn decode_audio(packet: &[u8], id: &Identification, setup: &Setup) -> Result<Vec<f32>, Error> {
    if packet.is_empty() {
        return Err(Error::Truncated);
    }
    let mut r = BitReader::new(packet);
    if r.read(1)? != 0 {
        return Err(Error::BadHeader);
    }
    let mode_bits = ilog(setup.modes.len() - 1);
    let mode_number = if mode_bits == 0 {
        0
    } else {
        r.read(mode_bits)? as usize
    };
    if mode_number >= setup.modes.len() {
        return Err(Error::BadHeader);
    }
    let mode = &setup.modes[mode_number];
    if mode.blockflag != 0 {
        return Err(Error::Unsupported);
    }
    let mapping = &setup.mappings[mode.mapping];
    if mapping.coupling || id.channels != 1 {
        return Err(Error::Unsupported);
    }
    let nonzero = r.read(1)?;
    if nonzero != 0 {
        return Err(Error::Unsupported);
    }
    synthesize_short_frame(id.blocksize_0)
}

fn synthesize_short_frame(blocksize: usize) -> Result<Vec<f32>, Error> {
    if blocksize < 64 || blocksize % 2 != 0 {
        return Err(Error::BadHeader);
    }
    let spectrum = vec![0.0f32; blocksize / 2];
    let time = imdct(&spectrum);
    let window = vorbis_window(blocksize);
    // Previous half-window is silence on the first (and only) frame.
    let pcm = time
        .iter()
        .zip(window.iter())
        .take(blocksize / 2)
        .map(|(sample, weight)| sample * weight)
        .collect();
    Ok(pcm)
}

fn imdct(spectrum: &[f32]) -> Vec<f32> {
    let bins = spectrum.len();
    let n = bins * 2;
    let mut time = vec![0.0f32; n];
    if spectrum.iter().all(|sample| *sample == 0.0) {
        return time;
    }
    let factor = std::f32::consts::PI / bins as f32;
    let scale = 1.0 / bins as f32;
    for (i, sample) in time.iter_mut().enumerate() {
        let mut sum = 0.0f32;
        for (k, coeff) in spectrum.iter().enumerate() {
            let angle = factor * ((i as f32) + 0.5 + (bins as f32) / 2.0) * ((k as f32) + 0.5);
            sum += coeff * angle.cos();
        }
        *sample = sum * scale;
    }
    time
}

fn vorbis_window(n: usize) -> Vec<f32> {
    let mut window = vec![0.0f32; n];
    for (i, sample) in window.iter_mut().enumerate() {
        let z = ((i as f32) + 0.5) / n as f32 * std::f32::consts::PI;
        *sample = (0.5 * std::f32::consts::PI * z.sin() * z.sin()).sin();
    }
    window
}

struct BitReader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }

    fn read(&mut self, n: usize) -> Result<u32, Error> {
        if n == 0 {
            return Ok(0);
        }
        if n > 32 {
            return Err(Error::BadHeader);
        }
        let mut value = 0u32;
        for i in 0..n {
            let byte_index = self.bit / 8;
            let bit_index = self.bit % 8;
            if byte_index >= self.data.len() {
                return Err(Error::Truncated);
            }
            let bit = (self.data[byte_index] >> bit_index) & 1;
            value |= u32::from(bit) << i;
            self.bit += 1;
        }
        Ok(value)
    }
}

fn split_packets(bytes: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
    let mut packets = Vec::new();
    let mut open = Vec::new();
    let mut continued = false;
    let mut offset = 0usize;
    while offset < bytes.len() {
        if bytes.len() - offset < 27 {
            return Err(Error::Truncated);
        }
        if &bytes[offset..offset + 4] != b"OggS" {
            return Err(Error::BadHeader);
        }
        if bytes[offset + 4] != 0 {
            return Err(Error::BadHeader);
        }
        let header_type = bytes[offset + 5];
        let nseg = bytes[offset + 26] as usize;
        let table = offset + 27;
        if bytes.len() - table < nseg {
            return Err(Error::Truncated);
        }
        let body_start = table + nseg;
        let body_len: usize = bytes[table..body_start].iter().map(|&v| v as usize).sum();
        if bytes.len() - body_start < body_len {
            return Err(Error::Truncated);
        }
        let mut page = bytes[offset..body_start + body_len].to_vec();
        let stored = u32::from_le_bytes([page[22], page[23], page[24], page[25]]);
        page[22..26].fill(0);
        if ogg_crc(&page) != stored {
            return Err(Error::BadHeader);
        }
        let page_continues = header_type & 0x01 != 0;
        if page_continues != continued {
            return Err(Error::BadHeader);
        }
        let lacing = &bytes[table..body_start];
        let body = &bytes[body_start..body_start + body_len];
        let mut body_off = 0usize;
        for &seg in lacing {
            let seg = seg as usize;
            open.extend_from_slice(&body[body_off..body_off + seg]);
            body_off += seg;
            if seg < 255 {
                packets.push(std::mem::take(&mut open));
                continued = false;
            } else {
                continued = true;
            }
        }
        offset = body_start + body_len;
    }
    if continued {
        return Err(Error::Truncated);
    }
    Ok(packets)
}

fn parse_tone_setup(packet: &[u8]) -> Result<[f32; 256], Error> {
    if packet.len() < 7 {
        return Err(Error::Truncated);
    }
    if packet[0] != 5 || &packet[1..7] != b"vorbis" {
        return Err(Error::BadHeader);
    }
    let mut r = BitReader::new(&packet[7..]);
    expect(&mut r, 8, 1)?;
    expect_classbook(&mut r)?;
    let values = read_value_book(&mut r)?;
    expect(&mut r, 6, 0)?;
    expect(&mut r, 16, 0)?;
    expect(&mut r, 6, 0)?;
    expect(&mut r, 16, 1)?;
    expect(&mut r, 5, 0)?;
    expect(&mut r, 2, 0)?;
    expect(&mut r, 4, 5)?;
    expect(&mut r, 6, 0)?;
    expect(&mut r, 16, 1)?;
    expect(&mut r, 24, 0)?;
    expect(&mut r, 24, (BLOCKSIZE / 2) as u32)?;
    expect(&mut r, 24, 0)?;
    expect(&mut r, 6, 0)?;
    expect(&mut r, 8, 0)?;
    expect(&mut r, 3, 1)?;
    expect(&mut r, 1, 0)?;
    expect(&mut r, 8, 1)?;
    expect(&mut r, 6, 0)?;
    expect(&mut r, 16, 0)?;
    expect(&mut r, 1, 0)?;
    expect(&mut r, 1, 0)?;
    expect(&mut r, 2, 0)?;
    expect(&mut r, 8, 0)?;
    expect(&mut r, 8, 0)?;
    expect(&mut r, 8, 0)?;
    expect(&mut r, 6, 0)?;
    expect(&mut r, 1, 0)?;
    expect(&mut r, 16, 0)?;
    expect(&mut r, 16, 0)?;
    expect(&mut r, 8, 0)?;
    expect(&mut r, 1, 1)?;
    Ok(values)
}

fn expect(r: &mut BitReader<'_>, bits: usize, value: u32) -> Result<(), Error> {
    if r.read(bits)? != value {
        Err(Error::Unsupported)
    } else {
        Ok(())
    }
}

fn expect_sync(r: &mut BitReader<'_>) -> Result<(), Error> {
    if r.read(24)? != 0x0056_4342 {
        Err(Error::BadHeader)
    } else {
        Ok(())
    }
}

fn expect_classbook(r: &mut BitReader<'_>) -> Result<(), Error> {
    expect_sync(r)?;
    expect(r, 16, 1)?;
    expect(r, 24, 1)?;
    expect(r, 1, 0)?;
    expect(r, 1, 0)?;
    expect(r, 5, 0)?;
    expect(r, 4, 0)?;
    Ok(())
}

fn read_value_book(r: &mut BitReader<'_>) -> Result<[f32; 256], Error> {
    expect_sync(r)?;
    expect(r, 16, 1)?;
    expect(r, 24, 256)?;
    expect(r, 1, 0)?;
    expect(r, 1, 0)?;
    for _ in 0..256 {
        expect(r, 5, 7)?;
    }
    expect(r, 4, 1)?;
    let min = unpack_vorbis_float(r.read(32)?);
    let step = unpack_vorbis_float(r.read(32)?);
    expect(r, 4, 7)?;
    expect(r, 1, 0)?;
    let mut values = [0.0f32; 256];
    for (index, slot) in values.iter_mut().enumerate() {
        expect(r, 8, index as u32)?;
        *slot = min + index as f32 * step;
    }
    Ok(values)
}

fn decode_tone_frame(
    packet: &[u8],
    values: &[f32; 256],
    saved: &[f64; 16],
) -> Result<([f32; dsp::BINS], [f64; 16]), Error> {
    if packet.is_empty() {
        return Err(Error::Truncated);
    }
    let mut r = BitReader::new(packet);
    if r.read(1)? != 0 {
        return Err(Error::BadHeader);
    }
    let mut spectrum = [0.0f64; dsp::BINS];
    if r.read(1)? == 1 {
        if r.read(8)? != 255 || r.read(8)? != 255 {
            return Err(Error::Unsupported);
        }
        for bin in &mut spectrum {
            if r.read(1)? != 0 {
                return Err(Error::BadHeader);
            }
            let index = bitreverse(r.read(8)?, 8) as usize;
            *bin = f64::from(values[index]);
        }
    }
    let time = dsp::imdct(&spectrum);
    let (pcm, next) = dsp::overlap(saved, &time);
    Ok((pcm.map(|sample| sample as f32), next))
}

fn bitreverse(mut value: u32, bits: u32) -> u32 {
    let mut out = 0u32;
    for _ in 0..bits {
        out = (out << 1) | (value & 1);
        value >>= 1;
    }
    out
}

fn ilog(mut value: usize) -> usize {
    let mut bits = 0usize;
    while value > 0 {
        value >>= 1;
        bits += 1;
    }
    bits
}
