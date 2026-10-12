//! Build one Ogg Vorbis bitstream: mono, 48 kHz, one short frame.
//!
//! The audio packet uses an unused floor, so the frame is silence. This is one
//! packet, not a song encoder.

use crate::{ogg_crc, Error, BLOCKSIZE, BLOCKSIZE_EXP, FRAME_SAMPLES, SAMPLE_RATE_HZ};

const VENDOR: &[u8] = b"export-vorbis";
const SERIAL: u32 = 0x766F_7262;

/// Encode one silent short frame as a single Vorbis audio packet in an Ogg bitstream.
///
/// `pcm` must be exactly [`FRAME_SAMPLES`] finite zeros. Any other frame is
/// unsupported: this crate does not encode a song.
pub fn encode_packet(pcm: &[f32]) -> Result<Vec<u8>, Error> {
    if pcm.len() != FRAME_SAMPLES || pcm.iter().any(|s| !s.is_finite() || s.abs() > 1.0e-6) {
        return Err(Error::Unsupported);
    }

    let id = identification_packet();
    let comment = comment_packet();
    let setup = setup_packet();
    let audio = audio_packet();

    let mut out = Vec::with_capacity(256);
    out.extend(ogg_page(0x02, 0, SERIAL, 0, &[&id]));
    out.extend(ogg_page(0x00, 0, SERIAL, 1, &[&comment]));
    out.extend(ogg_page(0x00, 0, SERIAL, 2, &[&setup]));
    out.extend(ogg_page(0x04, FRAME_SAMPLES as u64, SERIAL, 3, &[&audio]));
    Ok(out)
}

fn identification_packet() -> Vec<u8> {
    let mut packet = vec![0u8; 30];
    packet[0] = 1;
    packet[1..7].copy_from_slice(b"vorbis");
    packet[11] = 1;
    packet[12..16].copy_from_slice(&SAMPLE_RATE_HZ.to_le_bytes());
    packet[28] = (BLOCKSIZE_EXP << 4) | BLOCKSIZE_EXP;
    packet[29] = 1;
    let _ = BLOCKSIZE;
    packet
}

fn comment_packet() -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(3, 8);
    for &byte in b"vorbis" {
        w.write_bits(u32::from(byte), 8);
    }
    w.write_bits(VENDOR.len() as u32, 32);
    for &byte in VENDOR {
        w.write_bits(u32::from(byte), 8);
    }
    w.write_bits(0, 32);
    w.write_bits(1, 1);
    w.finish()
}

fn setup_packet() -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(5, 8);
    for &byte in b"vorbis" {
        w.write_bits(u32::from(byte), 8);
    }

    // One codebook.
    w.write_bits(0, 8);
    write_codebook(&mut w);

    // One placeholder time backend of type 0.
    w.write_bits(0, 6);
    w.write_bits(0, 16);

    // One floor type 1, two points, no partitions.
    w.write_bits(0, 6);
    w.write_bits(1, 16);
    w.write_bits(0, 5);
    w.write_bits(0, 2);
    w.write_bits(5, 4);

    // One residue type 2 covering the short-block half window.
    w.write_bits(0, 6);
    w.write_bits(2, 16);
    w.write_bits(0, 24);
    w.write_bits((BLOCKSIZE / 2) as u32, 24);
    // Partition size and classification count are stored minus one.
    w.write_bits((BLOCKSIZE / 2 - 1) as u32, 24);
    w.write_bits(0, 6);
    w.write_bits(0, 8);
    w.write_bits(0, 3);
    w.write_bits(0, 1);

    // One mapping, no coupling, one submap.
    w.write_bits(0, 6);
    w.write_bits(0, 16);
    w.write_bits(0, 1);
    w.write_bits(0, 1);
    w.write_bits(0, 2);
    w.write_bits(0, 8);
    w.write_bits(0, 8);
    w.write_bits(0, 8);

    // One short mode.
    w.write_bits(0, 6);
    w.write_bits(0, 1);
    w.write_bits(0, 16);
    w.write_bits(0, 16);
    w.write_bits(0, 8);
    w.write_bits(1, 1);
    w.finish()
}

fn write_codebook(w: &mut BitWriter) {
    w.write_bits(0x0056_4342, 24);
    w.write_bits(1, 16);
    w.write_bits(1, 24);
    w.write_bits(0, 1);
    w.write_bits(0, 1);
    // Length is stored minus one. Zero decodes as a one-bit codeword.
    w.write_bits(0, 5);
    w.write_bits(0, 4);
}

fn audio_packet() -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(0, 1);
    w.write_bits(0, 1);
    w.finish()
}

fn ogg_page(
    header_type: u8,
    granule: u64,
    serial: u32,
    sequence: u32,
    packets: &[&[u8]],
) -> Vec<u8> {
    let mut lacing = Vec::new();
    let mut body = Vec::new();
    for packet in packets {
        let mut rest = *packet;
        if rest.is_empty() {
            lacing.push(0);
            continue;
        }
        while rest.len() >= 255 {
            lacing.push(255);
            body.extend_from_slice(&rest[..255]);
            rest = &rest[255..];
        }
        lacing.push(rest.len() as u8);
        body.extend_from_slice(rest);
    }

    let mut page = Vec::with_capacity(27 + lacing.len() + body.len());
    page.extend_from_slice(b"OggS");
    page.push(0);
    page.push(header_type);
    page.extend_from_slice(&granule.to_le_bytes());
    page.extend_from_slice(&serial.to_le_bytes());
    page.extend_from_slice(&sequence.to_le_bytes());
    page.extend_from_slice(&0u32.to_le_bytes());
    page.push(lacing.len() as u8);
    page.extend_from_slice(&lacing);
    page.extend_from_slice(&body);
    let crc = ogg_crc(&page);
    page[22..26].copy_from_slice(&crc.to_le_bytes());
    page
}

struct BitWriter {
    bytes: Vec<u8>,
    filled: u8,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            filled: 0,
        }
    }

    fn write_bits(&mut self, value: u32, n: u32) {
        for i in 0..n {
            self.write_bit(((value >> i) & 1) as u8);
        }
    }

    fn write_bit(&mut self, bit: u8) {
        if self.filled == 0 {
            self.bytes.push(0);
        }
        if bit != 0 {
            let index = self.bytes.len() - 1;
            self.bytes[index] |= 1 << self.filled;
        }
        self.filled += 1;
        if self.filled == 8 {
            self.filled = 0;
        }
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}
