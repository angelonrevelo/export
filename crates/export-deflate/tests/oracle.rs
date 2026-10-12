use std::io::Write;
use std::process::{Command, Stdio};

use export_deflate::{compress_with, inflate, Method};

fn script(cmd: &str, input: &[u8]) -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/zlib_oracle.py");
    let mut child = Command::new("python3")
        .arg(path)
        .arg(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("python3");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(input).expect("write");
    }
    let out = child.wait_with_output().expect("wait");
    assert!(
        out.status.success(),
        "{cmd} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

#[test]
fn python_zlib_byte_compare() {
    let mut samples: Vec<Vec<u8>> = vec![
        b"".to_vec(),
        b"a".to_vec(),
        b"hello".to_vec(),
        b"The quick brown fox jumps over the lazy dog. ".repeat(4),
        b"abc".repeat(200),
        (0..=255).collect(),
        vec![9u8; 4096],
    ];
    let mut state = 0xA5A5_5A5Au32;
    let mut mixed = Vec::with_capacity(12_000);
    for i in 0..12_000 {
        state = state.wrapping_mul(1103515245).wrapping_add(12345);
        if i % 17 == 0 {
            mixed.extend_from_slice(b"export");
        } else {
            mixed.push((state >> 16) as u8);
        }
    }
    samples.push(mixed);

    for sample in &samples {
        let theirs = script("compress", sample);
        assert_eq!(inflate(&theirs).unwrap(), *sample);
        for method in [Method::Auto, Method::Stored, Method::Fixed, Method::Dynamic] {
            let ours = compress_with(sample, method);
            assert_eq!(script("decompress", &ours), *sample);
        }
    }
}
