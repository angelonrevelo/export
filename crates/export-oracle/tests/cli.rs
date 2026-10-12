use std::fs;
use std::process::Command;

fn bin() -> String {
    // Cargo 1.83 does not inject CARGO_BIN_EXE into this workspace's
    // integration tests. The debug binary sits next to the test harness deps.
    let mut path = std::env::current_exe().unwrap();
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.push(if cfg!(windows) {
        "export-rust-oracle.exe"
    } else {
        "export-rust-oracle"
    });
    path.to_string_lossy().into_owned()
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("export-oracle-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn cli_copies_an_under_cap_phone_jpeg() {
    let dir = scratch("copy");
    let src = dir.join("src.bin");
    let dest = dir.join("rust-out");
    fs::write(&src, b"not-a-photo").unwrap();
    let status = Command::new(bin())
        .arg(&src)
        .arg(&dest)
        .arg("phone-jpeg")
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(fs::read(&dest).unwrap(), b"not-a-photo");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cli_over_cap_heic_writes_no_file() {
    let dir = scratch("heic");
    let src = dir.join("src.heic");
    let dest = dir.join("rust-out");
    fs::write(&src, vec![0u8; 1_000_001]).unwrap();
    let output = Command::new(bin())
        .arg(&src)
        .arg(&dest)
        .arg("heic")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!dest.exists());
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("no decoder"), "{err}");
    let _ = fs::remove_dir_all(&dir);
}
