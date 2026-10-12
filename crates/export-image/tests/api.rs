use export_image::{
    decode_jpeg, decode_png, encode_jpeg, encode_png, resize, restore_lossless_all, sha256,
    shrink_png, sniff, Filter, Format, Raster, ShrinkCache,
};

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn solid(w: u32, h: u32, rgb: [u8; 3]) -> Raster {
    let mut pixels = Vec::with_capacity((w * h * 3) as usize);
    for _ in 0..w * h {
        pixels.extend_from_slice(&rgb);
    }
    Raster::new(w, h, 3, pixels).unwrap()
}

fn ramp(w: u32, h: u32) -> Raster {
    let mut pixels = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            pixels.push((x * 3) as u8);
            pixels.push((y * 2) as u8);
            pixels.push(48);
        }
    }
    Raster::new(w, h, 3, pixels).unwrap()
}

fn noise(w: u32, h: u32) -> Raster {
    let mut pixels = Vec::with_capacity((w * h * 3) as usize);
    let mut x = 0x1234_5678u32;
    for _ in 0..w * h * 3 {
        x = x.wrapping_mul(1664525).wrapping_add(1013904223);
        pixels.push((x >> 24) as u8);
    }
    Raster::new(w, h, 3, pixels).unwrap()
}

fn mae(a: &Raster, b: &Raster) -> f64 {
    let mut sum = 0u64;
    for (p, q) in a.pixels.iter().zip(b.pixels.iter()) {
        sum += u64::from((*p as i16 - *q as i16).unsigned_abs());
    }
    sum as f64 / a.pixels.len() as f64
}

#[test]
fn sha256_of_png_is_stable() {
    let png = encode_png(&solid(2, 2, [1, 2, 3])).unwrap();
    assert_eq!(sha256(&png), sha256(&png));
}

#[test]
fn sniff_uses_magic_not_a_name() {
    let png = encode_png(&solid(2, 2, [9, 8, 7])).unwrap();
    assert_eq!(sniff(&png), Format::Png);
    assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xd9]), Format::Jpeg);
    assert_eq!(sniff(b"RIFF\x00\x00\x00\x00WEBP"), Format::Webp);
    assert_eq!(sniff(b"not an image"), Format::Unknown);
}

#[test]
fn png_roundtrip_is_sample_exact() {
    let src = noise(17, 13);
    let png = encode_png(&src).unwrap();
    let back = decode_png(&png).unwrap();
    assert_eq!(back, src);
}

#[test]
fn python_zlib_pngs_decode_exact() {
    let rgb = hex("89504e470d0a1a0a0000000d49484452000000040000000308020000003b9639910000002d4944415478da0dc7410100300803b19383888a404efd21021115c1965f802bd2ac19d095d25aebc7574e7bed7917f0102dba2e5e940000000049454e44ae426082");
    let img = decode_png(&rgb).unwrap();
    assert_eq!((img.width, img.height, img.channels), (4, 3, 3));
    assert_eq!(&img.pixels[0..3], &[0, 0, 255]);
    assert_eq!(&img.pixels[3..6], &[40, 0, 235]);
    assert_eq!(&img.pixels[12..15], &[0, 60, 255]);

    let gray = hex("89504e470d0a1a0a0000000d494844520000000200000002080000000057dd52f80000000e4944415478da6310506030700000017600a1f158c4820000000049454e44ae426082");
    let img = decode_png(&gray).unwrap();
    assert_eq!(img.pixels, vec![0x10, 0x20, 0x30, 0x40]);

    let sub = hex("89504e470d0a1a0a0000000d49484452000000020000000108020000007b40e8dd0000000f4944415478da63e4129163606000000126003eda83dc1e0000000049454e44ae426082");
    let img = decode_png(&sub).unwrap();
    assert_eq!(img.pixels, vec![10, 20, 30, 10, 20, 30]);

    let pal = hex("89504e470d0a1a0a0000000d49484452000000030000000108030000002c3ee48600000009504c5445ff000000ff000000ff2d4acd8a0000000374524e53ff80007f6d68780000000c4944415478da63606064020000080004081d630a0000000049454e44ae426082");
    let img = decode_png(&pal).unwrap();
    assert_eq!(img.channels, 4);
    assert_eq!(
        img.pixels,
        vec![255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0]
    );

    let rgba = hex("89504e470d0a1a0a0000000d494844520000000200000002080600000072b60d24000000184944415478da63e01291fbcfc8c4dcc00002ffffff770000202d04ffd11dd2930000000049454e44ae426082");
    let img = decode_png(&rgba).unwrap();
    assert_eq!(
        img.pixels,
        vec![10, 20, 30, 255, 1, 2, 3, 128, 0, 0, 0, 0, 255, 255, 255, 64]
    );
}

#[test]
fn lossless_report_counts_failures_and_keeps_bytes() {
    let png = encode_png(&solid(4, 4, [3, 4, 5])).unwrap();
    let owned = png.clone();
    let report = restore_lossless_all(&[&png, b"nope", &png[..8]]);
    assert_eq!(png, owned);
    assert_eq!(report.seen, 3);
    assert_eq!(report.restored, 1);
    assert_eq!(report.failed, 2);
    assert_eq!(report.restored + report.failed, report.seen);
}

#[test]
fn jpeg_roundtrip_and_quality_changes_size() {
    let gray = solid(16, 16, [128, 128, 128]);
    let encoded = encode_jpeg(&gray, 40).unwrap();
    let back = decode_jpeg(&encoded).unwrap();
    assert_eq!(mae(&gray, &back), 0.0);

    let red = solid(16, 16, [200, 10, 10]);
    let encoded = encode_jpeg(&red, 90).unwrap();
    let back = decode_jpeg(&encoded).unwrap();
    assert!(mae(&red, &back) < 6.0, "mae {}", mae(&red, &back));

    let n = noise(32, 32);
    let small = encode_jpeg(&n, 15).unwrap();
    let large = encode_jpeg(&n, 85).unwrap();
    assert!(small.len() < large.len(), "{} vs {}", small.len(), large.len());
}

#[test]
fn resize_area_and_bilinear_bounds() {
    let src = Raster::new(
        2,
        2,
        3,
        vec![0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 2],
    )
    .unwrap();
    let area = resize(&src, 1, 1, Filter::Area).unwrap();
    assert_eq!(area.pixels, vec![1, 1, 1]);
    let up = resize(&solid(1, 1, [9, 8, 7]), 3, 2, Filter::Bilinear).unwrap();
    assert_eq!((up.width, up.height), (3, 2));
    assert!(up.pixels.iter().all(|&p| p == 9 || p == 8 || p == 7));
}

#[test]
fn shrink_measures_quality_and_lands_on_the_cap() {
    // Stored IDAT keeps the PNG larger than a JPEG of the same pixels, so the
    // cap sits under the source and the search has to encode.
    let flat = ramp(64, 64);
    let busy = noise(64, 64);
    let flat_png = stored_png(&flat);
    let busy_png = stored_png(&busy);
    let flat_hi = encode_jpeg(&flat, 80).unwrap().len() as u64;
    let busy_lo = encode_jpeg(&busy, 30).unwrap().len() as u64;
    assert!(
        flat_hi + 32 < busy_lo,
        "fixture did not separate sizes: flat {flat_hi} busy {busy_lo}"
    );
    let target = flat_hi + 64;
    assert!(
        (flat_png.len() as u64) > target && (busy_png.len() as u64) > target,
        "png flat {} busy {} target {target}",
        flat_png.len(),
        busy_png.len()
    );
    let mut cache = ShrinkCache::new();
    let flat_out = shrink_png(&flat_png, target, &mut cache).unwrap();
    let busy_out = shrink_png(&busy_png, target, &mut cache).unwrap();
    assert!(!flat_out.copied && !busy_out.copied);
    assert_eq!(flat_out.bytes.len() as u64, target);
    assert_eq!(busy_out.bytes.len() as u64, target);
    let fq = flat_out.quality.unwrap();
    let bq = busy_out.quality.unwrap();
    assert!(fq > bq, "flat q {fq} busy q {bq}");
    assert!(flat_out.probes.len() > 1 && busy_out.probes.len() > 1);
    assert!(flat_out.probes.iter().any(|p| {
        p.quality == fq && matches!(p.bytes, Some(n) if n <= target)
    }));
    for probe in &flat_out.probes {
        if probe.quality > fq {
            assert!(probe.bytes.map(|n| n > target).unwrap_or(true));
        }
    }
    let decoded = decode_jpeg(&flat_out.bytes).unwrap();
    assert_eq!((decoded.width, decoded.height), (flat_out.width, flat_out.height));
}

#[test]
fn under_cap_is_copied_and_repeat_skips_encode() {
    let png = encode_png(&solid(8, 8, [4, 5, 6])).unwrap();
    let mut cache = ShrinkCache::new();
    let cap = png.len() as u64 + 25;
    let first = shrink_png(&png, cap, &mut cache).unwrap();
    assert!(first.copied);
    assert_eq!(first.bytes, png);
    assert!(first.quality.is_none());
    assert_eq!(cache.encodes, 0);
    let passes = cache.hash_passes;
    let second = shrink_png(&png, cap, &mut cache).unwrap();
    assert!(second.dedup_hit);
    assert_eq!(second.bytes, png);
    assert_eq!(cache.encodes, 0);
    assert_eq!(cache.hash_passes, passes + 1);
}

#[test]
fn impossible_cap_is_an_error() {
    let png = encode_png(&noise(24, 24)).unwrap();
    let mut cache = ShrinkCache::new();
    let err = shrink_png(&png, 80, &mut cache).unwrap_err();
    assert_eq!(err.code, export_image::ErrorCode::OverBudget);
}

#[test]
fn pixel_cap_rejects_a_bomb_header() {
    // IHDR claims 20000x20000. The chunk CRC is wrong on purpose only if we
    // fail closed either way: a real bomb header is TOO_LARGE.
    let mut png = Vec::new();
    png.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&20_000u32.to_be_bytes());
    ihdr[4..8].copy_from_slice(&20_000u32.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = 2;
    // Build a valid CRC so the failure is the pixel cap, not a bad checksum.
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(b"IHDR");
    png.extend_from_slice(&ihdr);
    let crc = png_crc(b"IHDR", &ihdr);
    png.extend_from_slice(&crc.to_be_bytes());
    let err = decode_png(&png).unwrap_err();
    assert_eq!(err.code, export_image::ErrorCode::TooLarge);
}

fn stored_png(src: &Raster) -> Vec<u8> {
    let stride = src.width as usize * 3;
    let mut raw = Vec::with_capacity((stride + 1) * src.height as usize);
    for y in 0..src.height as usize {
        raw.push(0);
        let row = y * stride;
        raw.extend_from_slice(&src.pixels[row..row + stride]);
    }
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&src.width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&src.height.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = 2;
    let mut out = Vec::new();
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    push_chunk(&mut out, b"IHDR", &ihdr);
    push_chunk(&mut out, b"IDAT", &stored_zlib(&raw));
    push_chunk(&mut out, b"IEND", &[]);
    out
}

fn push_chunk(out: &mut Vec<u8>, tag: &[u8], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let crc = png_crc(tag, data);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn stored_zlib(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 32);
    out.extend_from_slice(&[0x78, 0x01]);
    let mut i = 0;
    while i < data.len() {
        let n = (data.len() - i).min(65535);
        let last = i + n == data.len();
        out.push(if last { 0x01 } else { 0x00 });
        out.extend_from_slice(&(n as u16).to_le_bytes());
        out.extend_from_slice(&((n as u16) ^ 0xffff).to_le_bytes());
        out.extend_from_slice(&data[i..i + n]);
        i += n;
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn png_crc(tag: &[u8], data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in tag.iter().chain(data.iter()) {
        c ^= u32::from(b);
        for _ in 0..8 {
            let mask = (c & 1).wrapping_neg();
            c = (c >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !c
}
