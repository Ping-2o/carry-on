//! kiwix_firstpaint — the companion-shell FIRST PAINT (ADAPTER-DESIGN §7,
//! option 2): serve the CARRIED `kiwix.article.v1` bytes — read straight from
//! the destination engine's content-addressed object store after a real
//! carry — over a loopback HTTP socket, so the device's own WebView renders
//! the carried bytes as a visible page. Until this existed, "first paint"
//! was a sha256 match, not a rendered surface.
//!
//! ```sh
//! kiwix_firstpaint <engine_data_dir> <article_sha256> [zim_path] [port]
//! ```
//!
//! Two honest resource tiers:
//!  * `/` is the CARRIED object — sha256-verified against the cut's declared
//!    digest before anything is served; fail closed on mismatch.
//!  * everything else (`_res_/style.css`, `_assets_/*.jpg`, …) is resolved
//!    from the LOCAL ZIM through the libzim backend, exactly like the Kiwix
//!    app's own WebView does — the library is already on the device; the
//!    carry brought the position, not the book. Without a `zim_path` the
//!    resources 404 honestly (the library-less case — that cost is the
//!    materializer's story, PHASE4-EVIDENCE §B).
//!
//! Browsers re-encode some characters in ZIM paths (observed: ``,` encoded as
//! `%252C`), so each request is resolved trying the raw target, then
//! percent-decoded once, then twice — first hit wins, the attempt is logged.
//!
//! The digest-addressed store layout is the engine's own (spec §19.1):
//! `<data_dir>/objects/sha256/ab/cd/<full-digest>` — digests, never paths.

use carryon_adapter_kiwix::zim_backend::ZimArchive;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// percent-decode once (%xx → byte). Invalid escapes left as-is.
fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() + 1 && i + 2 < b.len() {
            let hi = (b[i + 1] as char).to_digit(16);
            let lo = (b[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

struct Resolved {
    bytes: Vec<u8>,
    mimetype: String,
    via: String,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: kiwix_firstpaint <engine_data_dir> <article_sha256> [zim_path] [port]");
        std::process::exit(2);
    }
    let data_dir = &args[0];
    let digest = &args[1];
    let zim_path = args.get(2).filter(|p| !p.is_empty());
    let port: u16 = args.get(3).map(|p| p.parse().unwrap()).unwrap_or(8080);

    // Content-addressed store: objects/sha256/ab/cd/<full-digest> (§19.1).
    let path = std::path::Path::new(data_dir)
        .join("objects/sha256")
        .join(&digest[..2])
        .join(&digest[2..4])
        .join(digest);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("FAIL read carried object {}: {e}", path.display());
            std::process::exit(1);
        }
    };
    // Fail-closed: verify the stored bytes against the declared digest.
    let mut h = Sha256::new();
    h.update(&bytes);
    let actual = hex(&h.finalize());
    if actual != *digest {
        eprintln!("FAIL sha256 mismatch: store says {actual}, cut declared {digest}");
        std::process::exit(1);
    }
    println!(
        "firstpaint: carried article {digest} verified ({} bytes) — serving",
        bytes.len()
    );

    let zim = zim_path.and_then(|p| match ZimArchive::open(p) {
        Ok(z) => {
            println!("firstpaint: local ZIM open — resources resolve from the library");
            Some(z)
        }
        Err(e) => {
            eprintln!("firstpaint: local ZIM open failed ({e}) — resources will 404");
            None
        }
    });

    let resolve = |target: &str| -> Option<Resolved> {
        // strip query/fragment, then try raw / decoded-once / decoded-twice
        let t = target.split(['?', '#']).next().unwrap_or("");
        let zim = zim.as_ref()?;
        let mut candidates: Vec<(String, &str)> = vec![];
        let d1 = pct_decode(t);
        let d2 = pct_decode(&d1);
        for (cand, via) in [(t, "raw"), (&d1, "decoded"), (&d2, "decoded-twice")] {
            if candidates.iter().any(|(c, _)| c == cand) {
                continue;
            }
            candidates.push((cand.to_string(), via));
        }
        for (cand, via) in candidates {
            if cand.is_empty() || cand == "/" {
                continue;
            }
            let path = cand.trim_start_matches('/');
            if let Ok(item) = zim.item(path) {
                let size = item.size();
                let full = item.read_range(0, size).ok()?;
                let mt = item
                    .mimetype()
                    .unwrap_or_else(|_| "application/octet-stream".to_string());
                return Some(Resolved {
                    bytes: full,
                    mimetype: mt,
                    via: format!("{via}:{path}"),
                });
            }
        }
        None
    };

    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).expect("bind loopback");
    println!("FIRSTPAINT_SERVING http://127.0.0.1:{port}/");
    let mut served = 0u64;
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let mut req = [0u8; 4096];
        let n = stream.read(&mut req).unwrap_or(0);
        let line = String::from_utf8_lossy(&req[..n]);
        let target = line.split_whitespace().nth(1).unwrap_or("");
        if target == "/" || target.is_empty() {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&bytes);
            served += 1;
            println!(
                "firstpaint: GET / -> 200 carried object ({} bytes, request #{served})",
                bytes.len()
            );
        } else if let Some(r) = resolve(target) {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                r.mimetype,
                r.bytes.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&r.bytes);
            println!(
                "firstpaint: GET {target} -> 200 from local ZIM ({} B, {})",
                r.bytes.len(),
                r.via
            );
        } else {
            let head = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(head.as_bytes());
            if zim.is_some() {
                println!("firstpaint: GET {target} -> 404 (not in the local ZIM either)");
            } else {
                println!(
                    "firstpaint: GET {target} -> 404 (no local ZIM — library-less first paint)"
                );
            }
        }
        let _ = stream.flush();
    }
}
