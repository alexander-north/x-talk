use std::path::Path;
fn hash_files(path: &Path, hash: &mut u64) {
    if path.is_dir() {
        let mut children: Vec<_> = std::fs::read_dir(path)
            .unwrap()
            .map(|p| p.unwrap().path())
            .collect();
        children.sort();
        for child in children {
            hash_files(&child, hash);
        }
    } else if path.is_file() {
        // A source fingerprint, not a security checksum. Explicit paths exclude secrets and data.
        for byte in std::fs::read(path).unwrap() {
            *hash = (*hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
    }
}
fn main() {
    let mut hash = 0xcbf29ce484222325;
    for path in [
        "src",
        "migrations",
        "permissions",
        "capabilities",
        "Cargo.toml",
        "Cargo.lock",
        "tauri.conf.json",
        "build.rs",
        "../src",
        "../package-lock.json",
        "../prompts",
        "../scripts/media.lock.json",
    ] {
        println!("cargo:rerun-if-changed={path}");
        hash_files(Path::new(path), &mut hash);
    }
    println!("cargo:rustc-env=XTALK_BUILD_ID={hash:016x}");
    #[cfg(feature = "desktop")]
    tauri_build::build()
}
