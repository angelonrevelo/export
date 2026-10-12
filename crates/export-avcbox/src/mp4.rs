use crate::error::Error;
use crate::param::Sps;

/// Non-fragmented MP4: `ftyp`, `moov`, `mdat`. One `avc1` sample.
pub fn write(
    sps: &Sps,
    sps_nal: &[u8],
    pps_nal: &[u8],
    idr: &[u8],
    sei: &[&[u8]],
) -> Result<Vec<u8>, Error> {
    if sps_nal.len() < 4 || pps_nal.is_empty() || idr.is_empty() {
        return Err(Error::Truncated);
    }
    let sample = sample(idr, sei)?;
    let ftyp = ftyp();
    let moov_size = moov(0, sample.len() as u32, sps, sps_nal, pps_nal)?.len();
    let offset = 8usize
        .checked_add(ftyp.len())
        .and_then(|n| n.checked_add(moov_size))
        .ok_or(Error::Size)?;
    if offset > u32::MAX as usize {
        return Err(Error::Size);
    }
    let moov = moov(offset as u32, sample.len() as u32, sps, sps_nal, pps_nal)?;
    let mut out = ftyp;
    out.extend_from_slice(&moov);
    out.extend(atom(b"mdat", &sample)?);
    Ok(out)
}

fn sample(idr: &[u8], sei: &[&[u8]]) -> Result<Vec<u8>, Error> {
    let mut payload = Vec::new();
    for nal in sei {
        push_nal(&mut payload, nal)?;
    }
    push_nal(&mut payload, idr)?;
    if payload.len() > u32::MAX as usize {
        return Err(Error::Size);
    }
    Ok(payload)
}

fn push_nal(out: &mut Vec<u8>, nal: &[u8]) -> Result<(), Error> {
    if nal.len() > u32::MAX as usize {
        return Err(Error::Size);
    }
    out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
    out.extend_from_slice(nal);
    Ok(())
}

fn ftyp() -> Vec<u8> {
    let mut body = Vec::with_capacity(24);
    body.extend_from_slice(b"isom");
    body.extend_from_slice(&0x200u32.to_be_bytes());
    body.extend_from_slice(b"isom");
    body.extend_from_slice(b"iso2");
    body.extend_from_slice(b"avc1");
    body.extend_from_slice(b"mp41");
    atom(b"ftyp", &body).expect("ftyp fits")
}

fn moov(
    offset: u32,
    sample_size: u32,
    sps: &Sps,
    sps_nal: &[u8],
    pps_nal: &[u8],
) -> Result<Vec<u8>, Error> {
    let mvhd = mvhd();
    let trak = trak(offset, sample_size, sps, sps_nal, pps_nal)?;
    let mut body = mvhd;
    body.extend(trak);
    atom(b"moov", &body)
}

fn mvhd() -> Vec<u8> {
    let mut body = Vec::with_capacity(100);
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&TIMESCALE.to_be_bytes());
    body.extend_from_slice(&1u32.to_be_bytes());
    body.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    body.extend_from_slice(&0x0100u16.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    body.extend_from_slice(&[0u8; 8]);
    body.extend_from_slice(&matrix());
    body.extend_from_slice(&[0u8; 24]);
    body.extend_from_slice(&2u32.to_be_bytes());
    atom(b"mvhd", &body).expect("mvhd fits")
}

fn trak(
    offset: u32,
    sample_size: u32,
    sps: &Sps,
    sps_nal: &[u8],
    pps_nal: &[u8],
) -> Result<Vec<u8>, Error> {
    let mut body = tkhd(sps);
    body.extend(mdia(offset, sample_size, sps, sps_nal, pps_nal)?);
    atom(b"trak", &body)
}

fn tkhd(sps: &Sps) -> Vec<u8> {
    let mut body = Vec::with_capacity(84);
    body.extend_from_slice(&7u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&1u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&1u32.to_be_bytes());
    body.extend_from_slice(&[0u8; 8]);
    body.extend_from_slice(&0u16.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    body.extend_from_slice(&matrix());
    body.extend_from_slice(&(sps.width << 16).to_be_bytes());
    body.extend_from_slice(&(sps.height << 16).to_be_bytes());
    atom(b"tkhd", &body).expect("tkhd fits")
}

fn mdia(
    offset: u32,
    sample_size: u32,
    sps: &Sps,
    sps_nal: &[u8],
    pps_nal: &[u8],
) -> Result<Vec<u8>, Error> {
    let mut body = mdhd();
    body.extend(hdlr());
    body.extend(minf(offset, sample_size, sps, sps_nal, pps_nal)?);
    atom(b"mdia", &body)
}

fn mdhd() -> Vec<u8> {
    let mut body = Vec::with_capacity(24);
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&TIMESCALE.to_be_bytes());
    body.extend_from_slice(&1u32.to_be_bytes());
    body.extend_from_slice(&0x55c4u16.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    atom(b"mdhd", &body).expect("mdhd fits")
}

fn hdlr() -> Vec<u8> {
    let mut body = Vec::with_capacity(37);
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(b"vide");
    body.extend_from_slice(&[0u8; 12]);
    body.extend_from_slice(b"VideoHandler\0");
    atom(b"hdlr", &body).expect("hdlr fits")
}

fn minf(
    offset: u32,
    sample_size: u32,
    sps: &Sps,
    sps_nal: &[u8],
    pps_nal: &[u8],
) -> Result<Vec<u8>, Error> {
    let mut body = vmhd();
    body.extend(dinf());
    body.extend(stbl(offset, sample_size, sps, sps_nal, pps_nal)?);
    atom(b"minf", &body)
}

fn vmhd() -> Vec<u8> {
    let mut body = Vec::with_capacity(12);
    body.extend_from_slice(&1u32.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    body.extend_from_slice(&[0u8; 6]);
    atom(b"vmhd", &body).expect("vmhd fits")
}

fn dinf() -> Vec<u8> {
    let url = atom(b"url ", &1u32.to_be_bytes()).expect("url fits");
    let mut dref_body = Vec::with_capacity(20);
    dref_body.extend_from_slice(&0u32.to_be_bytes());
    dref_body.extend_from_slice(&1u32.to_be_bytes());
    dref_body.extend(url);
    let dref = atom(b"dref", &dref_body).expect("dref fits");
    atom(b"dinf", &dref).expect("dinf fits")
}

fn stbl(
    offset: u32,
    sample_size: u32,
    sps: &Sps,
    sps_nal: &[u8],
    pps_nal: &[u8],
) -> Result<Vec<u8>, Error> {
    let mut body = stsd(sps, sps_nal, pps_nal)?;
    body.extend(stts());
    body.extend(stsc());
    body.extend(stsz(sample_size));
    body.extend(stco(offset));
    atom(b"stbl", &body)
}

fn stsd(sps: &Sps, sps_nal: &[u8], pps_nal: &[u8]) -> Result<Vec<u8>, Error> {
    let avc1 = avc1(sps, sps_nal, pps_nal)?;
    let mut body = Vec::new();
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&1u32.to_be_bytes());
    body.extend(avc1);
    atom(b"stsd", &body)
}

fn avc1(sps: &Sps, sps_nal: &[u8], pps_nal: &[u8]) -> Result<Vec<u8>, Error> {
    let avcc = avcc(sps_nal, pps_nal)?;
    let mut body = Vec::with_capacity(78 + avcc.len());
    body.extend_from_slice(&[0u8; 6]);
    body.extend_from_slice(&1u16.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    body.extend_from_slice(&[0u8; 12]);
    body.extend_from_slice(&(sps.width as u16).to_be_bytes());
    body.extend_from_slice(&(sps.height as u16).to_be_bytes());
    body.extend_from_slice(&0x0048_0000u32.to_be_bytes());
    body.extend_from_slice(&0x0048_0000u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&1u16.to_be_bytes());
    body.extend_from_slice(&[0u8; 32]);
    body.extend_from_slice(&0x0018u16.to_be_bytes());
    body.extend_from_slice(&0xffffu16.to_be_bytes());
    body.extend(avcc);
    atom(b"avc1", &body)
}

fn avcc(sps_nal: &[u8], pps_nal: &[u8]) -> Result<Vec<u8>, Error> {
    if sps_nal.len() < 4 || sps_nal.len() > u16::MAX as usize || pps_nal.len() > u16::MAX as usize {
        return Err(Error::Truncated);
    }
    let mut body = Vec::with_capacity(11 + sps_nal.len() + pps_nal.len());
    body.push(1);
    body.push(sps_nal[1]);
    body.push(sps_nal[2]);
    body.push(sps_nal[3]);
    body.push(0xff);
    body.push(0xe1);
    body.extend_from_slice(&(sps_nal.len() as u16).to_be_bytes());
    body.extend_from_slice(sps_nal);
    body.push(1);
    body.extend_from_slice(&(pps_nal.len() as u16).to_be_bytes());
    body.extend_from_slice(pps_nal);
    atom(b"avcC", &body)
}

fn stts() -> Vec<u8> {
    let mut body = Vec::with_capacity(16);
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&1u32.to_be_bytes());
    body.extend_from_slice(&1u32.to_be_bytes());
    body.extend_from_slice(&1u32.to_be_bytes());
    atom(b"stts", &body).expect("stts fits")
}

fn stsc() -> Vec<u8> {
    let mut body = Vec::with_capacity(20);
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&1u32.to_be_bytes());
    body.extend_from_slice(&1u32.to_be_bytes());
    body.extend_from_slice(&1u32.to_be_bytes());
    body.extend_from_slice(&1u32.to_be_bytes());
    atom(b"stsc", &body).expect("stsc fits")
}

fn stsz(sample_size: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(16);
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&1u32.to_be_bytes());
    body.extend_from_slice(&sample_size.to_be_bytes());
    atom(b"stsz", &body).expect("stsz fits")
}

fn stco(offset: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(12);
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&1u32.to_be_bytes());
    body.extend_from_slice(&offset.to_be_bytes());
    atom(b"stco", &body).expect("stco fits")
}

fn matrix() -> [u8; 36] {
    let mut m = [0u8; 36];
    m[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    m[16..20].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    m[32..36].copy_from_slice(&0x4000_0000u32.to_be_bytes());
    m
}

fn atom(kind: &[u8; 4], payload: &[u8]) -> Result<Vec<u8>, Error> {
    let total = 8usize.checked_add(payload.len()).ok_or(Error::Size)?;
    if total > u32::MAX as usize {
        return Err(Error::Size);
    }
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&(total as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    Ok(out)
}

const TIMESCALE: u32 = 30;
