use export_pack::{Error, Method, ZipReader, ZipWriter, CHUNK_BYTES};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SCRATCH: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let n = SCRATCH.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("export-pack-{}-{n}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_archive(path: &Path, entries: &[(&str, &[u8])]) {
    let mut zip = ZipWriter::create(path).unwrap();
    for (name, bytes) in entries {
        zip.write_bytes(name, bytes).unwrap();
    }
    zip.finish().unwrap();
    assert!(!sibling_tmp(path).exists());
    assert!(path.exists());
}

fn sibling_tmp(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(".tmp");
    PathBuf::from(os)
}

fn read_entry(path: &Path, name: &str) -> Vec<u8> {
    let mut zip = ZipReader::open(path).unwrap();
    let index = zip.index_of(name).unwrap();
    let mut out = Vec::new();
    zip.copy_entry(index, &mut out).unwrap();
    out
}

#[test]
fn roundtrip_reloads_the_bytes() {
    let dir = Scratch::new();
    let path = dir.path("files.zip");
    let sizes = [
        0usize,
        1,
        2,
        64,
        CHUNK_BYTES - 1,
        CHUNK_BYTES,
        CHUNK_BYTES + 1,
        CHUNK_BYTES * 2 + 3,
    ];
    let mut zip = ZipWriter::create(&path).unwrap();
    let mut payloads = Vec::new();
    for (i, size) in sizes.iter().copied().enumerate() {
        let bytes: Vec<u8> = (0..size).map(|b| (b + i * 13) as u8).collect();
        let name = format!("f{i}.bin");
        zip.write_bytes(&name, &bytes).unwrap();
        payloads.push((name, bytes));
    }
    zip.write_bytes("dir/note.txt", b"hello").unwrap();
    zip.write_bytes("café.txt", "café".as_bytes()).unwrap();
    zip.finish().unwrap();

    let mut reader = ZipReader::open(&path).unwrap();
    assert_eq!(reader.len(), payloads.len() + 2);
    for (name, bytes) in &payloads {
        let index = reader.index_of(name).unwrap();
        let method = reader.entries()[index].method();
        let size = reader.entries()[index].size();
        let crc = reader.entries()[index].crc32();
        assert_eq!(method, Method::Store);
        assert_eq!(size, bytes.len() as u64);
        let mut out = Vec::new();
        reader.copy_entry(index, &mut out).unwrap();
        assert_eq!(&out, bytes);
        assert_eq!(crc, export_pack::Crc32::checksum_of(bytes));
    }
    assert_eq!(read_entry(&path, "dir/note.txt"), b"hello");
    assert_eq!(read_entry(&path, "café.txt"), "café".as_bytes());
    let again = reader.index_of("f0.bin").unwrap();
    let mut out = Vec::new();
    reader.copy_entry(again, &mut out).unwrap();
    assert_eq!(out, payloads[0].1);
}

#[test]
fn empty_archive() {
    let dir = Scratch::new();
    let path = dir.path("empty.zip");
    ZipWriter::create(&path).unwrap().finish().unwrap();
    let zip = ZipReader::open(&path).unwrap();
    assert!(zip.is_empty());
}

#[test]
fn drop_removes_tmp_and_does_not_publish() {
    let dir = Scratch::new();
    let path = dir.path("partial.zip");
    let zip = ZipWriter::create(&path).unwrap();
    assert!(sibling_tmp(&path).exists());
    drop(zip);
    assert!(!sibling_tmp(&path).exists());
    assert!(!path.exists());
}

#[test]
fn failed_reader_is_not_published() {
    let dir = Scratch::new();
    let path = dir.path("boom.zip");
    let mut zip = ZipWriter::create(&path).unwrap();
    zip.write_bytes("ok.txt", b"ok").unwrap();
    let err = zip.write_entry("bad.txt", Boom).unwrap_err();
    assert!(matches!(err, Error::Io(_)));
    assert!(zip.finish().is_err());
    assert!(!path.exists());
    assert!(!sibling_tmp(&path).exists());
}

struct Boom;

impl Read for Boom {
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("boom"))
    }
}

#[test]
fn checksum_mismatch_on_flipped_payload() {
    let dir = Scratch::new();
    let path = dir.path("flip.zip");
    let payload = b"hello-payload-unique";
    write_archive(&path, &[("a", payload)]);
    let mut bytes = fs::read(&path).unwrap();
    let at = bytes
        .windows(payload.len())
        .position(|window| window == payload)
        .unwrap();
    bytes[at] ^= 0xFF;
    fs::write(&path, &bytes).unwrap();
    let mut zip = ZipReader::open(&path).unwrap();
    let err = zip.copy_entry(0, &mut Vec::new()).unwrap_err();
    assert!(matches!(err, Error::ChecksumMismatch));
    assert_eq!(err.to_string(), "checksum mismatch");
    let dest = dir.path("out.bin");
    let err = zip.extract_entry(0, &dest).unwrap_err();
    assert!(matches!(err, Error::ChecksumMismatch));
    assert!(!dest.exists());
    assert!(!sibling_tmp(&dest).exists());
}

#[test]
fn truncated_archive_errors() {
    let dir = Scratch::new();
    let path = dir.path("cut.zip");
    write_archive(&path, &[("a", b"abcdefghijklmnopqrstuvwxyz")]);
    let len = fs::metadata(&path).unwrap().len();
    let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(len / 2).unwrap();
    drop(file);
    let err = ZipReader::open(&path).unwrap_err();
    assert!(matches!(err, Error::Format(_)), "{err}");
}

#[test]
fn refuses_unsafe_names_on_write() {
    let dir = Scratch::new();
    let path = dir.path("names.zip");
    let mut zip = ZipWriter::create(&path).unwrap();
    for bad in ["../x", "/etc/passwd", "a\\b", "foo/../../x"] {
        assert!(matches!(
            zip.write_bytes(bad, b"x").unwrap_err(),
            Error::UnsafeName
        ));
    }
    zip.write_bytes("ok.txt", b"x").unwrap();
    zip.finish().unwrap();
}

#[test]
fn extract_renames_after_crc() {
    let dir = Scratch::new();
    let path = dir.path("src.zip");
    let payload = vec![7u8; CHUNK_BYTES + 9];
    write_archive(&path, &[("blob", &payload)]);
    let dest = dir.path("blob.bin");
    let mut zip = ZipReader::open(&path).unwrap();
    zip.extract_entry(0, &dest).unwrap();
    assert_eq!(fs::read(&dest).unwrap(), payload);
    assert!(!sibling_tmp(&dest).exists());
}

#[test]
fn hostile_name_is_visible_but_not_safe() {
    let dir = Scratch::new();
    let path = dir.path("hostile.zip");
    write_store_with_python(&path, "../x", b"hi");
    let zip = ZipReader::open(&path).unwrap();
    let entry = &zip.entries()[0];
    assert_eq!(entry.name(), "../x");
    assert!(matches!(entry.safe_name(), Err(Error::UnsafeName)));
    assert_eq!(entry.method(), Method::Store);
}

#[test]
fn python_zipfile_roundtrip() {
    let dir = Scratch::new();
    let path = dir.path("ours.zip");
    let a = b"alpha";
    let b = vec![1u8, 2, 3, 4, 255];
    write_archive(
        &path,
        &[("a.txt", a.as_slice()), ("sub/b.bin", b.as_slice())],
    );
    python_read(&path, "a.txt", a);
    python_read(&path, "sub/b.bin", &b);

    let theirs = dir.path("theirs.zip");
    write_store_with_python(&theirs, "from-python", b"store-bytes");
    assert_eq!(read_entry(&theirs, "from-python"), b"store-bytes");
    let zip = ZipReader::open(&theirs).unwrap();
    assert_eq!(zip.entries()[0].method(), Method::Store);
    assert_eq!(
        zip.entries()[0].crc32(),
        export_pack::Crc32::checksum_of(b"store-bytes")
    );
}

fn python_read(path: &Path, name: &str, expect: &[u8]) {
    let script = r#"
import sys, zipfile
z = zipfile.ZipFile(sys.argv[1])
data = z.read(sys.argv[2])
info = z.getinfo(sys.argv[2])
assert info.compress_type == zipfile.ZIP_STORED
assert info.CRC == int(sys.argv[3])
sys.stdout.buffer.write(data)
"#;
    let crc = export_pack::Crc32::checksum_of(expect);
    let out = python(
        script,
        &[path, Path::new(name), Path::new(&crc.to_string())],
    );
    assert_eq!(out, expect);
}

fn write_store_with_python(path: &Path, name: &str, bytes: &[u8]) {
    let raw = path.with_extension("raw");
    fs::write(&raw, bytes).unwrap();
    let script = r#"
import sys, zipfile
name, src, dest = sys.argv[1:]
with open(src, "rb") as f:
    payload = f.read()
with zipfile.ZipFile(dest, "w", compression=zipfile.ZIP_STORED) as z:
    z.writestr(name, payload)
"#;
    python(script, &[Path::new(name), &raw, path]);
}

fn python(script: &str, args: &[&Path]) -> Vec<u8> {
    let mut cmd = std::process::Command::new("python3");
    cmd.arg("-c").arg(script);
    for arg in args {
        cmd.arg(arg);
    }
    let out = cmd.output().expect("python3");
    if !out.status.success() {
        panic!(
            "python failed: {}\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    out.stdout
}

#[test]
fn not_a_zip() {
    let dir = Scratch::new();
    let path = dir.path("nope");
    fs::write(&path, b"this is not a zip").unwrap();
    assert!(ZipReader::open(&path).is_err());
}

#[test]
fn deflate_huffman_from_python_errors_without_emitting() {
    let dir = Scratch::new();
    let path = dir.path("deflated.zip");
    let payload = b"hello";
    let script = r#"
import sys, zipfile
with zipfile.ZipFile(sys.argv[1], "w", compression=zipfile.ZIP_DEFLATED) as z:
    z.writestr("a.txt", open(sys.argv[2], "rb").read())
"#;
    let raw = dir.path("raw.bin");
    fs::write(&raw, payload).unwrap();
    python(script, &[&path, &raw]);
    let mut zip = ZipReader::open(&path).unwrap();
    assert_eq!(zip.entries()[0].method(), Method::Deflate);
    let mut out = Vec::new();
    let err = zip.copy_entry(0, &mut out).unwrap_err();
    assert_eq!(err.to_string(), "fixed huffman");
    assert!(out.is_empty());
}

// Keep File in scope so a reader can be built from an already-open handle.
#[test]
fn from_reader_matches_open() {
    let dir = Scratch::new();
    let path = dir.path("r.zip");
    write_archive(&path, &[("a", b"xyz")]);
    let file = File::open(&path).unwrap();
    let mut zip = ZipReader::from_reader(file).unwrap();
    let mut out = Vec::new();
    zip.copy_entry(0, &mut out).unwrap();
    assert_eq!(out, b"xyz");
}
