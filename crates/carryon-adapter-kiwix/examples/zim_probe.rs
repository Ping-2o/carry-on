//! zim_probe — minimal CLI over the kiwix adapter's libzim backend.
//!
//! Doubles as the cross-link evidence (built for aarch64-linux-android and
//! pushed to the device) and as the on-device ZIM-open probe:
//!
//! ```sh
//! zim_probe <path-to.zim> [article_path]
//! ```
//!
//! Prints the archive identity, the ZIM header magic at offset 0, and the
//! sha256 + size of the requested article (default: the main page). Exits
//! non-zero on any failure — fail closed, never fabricated output.

use carryon_adapter_kiwix::zim_backend::ZimArchive;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: zim_probe <path-to.zim> [article_path]");
            std::process::exit(2);
        }
    };
    let article = args.next();

    let arch = match ZimArchive::open(&path) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("FAIL open: {e}");
            std::process::exit(1);
        }
    };
    let info = match arch.info() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("FAIL info: {e}");
            std::process::exit(1);
        }
    };
    println!("uuid={}", info.uuid);
    println!("title={}", info.title.unwrap_or_default());
    println!("filesize={}", info.filesize);
    println!("entry_count={}", info.entry_count);
    let main_path = info.main_path.unwrap_or_default();
    println!("main_path={main_path}");

    // Raw file-offset read through the adapter (same pread path the bulk
    // object uses): first 80 bytes = the ZIM header.
    let file = std::fs::File::open(&path).expect("open file");
    use std::os::unix::fs::FileExt as _;
    let mut head = [0u8; 80];
    file.read_exact_at(&mut head, 0).expect("pread header");
    assert_eq!(&head[0..4], b"ZIM\x04", "zim magic");
    println!("header_magic=ZIM\\x04");

    let target = article.unwrap_or(main_path);
    let item = match arch.item(&target) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("FAIL item '{target}': {e}");
            std::process::exit(1);
        }
    };
    let size = item.size();
    let mut bytes = Vec::new();
    let mut off = 0u64;
    loop {
        let part = match item.read_range(off, 65536) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("FAIL read: {e}");
                std::process::exit(1);
            }
        };
        if part.is_empty() {
            break;
        }
        bytes.extend_from_slice(&part);
        off += part.len() as u64;
    }
    assert_eq!(bytes.len() as u64, size);
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(&bytes);
    let mut hex = String::new();
    for b in h.finalize() {
        hex.push_str(&format!("{b:02x}"));
    }
    println!("article_path={}", item.path().unwrap_or_default());
    println!("article_title={}", item.title().unwrap_or_default());
    println!("article_size={size}");
    println!("article_sha256={hex}");
    println!("PROBE_OK");
}
