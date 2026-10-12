//! Compound File Binary reader, one sector at a time.
//!
//! The header is 512 bytes. Sector size is 512 or 4096. The FAT, the mini FAT,
//! and the sector under the cursor each occupy one buffer of that size, allocated
//! once. Directory entries are matched in place. Nothing here grows with the
//! encrypted package.

use super::DecryptError;
use std::io::{Read, Seek, SeekFrom};

const MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const END: u32 = 0xFFFF_FFFE;
const FREE: u32 = 0xFFFF_FFFF;
const FAT_SECT: u32 = 0xFFFF_FFFD;
const HEADER: u64 = 512;
const MAX_DIR_SECTORS: u32 = 64;
const MAX_INFO: u64 = 64 * 1024;

pub struct StreamLoc {
    pub start: u32,
    pub size: u64,
    pub mini: bool,
}

struct Seq {
    mini: bool,
    size: u64,
    pos: u64,
    sector: u32,
    window_at: usize,
    window_len: usize,
    steps: u64,
    open: bool,
}

pub struct Cfb<R> {
    file: R,
    major: u16,
    sector_size: usize,
    cutoff: u32,
    fat_count: u32,
    dir_start: u32,
    mini_fat_start: u32,
    mini_fat_count: u32,
    difat_first: u32,
    header_difat: [u32; 109],
    fat_buf: Vec<u8>,
    fat_cached: Option<u32>,
    mini_fat_buf: Vec<u8>,
    mini_cached: Option<u32>,
    data_buf: Vec<u8>,
    hold: [u8; 64],
    root_start: u32,
    root_size: u64,
    root_cache: Option<(u64, u32)>,
    seq: Seq,
}

impl<R: Read + Seek> Cfb<R> {
    pub fn open(mut file: R) -> Result<Self, DecryptError> {
        file.seek(SeekFrom::Start(0))
            .map_err(DecryptError::from_io)?;
        let mut header = [0u8; HEADER as usize];
        let mut got = 0;
        while got < 8 {
            match file.read(&mut header[got..8]) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(DecryptError::from_io(e)),
            }
        }
        if got == 0 {
            return Err(DecryptError::Truncated);
        }
        if header[..got] != MAGIC[..got] {
            return Err(DecryptError::NotEncrypted);
        }
        if got < 8 {
            return Err(DecryptError::Truncated);
        }
        read_exact(&mut file, &mut header[8..])?;
        let major = u16le(&header, 0x1A)?;
        let order = u16le(&header, 0x1C)?;
        if order != 0xFFFE {
            return Err(DecryptError::Corrupt);
        }
        let shift = u16le(&header, 0x1E)?;
        // 4096-byte sectors (major 4) are a different layout and are not covered
        // by the fixture. Refuse them instead of guessing.
        let sector_size = match (major, shift) {
            (3, 9) => 512,
            (4, _) => return Err(DecryptError::Unsupported("cfb sector 4096")),
            (3, _) => return Err(DecryptError::Corrupt),
            _ => return Err(DecryptError::Unsupported("cfb version")),
        };
        let mini_shift = u16le(&header, 0x20)?;
        if mini_shift != 6 {
            return Err(DecryptError::Unsupported("mini sector"));
        }
        let mut header_difat = [FREE; 109];
        for (i, slot) in header_difat.iter_mut().enumerate() {
            *slot = u32le(&header, 0x4C + i * 4)?;
        }
        let sector_size_usize = sector_size;
        Ok(Self {
            file,
            major,
            sector_size: sector_size_usize,
            cutoff: u32le(&header, 0x38)?,
            fat_count: u32le(&header, 0x2C)?,
            dir_start: u32le(&header, 0x30)?,
            mini_fat_start: u32le(&header, 0x3C)?,
            mini_fat_count: u32le(&header, 0x40)?,
            difat_first: u32le(&header, 0x44)?,
            header_difat,
            fat_buf: vec![0; sector_size_usize],
            fat_cached: None,
            mini_fat_buf: vec![0; sector_size_usize],
            mini_cached: None,
            data_buf: vec![0; sector_size_usize],
            hold: [0; 64],
            root_start: END,
            root_size: 0,
            root_cache: None,
            seq: Seq {
                mini: false,
                size: 0,
                pos: 0,
                sector: END,
                window_at: 0,
                window_len: 0,
                steps: 0,
                open: false,
            },
        })
    }

    pub fn encryption_streams(&mut self) -> Result<(StreamLoc, StreamLoc), DecryptError> {
        let mut info = None;
        let mut package = None;
        let mut root = None;
        let mut sector = self.dir_start;
        let mut seen = 0u32;
        loop {
            if sector == END {
                break;
            }
            if special(sector) {
                return Err(DecryptError::Corrupt);
            }
            seen += 1;
            if seen > MAX_DIR_SECTORS {
                return Err(DecryptError::TooLarge);
            }
            self.load_data(sector)?;
            let n = self.sector_size / 128;
            for i in 0..n {
                let e = &self.data_buf[i * 128..i * 128 + 128];
                let ty = e[66];
                if ty == 0 {
                    continue;
                }
                let start = u32le(e, 116)?;
                let size = self.stream_size(e)?;
                if ty == 5 && name_eq(e, "Root Entry") {
                    root = Some((start, size));
                } else if ty == 2 && name_eq(e, "EncryptionInfo") {
                    if info.is_some() {
                        return Err(DecryptError::Corrupt);
                    }
                    info = Some(StreamLoc {
                        start,
                        size,
                        mini: size < self.cutoff as u64,
                    });
                } else if ty == 2 && name_eq(e, "EncryptedPackage") {
                    if package.is_some() {
                        return Err(DecryptError::Corrupt);
                    }
                    package = Some(StreamLoc {
                        start,
                        size,
                        mini: size < self.cutoff as u64,
                    });
                }
            }
            sector = self.fat_next(sector)?;
        }
        let (root_start, root_size) = root.ok_or(DecryptError::Corrupt)?;
        self.root_start = root_start;
        self.root_size = root_size;
        let info = info.ok_or(DecryptError::Corrupt)?;
        let package = package.ok_or(DecryptError::Corrupt)?;
        if info.size > MAX_INFO {
            return Err(DecryptError::TooLarge);
        }
        Ok((info, package))
    }

    pub fn read_entire(&mut self, loc: &StreamLoc) -> Result<Vec<u8>, DecryptError> {
        if loc.size > MAX_INFO {
            return Err(DecryptError::TooLarge);
        }
        let mut buf = vec![0u8; loc.size as usize];
        self.open_seq(loc);
        if loc.size > 0 {
            self.read_exact(&mut buf)?;
        }
        Ok(buf)
    }

    pub fn open_seq(&mut self, loc: &StreamLoc) {
        self.seq = Seq {
            mini: loc.mini,
            size: loc.size,
            pos: 0,
            sector: loc.start,
            window_at: 0,
            window_len: 0,
            steps: 0,
            open: true,
        };
        self.root_cache = None;
    }

    pub fn read_exact(&mut self, dest: &mut [u8]) -> Result<(), DecryptError> {
        let mut off = 0;
        while off < dest.len() {
            let n = self.read_some(&mut dest[off..])?;
            if n == 0 {
                return Err(DecryptError::Truncated);
            }
            off += n;
        }
        Ok(())
    }

    fn read_some(&mut self, dest: &mut [u8]) -> Result<usize, DecryptError> {
        if !self.seq.open || self.seq.pos >= self.seq.size || dest.is_empty() {
            return Ok(0);
        }
        if self.seq.window_at >= self.seq.window_len {
            self.fill_window()?;
        }
        let at = self.seq.window_at;
        let len = self.seq.window_len;
        let available = if self.seq.mini {
            &self.hold[at..len]
        } else {
            &self.data_buf[at..len]
        };
        let left = (self.seq.size - self.seq.pos) as usize;
        let n = dest.len().min(available.len()).min(left);
        dest[..n].copy_from_slice(&available[..n]);
        self.seq.window_at += n;
        self.seq.pos += n as u64;
        Ok(n)
    }

    fn fill_window(&mut self) -> Result<(), DecryptError> {
        if self.seq.steps > 0 {
            let next = if self.seq.mini {
                self.mini_fat_next(self.seq.sector)?
            } else {
                self.fat_next(self.seq.sector)?
            };
            if special(next) {
                return Err(DecryptError::Truncated);
            }
            self.seq.sector = next;
        } else if special(self.seq.sector) {
            return Err(DecryptError::Truncated);
        }
        self.seq.steps += 1;
        let unit = if self.seq.mini {
            64
        } else {
            self.sector_size as u64
        };
        let need = self.seq.size / unit + 2;
        if self.seq.steps > need {
            return Err(DecryptError::Corrupt);
        }
        if self.seq.mini {
            self.load_mini(self.seq.sector)?;
            self.seq.window_len = 64;
        } else {
            let id = self.seq.sector;
            self.load_data(id)?;
            self.seq.window_len = self.sector_size;
        }
        self.seq.window_at = 0;
        Ok(())
    }

    fn load_mini(&mut self, mini_id: u32) -> Result<(), DecryptError> {
        let offset = u64::from(mini_id) * 64;
        if offset.checked_add(64).ok_or(DecryptError::Corrupt)? > self.root_size {
            return Err(DecryptError::Truncated);
        }
        let ss = self.sector_size as u64;
        let index = offset / ss;
        let inner = (offset % ss) as usize;
        let sector = self.root_sector(index)?;
        self.load_data(sector)?;
        self.hold.copy_from_slice(&self.data_buf[inner..inner + 64]);
        Ok(())
    }

    fn root_sector(&mut self, index: u64) -> Result<u32, DecryptError> {
        if let Some((have, id)) = self.root_cache {
            if have == index {
                return Ok(id);
            }
            if have + 1 == index {
                let next = self.fat_next(id)?;
                if special(next) {
                    return Err(DecryptError::Truncated);
                }
                self.root_cache = Some((index, next));
                return Ok(next);
            }
        }
        let mut id = self.root_start;
        let mut i = 0u64;
        loop {
            if special(id) {
                return Err(DecryptError::Truncated);
            }
            if i == index {
                self.root_cache = Some((index, id));
                return Ok(id);
            }
            id = self.fat_next(id)?;
            i += 1;
            if i > self.root_size / self.sector_size as u64 + 2 {
                return Err(DecryptError::Corrupt);
            }
        }
    }

    fn fat_next(&mut self, sector: u32) -> Result<u32, DecryptError> {
        let per = (self.sector_size / 4) as u32;
        let fat_index = sector / per;
        let within = (sector % per) as usize * 4;
        let fat_sector = self.fat_sector_id(fat_index)?;
        if self.fat_cached != Some(fat_sector) {
            self.load_fat(fat_sector)?;
            self.fat_cached = Some(fat_sector);
        }
        u32le(&self.fat_buf, within)
    }

    fn mini_fat_next(&mut self, mini_sector: u32) -> Result<u32, DecryptError> {
        let per = (self.sector_size / 4) as u32;
        let idx = mini_sector / per;
        let within = (mini_sector % per) as usize * 4;
        let sector = self.mini_fat_sector(idx)?;
        if self.mini_cached != Some(sector) {
            self.load_mini_fat(sector)?;
            self.mini_cached = Some(sector);
        }
        u32le(&self.mini_fat_buf, within)
    }

    fn fat_sector_id(&mut self, index: u32) -> Result<u32, DecryptError> {
        if self.fat_count == 0 || index >= self.fat_count {
            return Err(DecryptError::Corrupt);
        }
        if index < 109 {
            let id = self.header_difat[index as usize];
            if special(id) {
                return Err(DecryptError::Corrupt);
            }
            return Ok(id);
        }
        let per = (self.sector_size / 4) - 1;
        let mut left = index - 109;
        let mut dif = self.difat_first;
        for _ in 0..1024 {
            if special(dif) {
                return Err(DecryptError::Corrupt);
            }
            self.load_fat(dif)?;
            self.fat_cached = None;
            if (left as usize) < per {
                return u32le(&self.fat_buf, left as usize * 4);
            }
            left -= per as u32;
            dif = u32le(&self.fat_buf, per * 4)?;
        }
        Err(DecryptError::Corrupt)
    }

    fn mini_fat_sector(&mut self, index: u32) -> Result<u32, DecryptError> {
        let mut id = self.mini_fat_start;
        for _ in 0..index {
            if special(id) {
                return Err(DecryptError::Corrupt);
            }
            id = self.fat_next(id)?;
        }
        if special(id) || self.mini_fat_count == 0 || index >= self.mini_fat_count {
            return Err(DecryptError::Corrupt);
        }
        Ok(id)
    }

    fn stream_size(&self, entry: &[u8]) -> Result<u64, DecryptError> {
        let size = u64le(entry, 120)?;
        if self.major < 4 {
            if size >> 32 != 0 {
                return Err(DecryptError::Corrupt);
            }
            Ok(size & 0xFFFF_FFFF)
        } else {
            Ok(size)
        }
    }

    fn load_data(&mut self, id: u32) -> Result<(), DecryptError> {
        self.load_into(id, Which::Data)
    }
    fn load_fat(&mut self, id: u32) -> Result<(), DecryptError> {
        self.load_into(id, Which::Fat)
    }
    fn load_mini_fat(&mut self, id: u32) -> Result<(), DecryptError> {
        self.load_into(id, Which::Mini)
    }

    fn load_into(&mut self, id: u32, which: Which) -> Result<(), DecryptError> {
        if special(id) {
            return Err(DecryptError::Corrupt);
        }
        let off = HEADER
            .checked_add(u64::from(id) * self.sector_size as u64)
            .ok_or(DecryptError::Corrupt)?;
        self.file
            .seek(SeekFrom::Start(off))
            .map_err(DecryptError::from_io)?;
        let n = self.sector_size;
        match which {
            Which::Data => read_exact(&mut self.file, &mut self.data_buf[..n])?,
            Which::Fat => read_exact(&mut self.file, &mut self.fat_buf[..n])?,
            Which::Mini => read_exact(&mut self.file, &mut self.mini_fat_buf[..n])?,
        }
        Ok(())
    }
}

enum Which {
    Data,
    Fat,
    Mini,
}

fn special(id: u32) -> bool {
    id >= 0xFFFF_FFF0
}

fn name_eq(entry: &[u8], expect: &str) -> bool {
    let n = u16::from_le_bytes([entry[64], entry[65]]) as usize;
    let chars = expect.encode_utf16().count();
    if n != chars * 2 + 2 || n > 64 {
        return false;
    }
    for (i, unit) in expect.encode_utf16().enumerate() {
        let got = u16::from_le_bytes([entry[i * 2], entry[i * 2 + 1]]);
        if got != unit {
            return false;
        }
    }
    true
}

fn u16le(b: &[u8], off: usize) -> Result<u16, DecryptError> {
    let s = b.get(off..off + 2).ok_or(DecryptError::Corrupt)?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

fn u32le(b: &[u8], off: usize) -> Result<u32, DecryptError> {
    let s = b.get(off..off + 4).ok_or(DecryptError::Corrupt)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn u64le(b: &[u8], off: usize) -> Result<u64, DecryptError> {
    let s = b.get(off..off + 8).ok_or(DecryptError::Corrupt)?;
    Ok(u64::from_le_bytes([
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
    ]))
}

fn read_exact<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<(), DecryptError> {
    let mut off = 0;
    while off < buf.len() {
        match r.read(&mut buf[off..]) {
            Ok(0) => return Err(DecryptError::Truncated),
            Ok(n) => off += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(DecryptError::from_io(e)),
        }
    }
    Ok(())
}

pub fn build(info: &[u8], package: &[u8]) -> Result<Vec<u8>, DecryptError> {
    const SS: usize = 512;
    const CUTOFF: u32 = 4096;
    let info_mini = (info.len() as u32) < CUTOFF;
    let pkg_mini = (package.len() as u32) < CUTOFF;

    let mut mini_data = Vec::new();
    let mut mini_fat: Vec<u32> = Vec::new();
    let info_mini_start = if info_mini {
        push_mini(&mut mini_data, &mut mini_fat, info)
    } else {
        0
    };
    let pkg_mini_start = if pkg_mini {
        push_mini(&mut mini_data, &mut mini_fat, package)
    } else {
        0
    };
    if mini_fat.len() > 128 {
        return Err(DecryptError::TooLarge);
    }

    let mini_stream_sectors = mini_data.len().div_ceil(SS);
    let mini_fat_sectors = if mini_data.is_empty() { 0 } else { 1 };
    let info_reg_sectors = if info_mini {
        0
    } else {
        info.len().div_ceil(SS)
    };
    let pkg_reg_sectors = if pkg_mini {
        0
    } else {
        package.len().div_ceil(SS)
    };
    let fixed = 1 + mini_fat_sectors + mini_stream_sectors + info_reg_sectors + pkg_reg_sectors;
    let fat_sectors = fat_len(fixed)?;
    let total = fat_sectors + fixed;

    let dir_at = fat_sectors;
    let mut cursor = fat_sectors + 1;
    let mini_fat_at = if mini_fat_sectors == 0 {
        None
    } else {
        let at = cursor;
        cursor += mini_fat_sectors;
        Some(at)
    };
    let mini_stream_at = if mini_stream_sectors == 0 {
        None
    } else {
        let at = cursor;
        cursor += mini_stream_sectors;
        Some(at)
    };
    let info_reg_at = if info_reg_sectors == 0 {
        None
    } else {
        let at = cursor;
        cursor += info_reg_sectors;
        Some(at)
    };
    let pkg_reg_at = if pkg_reg_sectors == 0 {
        None
    } else {
        let at = cursor;
        cursor += pkg_reg_sectors;
        Some(at)
    };
    if cursor != total {
        return Err(DecryptError::Corrupt);
    }

    let mut fat = vec![FREE; fat_sectors * 128];
    for slot in fat.iter_mut().take(fat_sectors) {
        *slot = FAT_SECT;
    }
    fat[dir_at] = END;
    chain(&mut fat, mini_fat_at, mini_fat_sectors);
    chain(&mut fat, mini_stream_at, mini_stream_sectors);
    chain(&mut fat, info_reg_at, info_reg_sectors);
    chain(&mut fat, pkg_reg_at, pkg_reg_sectors);

    let mut file = vec![0u8; (1 + total) * SS];
    file[..8].copy_from_slice(&MAGIC);
    file[0x18..0x1A].copy_from_slice(&0x003Eu16.to_le_bytes());
    file[0x1A..0x1C].copy_from_slice(&3u16.to_le_bytes());
    file[0x1C..0x1E].copy_from_slice(&0xFFFEu16.to_le_bytes());
    file[0x1E..0x20].copy_from_slice(&9u16.to_le_bytes());
    file[0x20..0x22].copy_from_slice(&6u16.to_le_bytes());
    file[0x2C..0x30].copy_from_slice(&(fat_sectors as u32).to_le_bytes());
    file[0x30..0x34].copy_from_slice(&(dir_at as u32).to_le_bytes());
    file[0x38..0x3C].copy_from_slice(&CUTOFF.to_le_bytes());
    let mini_fat_id = mini_fat_at.map(|n| n as u32).unwrap_or(END);
    file[0x3C..0x40].copy_from_slice(&mini_fat_id.to_le_bytes());
    file[0x40..0x44].copy_from_slice(&(mini_fat_sectors as u32).to_le_bytes());
    file[0x44..0x48].copy_from_slice(&END.to_le_bytes());
    for i in 0..109 {
        let id = if i < fat_sectors { i as u32 } else { FREE };
        file[0x4C + i * 4..0x50 + i * 4].copy_from_slice(&id.to_le_bytes());
    }

    for s in 0..fat_sectors {
        let base = (1 + s) * SS;
        for j in 0..128 {
            let v = fat[s * 128 + j];
            file[base + j * 4..base + j * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
    }

    if let Some(at) = mini_fat_at {
        let base = (1 + at) * SS;
        for j in 0..128 {
            let v = mini_fat.get(j).copied().unwrap_or(FREE);
            file[base + j * 4..base + j * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    if let Some(at) = mini_stream_at {
        copy_bytes(&mut file, at, &mini_data);
    }
    if let Some(at) = info_reg_at {
        copy_bytes(&mut file, at, info);
    }
    if let Some(at) = pkg_reg_at {
        copy_bytes(&mut file, at, package);
    }

    let dir = (1 + dir_at) * SS;
    let root_start = mini_stream_at.map(|n| n as u32).unwrap_or(END);
    put_dir(
        &mut file[dir..dir + 128],
        DirSpec {
            name: "Root Entry",
            ty: 5,
            left: FREE,
            right: FREE,
            child: 1,
            start: root_start,
            size: mini_data.len() as u64,
        },
    );
    let info_start = if info_mini {
        info_mini_start
    } else {
        info_reg_at.ok_or(DecryptError::Corrupt)? as u32
    };
    let pkg_start = if pkg_mini {
        pkg_mini_start
    } else {
        pkg_reg_at.ok_or(DecryptError::Corrupt)? as u32
    };
    put_dir(
        &mut file[dir + 128..dir + 256],
        DirSpec {
            name: "EncryptionInfo",
            ty: 2,
            left: 2,
            right: FREE,
            child: FREE,
            start: info_start,
            size: info.len() as u64,
        },
    );
    put_dir(
        &mut file[dir + 256..dir + 384],
        DirSpec {
            name: "EncryptedPackage",
            ty: 2,
            left: FREE,
            right: FREE,
            child: FREE,
            start: pkg_start,
            size: package.len() as u64,
        },
    );
    Ok(file)
}

fn push_mini(data: &mut Vec<u8>, fat: &mut Vec<u32>, bytes: &[u8]) -> u32 {
    if bytes.is_empty() {
        return END;
    }
    let start = fat.len() as u32;
    let mut off = 0;
    while off < bytes.len() {
        let mut sec = [0u8; 64];
        let n = (bytes.len() - off).min(64);
        sec[..n].copy_from_slice(&bytes[off..off + n]);
        data.extend_from_slice(&sec);
        fat.push(END);
        off += n;
    }
    let end = fat.len() as u32;
    let mut i = start;
    while i + 1 < end {
        fat[i as usize] = i + 1;
        i += 1;
    }
    start
}

fn chain(fat: &mut [u32], start: Option<usize>, n: usize) {
    let Some(start) = start else {
        return;
    };
    if n == 0 {
        return;
    }
    for i in 0..n - 1 {
        fat[start + i] = (start + i + 1) as u32;
    }
    fat[start + n - 1] = END;
}

fn copy_bytes(file: &mut [u8], sector: usize, data: &[u8]) {
    let mut off = 0;
    let mut s = sector;
    while off < data.len() {
        let base = (1 + s) * 512;
        let n = (data.len() - off).min(512);
        file[base..base + n].copy_from_slice(&data[off..off + n]);
        off += n;
        s += 1;
    }
}

fn fat_len(fixed: usize) -> Result<usize, DecryptError> {
    let mut fat = 1usize;
    loop {
        let total = fat + fixed;
        let need = total.div_ceil(128);
        if need <= fat {
            if fat > 109 {
                return Err(DecryptError::TooLarge);
            }
            return Ok(fat);
        }
        fat = need;
        if fat > 109 {
            return Err(DecryptError::TooLarge);
        }
    }
}

struct DirSpec {
    name: &'static str,
    ty: u8,
    left: u32,
    right: u32,
    child: u32,
    start: u32,
    size: u64,
}

fn put_dir(entry: &mut [u8], spec: DirSpec) {
    entry[..128].fill(0);
    let mut n = 0;
    for unit in spec.name.encode_utf16() {
        entry[n..n + 2].copy_from_slice(&unit.to_le_bytes());
        n += 2;
    }
    entry[64..66].copy_from_slice(&((n + 2) as u16).to_le_bytes());
    entry[66] = spec.ty;
    entry[67] = 1;
    entry[68..72].copy_from_slice(&spec.left.to_le_bytes());
    entry[72..76].copy_from_slice(&spec.right.to_le_bytes());
    entry[76..80].copy_from_slice(&spec.child.to_le_bytes());
    entry[116..120].copy_from_slice(&spec.start.to_le_bytes());
    entry[120..128].copy_from_slice(&spec.size.to_le_bytes());
}
