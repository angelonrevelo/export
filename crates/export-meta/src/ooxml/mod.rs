//! Decrypt an ECMA-376 standard-encrypted OOXML package.
//!
//! A password-protected docx, xlsx, or pptx is a Compound File with two streams:
//! `EncryptionInfo` and `EncryptedPackage`. The package stream is the zip, AES-128-ECB
//! under a key stretched from the password with SHA-1. This reader seeks to each
//! sector, decrypts 16-byte blocks, and writes them out. The plaintext is not buffered.
//!
//! # What a wrong password does
//!
//! The encrypted verifier is checked before the package stream is touched. A mismatch
//! is [`DecryptError::WrongPassword`] and the output is left alone.
//!
//! # Honest limits
//!
//! - Standard encryption only: version minor 2, AES-128 (`0x660E`), SHA-1 (`0x8004`),
//!   50,000 iterations, one key for the whole package (ECB). Agile (4.4) and extensible
//!   (minor 3) return [`DecryptError::Unsupported`], which is not a wrong password.
//! - No RC4, no AES-192, no AES-256. Software AES, no AES-NI.
//! - The password check is the verifier hash. There is no payload HMAC, so a bit flipped
//!   in `EncryptedPackage` still decrypts, to different bytes.
//! - `EncryptionInfo` is read whole and refused above 64 KiB. The package is not.
//! - Compound File major version 3, 512-byte sectors. A 4096-byte sector file is
//!   [`DecryptError::Unsupported`]. One FAT sector, one mini-FAT sector, and one data
//!   sector are cached. A truncated package after a correct password can already have
//!   written a prefix.
//! - [`encrypt_ooxml`] builds the fixture. It holds the ciphertext. [`decrypt_ooxml`]
//!   does not hold the plaintext.
//! - The inner zip is not parsed. docx, xlsx, and pptx are the same container.

mod cfb;

use crate::aes::Aes128;
use crate::sha1::sha1;
use std::io::{self, Read, Seek, Write};

const CSP: &str = "Microsoft Enhanced RSA and AES Cryptographic Provider";
const ITER: u32 = 50_000;

#[derive(Debug)]
pub enum DecryptError {
    WrongPassword,
    NotEncrypted,
    Truncated,
    Corrupt,
    TooLarge,
    Unsupported(&'static str),
    Io(io::Error),
}

impl DecryptError {
    fn from_io(err: io::Error) -> Self {
        if err.kind() == io::ErrorKind::UnexpectedEof {
            Self::Truncated
        } else {
            Self::Io(err)
        }
    }
}

impl std::fmt::Display for DecryptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongPassword => f.write_str("wrong password"),
            Self::NotEncrypted => f.write_str("not an encrypted OOXML package"),
            Self::Truncated => f.write_str("truncated encrypted package"),
            Self::Corrupt => f.write_str("encrypted package is corrupt"),
            Self::TooLarge => f.write_str("encrypted package exceeds a size limit"),
            Self::Unsupported(what) => write!(f, "unsupported OOXML encryption: {what}"),
            Self::Io(err) => write!(f, "io: {err}"),
        }
    }
}

impl std::error::Error for DecryptError {}

/// Wrap `plaintext` as a standard-encrypted OOXML package.
///
/// `salt` is the 16-byte verifier salt. The same password and salt always
/// produce the same file for a given plaintext.
pub fn encrypt_ooxml(
    password: &str,
    salt: &[u8; 16],
    plaintext: &[u8],
    out: &mut impl Write,
) -> Result<(), DecryptError> {
    let key = derive_key(password, salt);
    let aes = Aes128::new(&key);
    let info = encryption_info(salt, &aes);
    let package = seal(&aes, plaintext);
    let file = cfb::build(&info, &package)?;
    out.write_all(&file).map_err(DecryptError::from_io)?;
    Ok(())
}

/// Decrypt a standard-encrypted OOXML package into `out`.
///
/// Seeks within `input`. Writes plaintext only after the password verifier matches.
/// A wrong password returns [`DecryptError::WrongPassword`] and does not write.
pub fn decrypt_ooxml<R: Read + Seek, W: Write>(
    input: &mut R,
    password: &str,
    out: &mut W,
) -> Result<(), DecryptError> {
    let mut cfb = cfb::Cfb::open(input)?;
    let (info_loc, pkg_loc) = cfb.encryption_streams()?;
    let info = cfb.read_entire(&info_loc)?;
    let parsed = parse_info(&info)?;
    let key = derive_key(password, &parsed.salt);
    let aes = Aes128::new(&key);
    if !verifier_ok(&aes, &parsed.enc_verifier, &parsed.enc_hash) {
        return Err(DecryptError::WrongPassword);
    }
    cfb.open_seq(&pkg_loc);
    let mut size_buf = [0u8; 8];
    cfb.read_exact(&mut size_buf)?;
    let total = u64::from_le_bytes(size_buf);
    let padded = total.checked_add(15).ok_or(DecryptError::Corrupt)? & !15u64;
    let need = 8u64.checked_add(padded).ok_or(DecryptError::Corrupt)?;
    if need > pkg_loc.size {
        return Err(DecryptError::Truncated);
    }
    let mut left = total;
    let mut block = [0u8; 16];
    while left > 0 {
        cfb.read_exact(&mut block)?;
        let plain = aes.decrypt_block(&block);
        let n = usize::try_from(left.min(16)).unwrap_or(16);
        out.write_all(&plain[..n]).map_err(DecryptError::from_io)?;
        left -= n as u64;
    }
    Ok(())
}

struct Parsed {
    salt: [u8; 16],
    enc_verifier: [u8; 16],
    enc_hash: [u8; 32],
}

fn parse_info(data: &[u8]) -> Result<Parsed, DecryptError> {
    if data.len() < 4 {
        return Err(DecryptError::Truncated);
    }
    let major = u16::from_le_bytes([data[0], data[1]]);
    let minor = u16::from_le_bytes([data[2], data[3]]);
    if major == 4 && minor == 4 {
        return Err(DecryptError::Unsupported("agile"));
    }
    if minor == 3 && (major == 3 || major == 4) {
        return Err(DecryptError::Unsupported("extensible"));
    }
    if !(2..=4).contains(&major) || minor != 2 {
        return Err(DecryptError::Unsupported("version"));
    }
    if data.len() < 12 {
        return Err(DecryptError::Truncated);
    }
    let header_size = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
    if header_size < 32 || 12 + header_size > data.len() {
        return Err(DecryptError::Corrupt);
    }
    let header = &data[12..12 + header_size];
    let alg = u32::from_le_bytes(header[8..12].try_into().unwrap());
    let alg_hash = u32::from_le_bytes(header[12..16].try_into().unwrap());
    let key_size = u32::from_le_bytes(header[16..20].try_into().unwrap());
    if alg != 0x660E || alg_hash != 0x8004 || key_size != 128 {
        return Err(DecryptError::Unsupported("cipher"));
    }
    let rest = &data[12 + header_size..];
    if rest.len() < 72 {
        return Err(DecryptError::Truncated);
    }
    let salt_size = u32::from_le_bytes(rest[0..4].try_into().unwrap());
    if salt_size != 16 {
        return Err(DecryptError::Corrupt);
    }
    let hash_size = u32::from_le_bytes(rest[36..40].try_into().unwrap());
    if hash_size != 20 {
        return Err(DecryptError::Corrupt);
    }
    let mut salt = [0u8; 16];
    let mut enc_verifier = [0u8; 16];
    let mut enc_hash = [0u8; 32];
    salt.copy_from_slice(&rest[4..20]);
    enc_verifier.copy_from_slice(&rest[20..36]);
    enc_hash.copy_from_slice(&rest[40..72]);
    Ok(Parsed {
        salt,
        enc_verifier,
        enc_hash,
    })
}

fn derive_key(password: &str, salt: &[u8]) -> [u8; 16] {
    let mut pw = Vec::with_capacity(password.len() * 2);
    for unit in password.encode_utf16() {
        pw.extend_from_slice(&unit.to_le_bytes());
    }
    let mut first = Vec::with_capacity(salt.len() + pw.len());
    first.extend_from_slice(salt);
    first.extend_from_slice(&pw);
    let mut h = sha1(&first);
    for i in 0..ITER {
        let mut block = [0u8; 24];
        block[..4].copy_from_slice(&i.to_le_bytes());
        block[4..].copy_from_slice(&h);
        h = sha1(&block);
    }
    let mut last = [0u8; 24];
    last[..20].copy_from_slice(&h);
    last[20..].copy_from_slice(&0u32.to_le_bytes());
    let hfinal = sha1(&last);
    let mut b1 = [0x36u8; 64];
    let mut b2 = [0x5cu8; 64];
    for i in 0..20 {
        b1[i] ^= hfinal[i];
        b2[i] ^= hfinal[i];
    }
    let x1 = sha1(&b1);
    let _x2 = sha1(&b2);
    let mut key = [0u8; 16];
    key.copy_from_slice(&x1[..16]);
    key
}

fn verifier_ok(aes: &Aes128, enc_verifier: &[u8; 16], enc_hash: &[u8; 32]) -> bool {
    let verifier = aes.decrypt_block(enc_verifier);
    let mut hash_block = [0u8; 16];
    hash_block.copy_from_slice(&enc_hash[..16]);
    let h1 = aes.decrypt_block(&hash_block);
    hash_block.copy_from_slice(&enc_hash[16..]);
    let h2 = aes.decrypt_block(&hash_block);
    let expect = sha1(&verifier);
    let mut diff = 0u8;
    for i in 0..16 {
        diff |= h1[i] ^ expect[i];
    }
    diff |= h2[0] ^ expect[16];
    diff |= h2[1] ^ expect[17];
    diff |= h2[2] ^ expect[18];
    diff |= h2[3] ^ expect[19];
    diff == 0
}

fn encryption_info(salt: &[u8; 16], aes: &Aes128) -> Vec<u8> {
    let verifier = *b"export-meta-vrfy";
    let hash = sha1(&verifier);
    let mut hash32 = [0u8; 32];
    hash32[..20].copy_from_slice(&hash);
    let enc_v = aes.encrypt_block(&verifier);
    let mut left = [0u8; 16];
    let mut right = [0u8; 16];
    left.copy_from_slice(&hash32[..16]);
    right.copy_from_slice(&hash32[16..]);
    let h1 = aes.encrypt_block(&left);
    let h2 = aes.encrypt_block(&right);

    let mut csp = Vec::new();
    for unit in CSP.encode_utf16().chain(std::iter::once(0)) {
        csp.extend_from_slice(&unit.to_le_bytes());
    }
    let header_size = (32 + csp.len()) as u32;
    let mut out = Vec::with_capacity(12 + header_size as usize + 72);
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&0x24u32.to_le_bytes());
    out.extend_from_slice(&header_size.to_le_bytes());
    out.extend_from_slice(&0x24u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0x660Eu32.to_le_bytes());
    out.extend_from_slice(&0x8004u32.to_le_bytes());
    out.extend_from_slice(&128u32.to_le_bytes());
    out.extend_from_slice(&0x18u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&csp);
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(salt);
    out.extend_from_slice(&enc_v);
    out.extend_from_slice(&20u32.to_le_bytes());
    out.extend_from_slice(&h1);
    out.extend_from_slice(&h2);
    out
}

fn seal(aes: &Aes128, plain: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + plain.len() + 16);
    out.extend_from_slice(&(plain.len() as u64).to_le_bytes());
    let mut off = 0;
    while off < plain.len() {
        let mut block = [0u8; 16];
        let n = (plain.len() - off).min(16);
        block[..n].copy_from_slice(&plain[off..off + n]);
        out.extend_from_slice(&aes.encrypt_block(&block));
        off += 16;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, SeekFrom};

    const SALT: [u8; 16] = [
        0xe8, 0x82, 0x66, 0x49, 0x0c, 0x5b, 0xd1, 0xee, 0xbd, 0x2b, 0x43, 0x94, 0xe3, 0xf8, 0x30,
        0xef,
    ];

    #[test]
    fn published_key_and_verifier() {
        let key = derive_key("Password1234_", &SALT);
        assert_eq!(
            key,
            [
                0x40, 0xb1, 0x3a, 0x71, 0xf9, 0x0b, 0x96, 0x6e, 0x37, 0x54, 0x08, 0xf2, 0xd1, 0x81,
                0xa1, 0xaa
            ]
        );
        let enc_v = [
            0x51, 0x6f, 0x73, 0x2e, 0x96, 0x6f, 0xac, 0x17, 0xb1, 0xc5, 0xd7, 0xd8, 0xcc, 0x36,
            0xc9, 0x28,
        ];
        let enc_h = [
            0x2b, 0x61, 0x68, 0xda, 0xbe, 0x29, 0x11, 0xad, 0x2b, 0xd3, 0x7c, 0x17, 0x46, 0x74,
            0x5c, 0x14, 0xd3, 0xcf, 0x1b, 0xb1, 0x40, 0xa4, 0x8f, 0x4e, 0x6f, 0x3d, 0x23, 0x88,
            0x08, 0x72, 0xb1, 0x6a,
        ];
        assert!(verifier_ok(&Aes128::new(&key), &enc_v, &enc_h));
        let mut bad = key;
        bad[0] ^= 1;
        assert!(!verifier_ok(&Aes128::new(&bad), &enc_v, &enc_h));
    }

    #[test]
    fn tiny_docx_roundtrip_and_wrong_password() {
        let plain = stored_zip(&[("[Content_Types].xml", TYPES), ("word/document.xml", DOC)]);
        assert!(plain.len() < 4096);
        assert!(plain.windows(4).any(|w| w == b"PK\x03\x04"));
        let mut file = Vec::new();
        encrypt_ooxml("secret", &SALT, &plain, &mut file).unwrap();
        let mut out = Vec::new();
        decrypt_ooxml(&mut Cursor::new(file.clone()), "secret", &mut out).unwrap();
        assert_eq!(out, plain);

        let mut bomb = Bomb;
        let err = decrypt_ooxml(&mut Cursor::new(file), "nope", &mut bomb).unwrap_err();
        assert!(matches!(err, DecryptError::WrongPassword));
        assert_eq!(err.to_string(), "wrong password");
    }

    #[test]
    fn package_above_mini_cutoff_and_slow_reads() {
        let mut plain = vec![0x5au8; 5000];
        plain[..4].copy_from_slice(b"PK\x03\x04");
        let mut file = Vec::new();
        encrypt_ooxml("päss", &SALT, &plain, &mut file).unwrap();
        let mut slow = Slow {
            inner: Cursor::new(file),
        };
        let mut out = Vec::new();
        decrypt_ooxml(&mut slow, "päss", &mut out).unwrap();
        assert_eq!(out, plain);
    }

    #[test]
    fn agile_is_not_a_wrong_password() {
        let mut file = Vec::new();
        let bogus = cfb::build(&[0x04, 0x00, 0x04, 0x00], &[0; 8]).unwrap();
        file.extend_from_slice(&bogus);
        let err = decrypt_ooxml(&mut Cursor::new(file), "secret", &mut Bomb).unwrap_err();
        assert!(matches!(err, DecryptError::Unsupported("agile")));
    }

    #[test]
    fn plain_zip_is_not_encrypted() {
        let err = decrypt_ooxml(&mut Cursor::new(&b"PK\x03\x04"[..]), "x", &mut Bomb).unwrap_err();
        assert!(matches!(err, DecryptError::NotEncrypted));
    }

    #[test]
    fn truncated_package_errors() {
        let plain = vec![0x11u8; 6000];
        let mut file = Vec::new();
        encrypt_ooxml("secret", &SALT, &plain, &mut file).unwrap();
        file.pop();
        let err = decrypt_ooxml(&mut Cursor::new(file), "secret", &mut Vec::new()).unwrap_err();
        assert!(matches!(err, DecryptError::Truncated));
    }

    struct Bomb;
    impl Write for Bomb {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            panic!("plaintext written");
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct Slow<T> {
        inner: T,
    }
    impl<T: Read> Read for Slow<T> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            self.inner.read(&mut buf[..1])
        }
    }
    impl<T: Seek> Seek for Slow<T> {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.inner.seek(pos)
        }
    }

    const TYPES: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    const DOC: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>hello-ooxml</w:t></w:r></w:p></w:body></w:document>"#;

    fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut locals = Vec::new();
        let mut central = Vec::new();
        for (name, data) in files {
            let crc = crate::crc::crc32(data);
            let offset = locals.len() as u32;
            locals.extend_from_slice(&0x04034b50u32.to_le_bytes());
            locals.extend_from_slice(&20u16.to_le_bytes());
            locals.extend_from_slice(&0u16.to_le_bytes());
            locals.extend_from_slice(&0u16.to_le_bytes());
            locals.extend_from_slice(&0u16.to_le_bytes());
            locals.extend_from_slice(&0u16.to_le_bytes());
            locals.extend_from_slice(&crc.to_le_bytes());
            locals.extend_from_slice(&(data.len() as u32).to_le_bytes());
            locals.extend_from_slice(&(data.len() as u32).to_le_bytes());
            locals.extend_from_slice(&(name.len() as u16).to_le_bytes());
            locals.extend_from_slice(&0u16.to_le_bytes());
            locals.extend_from_slice(name.as_bytes());
            locals.extend_from_slice(data);

            central.extend_from_slice(&0x02014b50u32.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let mut out = locals;
        let cd_off = out.len() as u32;
        let cd_len = central.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x06054b50u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        let n = files.len() as u16;
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&cd_len.to_le_bytes());
        out.extend_from_slice(&cd_off.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }
}
