//! ECMA-376 agile encryption (EncryptionInfo 4.4): AES-256-CBC and SHA-512.
//!
//! Key derivation is locked to the published `Password1234_` vectors: the
//! unwrapped package key `40206609…2ac2`, and the verifier pair that hashes
//! to itself. Spin counts above 1,000,000 are refused. Other ciphers are
//! refused. The fixture writer uses fixed verifier and package-key bytes.

use super::cfb::{Cfb, StreamLoc};
use super::DecryptError;
use crate::aes::Aes256;
use crate::sha512::{sha512, HmacSha512};
use std::io::{Read, Seek, Write};

const SPIN_CAP: u32 = 1_000_000;
const SPIN: u32 = 100_000;
const BLK_VERIFIER: [u8; 8] = [0xfe, 0xa7, 0xd2, 0x76, 0x3b, 0x4b, 0x9e, 0x79];
const BLK_HASH: [u8; 8] = [0xd7, 0xaa, 0x0f, 0x6d, 0x30, 0x61, 0x34, 0x4e];
const BLK_KEY: [u8; 8] = [0x14, 0x6e, 0x0b, 0xe7, 0xab, 0xac, 0xd0, 0xd6];
const BLK_HMAC1: [u8; 8] = [0x5f, 0xb2, 0xad, 0x01, 0x0c, 0xb9, 0xe1, 0xf6];
const BLK_HMAC2: [u8; 8] = [0xa0, 0x67, 0x7f, 0x02, 0xb2, 0x2c, 0x84, 0x33];
const VERIFIER: [u8; 16] = *b"agile-verifier!!";
const SECRET: [u8; 32] = *b"0123456789abcdef0123456789abcdef";
const HMAC_SALT: [u8; 64] = [0xa5; 64];

struct AgileInfo {
    spin: u32,
    password_salt: [u8; 16],
    key_salt: [u8; 16],
    enc_verifier: [u8; 16],
    enc_hash: [u8; 64],
    enc_key: [u8; 32],
    enc_hmac_key: [u8; 64],
    enc_hmac_val: [u8; 64],
}

pub(super) fn is_agile(info: &[u8]) -> bool {
    info.len() >= 4 && info[0] == 4 && info[1] == 0 && info[2] == 4 && info[3] == 0
}

pub fn encrypt_ooxml_agile(
    password: &str,
    password_salt: &[u8; 16],
    key_salt: &[u8; 16],
    plaintext: &[u8],
    out: &mut impl Write,
) -> Result<(), DecryptError> {
    let h = iterated(password, password_salt, SPIN);
    let k1 = Aes256::new(&enc_key(&h, &BLK_VERIFIER));
    let k2 = Aes256::new(&enc_key(&h, &BLK_HASH));
    let k3 = Aes256::new(&enc_key(&h, &BLK_KEY));
    let mut enc_verifier = [0u8; 16];
    let hash = sha512(&VERIFIER);
    let mut enc_hash = [0u8; 64];
    let mut enc_secret = [0u8; 32];
    cbc_encrypt(&k1, password_salt, &VERIFIER, &mut enc_verifier)?;
    cbc_encrypt(&k2, password_salt, &hash, &mut enc_hash)?;
    cbc_encrypt(&k3, password_salt, &SECRET, &mut enc_secret)?;
    let secret = Aes256::new(&SECRET);
    let package = seal(&secret, key_salt, plaintext)?;
    let iv1 = iv_from(key_salt, &BLK_HMAC1);
    let iv2 = iv_from(key_salt, &BLK_HMAC2);
    let mut enc_hmac_key = [0u8; 64];
    let mut enc_hmac_val = [0u8; 64];
    cbc_encrypt(&secret, &iv1, &HMAC_SALT, &mut enc_hmac_key)?;
    let mut mac = HmacSha512::new(&HMAC_SALT);
    mac.update(&package);
    cbc_encrypt(&secret, &iv2, &mac.finish(), &mut enc_hmac_val)?;
    let info = descriptor(
        password_salt,
        key_salt,
        &enc_verifier,
        &enc_hash,
        &enc_secret,
        &enc_hmac_key,
        &enc_hmac_val,
    );
    let file = super::cfb::build(&info, &package)?;
    out.write_all(&file).map_err(DecryptError::from_io)?;
    Ok(())
}

pub(super) fn decrypt<R: Read + Seek, W: Write>(
    cfb: &mut Cfb<R>,
    info: &[u8],
    pkg: &StreamLoc,
    password: &str,
    out: &mut W,
) -> Result<(), DecryptError> {
    let parsed = parse(info)?;
    let h = iterated(password, &parsed.password_salt, parsed.spin);
    let k1 = Aes256::new(&enc_key(&h, &BLK_VERIFIER));
    let k2 = Aes256::new(&enc_key(&h, &BLK_HASH));
    let mut hash_input = [0u8; 16];
    cbc_decrypt(
        &k1,
        &parsed.password_salt,
        &parsed.enc_verifier,
        &mut hash_input,
    )?;
    let actual = sha512(&hash_input);
    let mut expected = [0u8; 64];
    cbc_decrypt(&k2, &parsed.password_salt, &parsed.enc_hash, &mut expected)?;
    if !ct_eq(&actual, &expected) {
        return Err(DecryptError::WrongPassword);
    }
    let k3 = Aes256::new(&enc_key(&h, &BLK_KEY));
    let mut secret_bytes = [0u8; 32];
    cbc_decrypt(
        &k3,
        &parsed.password_salt,
        &parsed.enc_key,
        &mut secret_bytes,
    )?;
    let secret = Aes256::new(&secret_bytes);
    let iv1 = iv_from(&parsed.key_salt, &BLK_HMAC1);
    let iv2 = iv_from(&parsed.key_salt, &BLK_HMAC2);
    let mut hmac_key = [0u8; 64];
    let mut hmac_value = [0u8; 64];
    cbc_decrypt(&secret, &iv1, &parsed.enc_hmac_key, &mut hmac_key)?;
    cbc_decrypt(&secret, &iv2, &parsed.enc_hmac_val, &mut hmac_value)?;
    let got = hmac_stream(cfb, pkg, &hmac_key)?;
    if !ct_eq(&got, &hmac_value) {
        return Err(DecryptError::Corrupt);
    }
    cfb.open_seq(pkg);
    let mut size_buf = [0u8; 8];
    cfb.read_exact(&mut size_buf)?;
    let total = u64::from_le_bytes(size_buf);
    if cipher_need(total)? > pkg.size {
        return Err(DecryptError::Truncated);
    }
    let mut left = total;
    let mut index = 0u32;
    let mut cipher = [0u8; 4096];
    let mut plain = [0u8; 4096];
    while left > 0 {
        let n = left.min(4096);
        let cn = ((n + 15) & !15) as usize;
        cfb.read_exact(&mut cipher[..cn])?;
        let iv = iv_from(&parsed.key_salt, &index.to_le_bytes());
        cbc_decrypt(&secret, &iv, &cipher[..cn], &mut plain[..cn])?;
        out.write_all(&plain[..n as usize])
            .map_err(DecryptError::from_io)?;
        left -= n;
        index = index.checked_add(1).ok_or(DecryptError::Corrupt)?;
    }
    Ok(())
}

fn iterated(password: &str, salt: &[u8; 16], spin: u32) -> [u8; 64] {
    let mut pw = Vec::with_capacity(password.len() * 2);
    for unit in password.encode_utf16() {
        pw.extend_from_slice(&unit.to_le_bytes());
    }
    let mut first = Vec::with_capacity(salt.len() + pw.len());
    first.extend_from_slice(salt);
    first.extend_from_slice(&pw);
    let mut h = sha512(&first);
    for i in 0..spin {
        let mut block = [0u8; 68];
        block[..4].copy_from_slice(&i.to_le_bytes());
        block[4..].copy_from_slice(&h);
        h = sha512(&block);
    }
    h
}

fn enc_key(h: &[u8; 64], block: &[u8; 8]) -> [u8; 32] {
    let mut buf = [0u8; 72];
    buf[..64].copy_from_slice(h);
    buf[64..].copy_from_slice(block);
    let digest = sha512(&buf);
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest[..32]);
    key
}

fn iv_from(salt: &[u8; 16], extra: &[u8]) -> [u8; 16] {
    let mut buf = [0u8; 24];
    buf[..16].copy_from_slice(salt);
    buf[16..16 + extra.len()].copy_from_slice(extra);
    let digest = sha512(&buf[..16 + extra.len()]);
    let mut iv = [0u8; 16];
    iv.copy_from_slice(&digest[..16]);
    iv
}

fn cbc_encrypt(
    aes: &Aes256,
    iv: &[u8; 16],
    data: &[u8],
    out: &mut [u8],
) -> Result<(), DecryptError> {
    if data.len() % 16 != 0 || out.len() < data.len() {
        return Err(DecryptError::Corrupt);
    }
    let mut prev = *iv;
    let mut off = 0;
    while off < data.len() {
        let mut block = [0u8; 16];
        for j in 0..16 {
            block[j] = data[off + j] ^ prev[j];
        }
        let enc = aes.encrypt_block(&block);
        out[off..off + 16].copy_from_slice(&enc);
        prev = enc;
        off += 16;
    }
    Ok(())
}

fn cbc_decrypt(
    aes: &Aes256,
    iv: &[u8; 16],
    data: &[u8],
    out: &mut [u8],
) -> Result<(), DecryptError> {
    if data.len() % 16 != 0 || out.len() < data.len() {
        return Err(DecryptError::Corrupt);
    }
    let mut prev = *iv;
    let mut off = 0;
    while off < data.len() {
        let mut block = [0u8; 16];
        block.copy_from_slice(&data[off..off + 16]);
        let dec = aes.decrypt_block(&block);
        for j in 0..16 {
            out[off + j] = dec[j] ^ prev[j];
        }
        prev = block;
        off += 16;
    }
    Ok(())
}

fn seal(secret: &Aes256, key_salt: &[u8; 16], plain: &[u8]) -> Result<Vec<u8>, DecryptError> {
    let mut out = Vec::with_capacity(8 + plain.len() + 16);
    out.extend_from_slice(&(plain.len() as u64).to_le_bytes());
    let mut off = 0;
    let mut index = 0u32;
    while off < plain.len() {
        let n = (plain.len() - off).min(4096);
        let cn = (n + 15) & !15;
        let mut block = [0u8; 4096];
        block[..n].copy_from_slice(&plain[off..off + n]);
        let mut enc = [0u8; 4096];
        let iv = iv_from(key_salt, &index.to_le_bytes());
        cbc_encrypt(secret, &iv, &block[..cn], &mut enc[..cn])?;
        out.extend_from_slice(&enc[..cn]);
        off += n;
        index += 1;
    }
    Ok(out)
}

fn cipher_need(total: u64) -> Result<u64, DecryptError> {
    let full = total / 4096;
    let rem = total % 4096;
    let body = full.checked_mul(4096).ok_or(DecryptError::Corrupt)?;
    let tail = if rem == 0 { 0 } else { (rem + 15) & !15 };
    body.checked_add(tail)
        .and_then(|n| n.checked_add(8))
        .ok_or(DecryptError::Corrupt)
}

fn hmac_stream<R: Read + Seek>(
    cfb: &mut Cfb<R>,
    pkg: &StreamLoc,
    key: &[u8],
) -> Result<[u8; 64], DecryptError> {
    cfb.open_seq(pkg);
    let mut mac = HmacSha512::new(key);
    let mut buf = [0u8; 4096];
    let mut left = pkg.size;
    while left > 0 {
        let n = usize::try_from(left.min(4096)).unwrap_or(4096);
        cfb.read_exact(&mut buf[..n])?;
        mac.update(&buf[..n]);
        left -= u64::try_from(n).unwrap_or(0);
    }
    Ok(mac.finish())
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn parse(info: &[u8]) -> Result<AgileInfo, DecryptError> {
    if info.len() < 8 {
        return Err(DecryptError::Truncated);
    }
    let reserved = u32::from_le_bytes([info[4], info[5], info[6], info[7]]);
    if reserved != 0x40 {
        return Err(DecryptError::Corrupt);
    }
    let xml = std::str::from_utf8(&info[8..]).map_err(|_| DecryptError::Corrupt)?;
    let xml = xml.strip_prefix('\u{feff}').unwrap_or(xml);
    let key_data = tag_body(xml, "keyData")?;
    let integrity = tag_body(xml, "dataIntegrity")?;
    let enc = password_tag(xml)?;
    require_profile(key_data)?;
    require_profile(enc)?;
    let spin = attr_u32(enc, "spinCount")?;
    if spin > SPIN_CAP {
        return Err(DecryptError::Unsupported("spin count"));
    }
    let mut parsed = AgileInfo {
        spin,
        password_salt: [0; 16],
        key_salt: [0; 16],
        enc_verifier: [0; 16],
        enc_hash: [0; 64],
        enc_key: [0; 32],
        enc_hmac_key: [0; 64],
        enc_hmac_val: [0; 64],
    };
    b64_into(attr(enc, "saltValue")?, &mut parsed.password_salt)?;
    b64_into(attr(key_data, "saltValue")?, &mut parsed.key_salt)?;
    b64_into(
        attr(enc, "encryptedVerifierHashInput")?,
        &mut parsed.enc_verifier,
    )?;
    b64_into(
        attr(enc, "encryptedVerifierHashValue")?,
        &mut parsed.enc_hash,
    )?;
    b64_into(attr(enc, "encryptedKeyValue")?, &mut parsed.enc_key)?;
    b64_into(
        attr(integrity, "encryptedHmacKey")?,
        &mut parsed.enc_hmac_key,
    )?;
    b64_into(
        attr(integrity, "encryptedHmacValue")?,
        &mut parsed.enc_hmac_val,
    )?;
    Ok(parsed)
}

fn require_profile(tag: &str) -> Result<(), DecryptError> {
    if attr_u32(tag, "saltSize")? == 16
        && attr_u32(tag, "blockSize")? == 16
        && attr_u32(tag, "keyBits")? == 256
        && attr_u32(tag, "hashSize")? == 64
        && attr(tag, "cipherAlgorithm")? == "AES"
        && attr(tag, "cipherChaining")? == "ChainingModeCBC"
        && attr(tag, "hashAlgorithm")? == "SHA512"
    {
        Ok(())
    } else {
        Err(DecryptError::Unsupported("agile cipher"))
    }
}

fn password_tag(xml: &str) -> Result<&str, DecryptError> {
    let mut from = 0;
    while let Some(rel) = xml[from..].find("encryptedKey") {
        let at = from + rel;
        if tag_boundary(xml, at, "encryptedKey") {
            let body = tag_after(xml, at + "encryptedKey".len())?;
            if body.contains("spinCount") {
                return Ok(body);
            }
        }
        from = at + "encryptedKey".len();
    }
    Err(DecryptError::Corrupt)
}

fn tag_body<'a>(xml: &'a str, local: &str) -> Result<&'a str, DecryptError> {
    let mut from = 0;
    while let Some(rel) = xml[from..].find(local) {
        let at = from + rel;
        if tag_boundary(xml, at, local) {
            return tag_after(xml, at + local.len());
        }
        from = at + local.len();
    }
    Err(DecryptError::Corrupt)
}

fn tag_boundary(xml: &str, at: usize, local: &str) -> bool {
    let before_ok = at == 0 || matches!(xml.as_bytes()[at - 1], b'<' | b':');
    let after = xml.as_bytes().get(at + local.len()).copied();
    let after_ok = matches!(after, Some(b' ' | b'\n' | b'\r' | b'\t' | b'/' | b'>'));
    before_ok && after_ok
}

fn tag_after(xml: &str, from: usize) -> Result<&str, DecryptError> {
    let rest = &xml[from..];
    let end = rest.find('>').ok_or(DecryptError::Corrupt)?;
    Ok(&rest[..end])
}

fn attr<'a>(tag: &'a str, name: &str) -> Result<&'a str, DecryptError> {
    let mut rest = tag;
    while let Some(at) = rest.find(name) {
        let before = if at == 0 {
            b' '
        } else {
            rest.as_bytes()[at - 1]
        };
        let after = rest.as_bytes().get(at + name.len()).copied();
        if matches!(before, b' ' | b'\n' | b'\r' | b'\t') && after == Some(b'=') {
            let v = &rest[at + name.len() + 1..];
            let quote = *v.as_bytes().first().ok_or(DecryptError::Corrupt)?;
            if quote != b'"' && quote != b'\'' {
                return Err(DecryptError::Corrupt);
            }
            let v = &v[1..];
            let end = v.find(quote as char).ok_or(DecryptError::Corrupt)?;
            return Ok(&v[..end]);
        }
        rest = &rest[at + name.len()..];
    }
    Err(DecryptError::Corrupt)
}

fn attr_u32(tag: &str, name: &str) -> Result<u32, DecryptError> {
    let s = attr(tag, name)?;
    if s.is_empty() || s.len() > 10 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(DecryptError::Corrupt);
    }
    s.parse().map_err(|_| DecryptError::Corrupt)
}

fn b64_into(s: &str, out: &mut [u8]) -> Result<(), DecryptError> {
    let raw = s.as_bytes();
    if raw.is_empty() || raw.len() % 4 != 0 {
        return Err(DecryptError::Corrupt);
    }
    let mut n = 0;
    let mut i = 0;
    while i < raw.len() {
        let (a, b) = (b64_val(raw[i])?, b64_val(raw[i + 1])?);
        let c_pad = raw[i + 2] == b'=';
        let d_pad = raw[i + 3] == b'=';
        let c = if c_pad { 0 } else { b64_val(raw[i + 2])? };
        let d = if d_pad { 0 } else { b64_val(raw[i + 3])? };
        if c_pad && !d_pad {
            return Err(DecryptError::Corrupt);
        }
        let v = (a << 18) | (b << 12) | (c << 6) | d;
        let bytes = if c_pad {
            1
        } else if d_pad {
            2
        } else {
            3
        };
        if n + bytes > out.len() {
            return Err(DecryptError::Corrupt);
        }
        out[n] = (v >> 16) as u8;
        if bytes > 1 {
            out[n + 1] = (v >> 8) as u8;
        }
        if bytes > 2 {
            out[n + 2] = v as u8;
        }
        n += bytes;
        i += 4;
        if bytes < 3 && i != raw.len() {
            return Err(DecryptError::Corrupt);
        }
    }
    if n != out.len() {
        Err(DecryptError::Corrupt)
    } else {
        Ok(())
    }
}

fn b64_val(b: u8) -> Result<u32, DecryptError> {
    let v = match b {
        b'A'..=b'Z' => b - b'A',
        b'a'..=b'z' => b - b'a' + 26,
        b'0'..=b'9' => b - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return Err(DecryptError::Corrupt),
    };
    Ok(u32::from(v))
}

fn b64(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = (u32::from(data[i]) << 16) | (u32::from(data[i + 1]) << 8) | u32::from(data[i + 2]);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push(T[(n & 63) as usize] as char);
        i += 3;
    }
    if i < data.len() {
        let mut n = u32::from(data[i]) << 16;
        if data.len() - i == 2 {
            n |= u32::from(data[i + 1]) << 8;
        }
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        if data.len() - i == 2 {
            out.push(T[((n >> 6) & 63) as usize] as char);
            out.push('=');
        } else {
            out.push('=');
            out.push('=');
        }
    }
    out
}

fn descriptor(
    password_salt: &[u8; 16],
    key_salt: &[u8; 16],
    enc_verifier: &[u8; 16],
    enc_hash: &[u8; 64],
    enc_key: &[u8; 32],
    enc_hmac_key: &[u8; 64],
    enc_hmac_val: &[u8; 64],
) -> Vec<u8> {
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<encryption xmlns=\"http://schemas.microsoft.com/office/2006/encryption\" xmlns:p=\"http://schemas.microsoft.com/office/2006/keyEncryptor/password\">\
<keyData saltSize=\"16\" blockSize=\"16\" keyBits=\"256\" hashSize=\"64\" cipherAlgorithm=\"AES\" cipherChaining=\"ChainingModeCBC\" hashAlgorithm=\"SHA512\" saltValue=\"{}\"/>\
<dataIntegrity encryptedHmacKey=\"{}\" encryptedHmacValue=\"{}\"/>\
<keyEncryptors><keyEncryptor uri=\"http://schemas.microsoft.com/office/2006/keyEncryptor/password\">\
<p:encryptedKey spinCount=\"{SPIN}\" saltSize=\"16\" blockSize=\"16\" keyBits=\"256\" hashSize=\"64\" cipherAlgorithm=\"AES\" cipherChaining=\"ChainingModeCBC\" hashAlgorithm=\"SHA512\" saltValue=\"{}\" encryptedVerifierHashInput=\"{}\" encryptedVerifierHashValue=\"{}\" encryptedKeyValue=\"{}\"/>\
</keyEncryptor></keyEncryptors></encryption>",
        b64(key_salt),
        b64(enc_hmac_key),
        b64(enc_hmac_val),
        b64(password_salt),
        b64(enc_verifier),
        b64(enc_hash),
        b64(enc_key),
    );
    let mut out = Vec::with_capacity(8 + xml.len());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&0x40u32.to_le_bytes());
    out.extend_from_slice(xml.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn published_package_key() {
        let salt = hex16("4c725d45dc610f939412a04da7910466");
        let wrapped = hex32("a16cd5165a7ab9d271113ed386a78cf49692e8e527b0c5fc0055ed080b7cb94b");
        let expected = hex32("40206609d9faadf24b076aebf2c435b74292c8b8a7aa81bc679be89711b02ac2");
        let h = iterated("Password1234_", &salt, 100_000);
        let aes = Aes256::new(&enc_key(&h, &BLK_KEY));
        let mut got = [0u8; 32];
        cbc_decrypt(&aes, &salt, &wrapped, &mut got).unwrap();
        assert_eq!(got, expected);
    }

    #[test]
    fn published_verifier() {
        let salt = hex16("cbca1c999343fbad92075634150034b0");
        let input = hex16("39eea54e26e514798c284bc7714d38ac");
        let hash_ct = hex64("14376d6d817334e6b0ff4fd8221a7c678e5d8a784e8f999f4c188930c36a4b29c5b333605b5cd403b05003adcf18cca8cbab8debe373c65604a0becfae5c0ad0");
        let h = iterated("Password1234_", &salt, 100_000);
        let k1 = Aes256::new(&enc_key(&h, &BLK_VERIFIER));
        let k2 = Aes256::new(&enc_key(&h, &BLK_HASH));
        let mut plain = [0u8; 16];
        cbc_decrypt(&k1, &salt, &input, &mut plain).unwrap();
        let actual = sha512(&plain);
        let mut expect = [0u8; 64];
        cbc_decrypt(&k2, &salt, &hash_ct, &mut expect).unwrap();
        assert_eq!(actual, expect);
    }

    #[test]
    fn roundtrip_wrong_password_and_flipped_byte() {
        let password_salt = [0x22u8; 16];
        let key_salt = [0x33u8; 16];
        let mut plain = b"PK\x03\x04hello-agile".to_vec();
        plain.extend(std::iter::repeat(0x5a).take(5000));
        let mut file = Vec::new();
        encrypt_ooxml_agile("secret", &password_salt, &key_salt, &plain, &mut file).unwrap();
        let mut out = Vec::new();
        crate::decrypt_ooxml(&mut std::io::Cursor::new(file.clone()), "secret", &mut out).unwrap();
        assert_eq!(out, plain);

        let err = crate::decrypt_ooxml(&mut std::io::Cursor::new(file.clone()), "nope", &mut Bomb)
            .unwrap_err();
        assert!(matches!(err, DecryptError::WrongPassword));
        assert_eq!(err.to_string(), "wrong password");

        let in_stream = file.len() - 512;
        file[in_stream] ^= 0x01;
        let err =
            crate::decrypt_ooxml(&mut std::io::Cursor::new(file), "secret", &mut Bomb).unwrap_err();
        assert!(matches!(err, DecryptError::Corrupt));
    }

    struct Bomb;
    impl Write for Bomb {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            panic!("plaintext written");
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn hex16(s: &str) -> [u8; 16] {
        hex_n(s)
    }
    fn hex32(s: &str) -> [u8; 32] {
        hex_n(s)
    }
    fn hex64(s: &str) -> [u8; 64] {
        hex_n(s)
    }
    fn hex_n<const N: usize>(s: &str) -> [u8; N] {
        let mut out = [0u8; N];
        for i in 0..N {
            out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
        }
        out
    }
}
