//! kiwix_materialize — progressive ZIM materialization over the real pinned
//! TLS transport (physical, mac ↔ Galaxy A04).
//!
//! ```sh
//! kiwix_materialize source <id_dir> <zim_path> <bind_addr> <dest_pin_hex> [article_path]
//! kiwix_materialize dest   <id_dir> <out_zim>  <src_addr>  <src_pin_hex>  <bulk_digest> <total> <article_path>
//! ```
//!
//! Protocol: the engine's own wire messages (`carryon_net::Message`:
//! `TransferRequest`/`ChunkData`, digest-addressed, per-chunk sha256) over the
//! real mutual-pinned TLS session — the same transport the core's serve/import
//! loop uses. The CHUNK LOOP itself is adapter-level: the core only fetches
//! whole sealed cuts (no partial-object fetch API — recorded as a gap for the
//! engine; see PHASE3-EVIDENCE.md). Bytes are addressed by the bulk object's
//! content digest, never by path.
//!
//! Destination algorithm (the progressive story, made physical):
//!  1. fetch the 80-byte ZIM header; parse the layout offsets from it;
//!  2. fetch the "index" region [header, clusterPtrPos + clusterCount*8)
//!     — mime list, all dirents, url/title/cluster pointer tables;
//!  3. try `ZimArchive::open` + article read (honest: record what works —
//!     typically the archive OPENS here, the cluster read fails);
//!  4. parse the article's dirent from the fetched index; fetch ONLY its
//!     cluster range; retry the read;
//!  5. report the fraction of the ZIM materialized when the article became
//!     readable, plus the real wire counters.
//!
//! PHYSICAL cross-device evidence (PLAT-001): two machines, two NICs, pinned
//! TLS. Not an APK; no platform "supported" claim.

use carryon_adapter_api::Adapter as _;
use carryon_adapter_kiwix::zim_backend::ZimArchive;
use carryon_net::{DeviceIdentity, Message, Pin, Session, TrustStore};
use sha2::{Digest, Sha256};
use std::net::TcpListener;
use std::path::Path;

const MAX_CHUNK: u64 = 256 * 1024; // stay under MAX_CONTROL_FRAME (base64)

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn sha256(b: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b);
    hex(&h.finalize())
}

fn load_or_make_identity(id_dir: &str, name: &str) -> DeviceIdentity {
    let dir = Path::new(id_dir);
    std::fs::create_dir_all(dir).expect("mkdir id_dir");
    let cert_p = dir.join("cert.der");
    let key_p = dir.join("key.der");
    if let (Ok(c), Ok(k)) = (std::fs::read(&cert_p), std::fs::read(&key_p)) {
        return DeviceIdentity::from_der(name, c, k).expect("identity_from_der");
    }
    let id = DeviceIdentity::generate(name).expect("identity_generate");
    std::fs::write(&cert_p, &id.cert_der).expect("write cert");
    std::fs::write(&key_p, id.export_key_der()).expect("write key");
    id
}

fn trust_with(pin_hex: &str, name: &str) -> std::sync::Arc<TrustStore> {
    let pin = Pin::from_hex(pin_hex).expect("bad pin hex");
    let mut t = TrustStore::new();
    t.pair(name, pin, "kiwix-materialize");
    std::sync::Arc::new(t)
}

// ---------------------------------------------------------------- source --

fn do_source(
    id_dir: &str,
    zim_path: &str,
    bind_addr: &str,
    dest_pin: &str,
    article: Option<String>,
) {
    // Real adapter over the real ZIM: declares the bulk digest + the article.
    let main = {
        let a = ZimArchive::open(zim_path).expect("open zim");
        a.info().expect("info").main_path.expect("main path")
    };
    let article_path = article.unwrap_or(main);
    let mut adapter = carryon_adapter_kiwix::KiwixAdapter::from_zim(
        "kiwix-materialize-src",
        zim_path,
        &article_path,
        0,
    )
    .expect("from_zim");
    let bulk_digest = adapter.zim_bulk_digest().expect("bulk digest");
    let total = adapter.zim_bulk_size();
    let article_bytes = {
        let gen = adapter.current_generation("s").unwrap();
        let tok = adapter.begin_snapshot("s", gen).unwrap();
        let manifest = adapter.describe_snapshot(&tok).unwrap();
        let size = manifest
            .objects
            .iter()
            .find(|o| o.object_id == "kiwix.article.v1")
            .unwrap()
            .logical_size;
        adapter
            .read_object(&tok, "kiwix.article.v1", 0, size)
            .unwrap()
    };
    println!("source: bulk_sha256={bulk_digest}");
    println!("source: bulk_size={total}");
    println!("source: article_path={article_path}");
    println!("source: article_sha256={}", sha256(&article_bytes));

    let file = std::fs::File::open(zim_path).expect("open raw zim");
    let id = load_or_make_identity(id_dir, "source");
    let trust = trust_with(dest_pin, "dest");
    let listener = TcpListener::bind(bind_addr).expect("bind");
    println!("source: listening on {bind_addr}, waiting for destination...");
    let mut sess = Session::accept(&listener, &id, trust).expect("accept (pin)");
    // We are the acceptor: the connector sends Hello, we reply Welcome.
    sess.server_negotiate(vec![]).expect("server negotiate");

    let mut served = 0u64;
    loop {
        match sess.recv().expect("recv") {
            Message::TransferRequest {
                content_hash,
                offset,
                length,
            } => {
                if content_hash != bulk_digest {
                    sess.send(Message::Unavailable {
                        content_hash,
                        reason: "digest is not the declared bulk object".into(),
                    })
                    .expect("send unavailable");
                    continue;
                }
                let n = length.min(MAX_CHUNK);
                let mut buf = vec![0u8; n as usize];
                use std::os::unix::fs::FileExt as _;
                file.read_exact_at(&mut buf, offset).expect("pread");
                let chunk_digest = sha256(&buf);
                served += buf.len() as u64;
                sess.send(Message::ChunkData {
                    content_hash,
                    offset,
                    chunk_digest,
                    bytes: buf,
                })
                .expect("send chunk");
            }
            Message::ImportComplete { .. } => break,
            other => {
                eprintln!("source: unexpected {other:?}");
                break;
            }
        }
    }
    println!(
        "source: served {served} bytes of {total} ({:.3}%) — wire sent={} recv={}",
        served as f64 * 100.0 / total as f64,
        sess.bytes_sent(),
        sess.bytes_recv()
    );
    println!("source: MATERIALIZE_SERVE_OK");
}

// ------------------------------------------------------------------- dest --

/// Parsed ZIM header (layout from libzim src/fileheader.cpp read()).
#[derive(Debug)]
struct ZimHeader {
    article_count: u32,
    cluster_count: u32,
    path_ptr_pos: u64,
    title_idx_pos: u64,
    cluster_ptr_pos: u64,
    mime_list_pos: u64,
    checksum_pos: u64,
}

fn parse_header(b: &[u8]) -> ZimHeader {
    assert_eq!(&b[0..4], b"ZIM\x04", "zim magic");
    let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let u64_at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    ZimHeader {
        article_count: u32_at(24),
        cluster_count: u32_at(28),
        path_ptr_pos: u64_at(32),
        title_idx_pos: u64_at(40),
        cluster_ptr_pos: u64_at(48),
        mime_list_pos: u64_at(56),
        checksum_pos: u64_at(72),
    }
}

/// One parsed dirent (layout from libzim src/dirent.cpp DirentReader).
#[derive(Debug)]
struct Dirent {
    is_redirect: bool,
    redirect_index: u32,
    cluster: u32,
    _blob: u32,
    ns: u8,
    path: String,
}

/// Read + parse the dirent stored at `off` in the materialized file.
fn read_dirent_at(file: &std::fs::File, off: u64) -> Option<Dirent> {
    let mut db = [0u8; 512];
    use std::os::unix::fs::FileExt as _;
    let n = file.read_at(&mut db, off).expect("read dirent");
    parse_dirent(&db[..n])
}

fn parse_dirent(b: &[u8]) -> Option<Dirent> {
    let u16_at = |o: usize| u16::from_le_bytes(b[o..o + 2].try_into().unwrap());
    let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let mime = u16_at(0);
    let redirect = mime == 0xffff;
    let mut o = 2usize;
    let _extra_len = b[o];
    o += 1;
    let ns = b[o];
    o += 1;
    o += 4; // version
    let (redirect_index, cluster, blob) = if redirect {
        (u32_at(o), 0, 0)
    } else {
        let c = u32_at(o);
        let bl = u32_at(o + 4);
        (0, c, bl)
    };
    o += if redirect { 4 } else { 8 };
    let path_end = b[o..].iter().position(|&x| x == 0)?;
    let path = String::from_utf8_lossy(&b[o..o + path_end]).into_owned();
    Some(Dirent {
        is_redirect: redirect,
        redirect_index,
        cluster,
        _blob: blob,
        ns,
        path,
    })
}

struct Fetcher<'a> {
    sess: &'a mut Session,
    digest: String,
    total: u64,
    file: std::fs::File,
    fetched: u64,
}

impl Fetcher<'_> {
    fn range(&mut self, offset: u64, len: u64) -> Result<(), String> {
        // Fail closed: never fetch outside the declared bulk object.
        if offset.checked_add(len).is_none_or(|end| end > self.total) {
            return Err(format!(
                "range [{offset}, +{len}) outside the declared bulk object ({})",
                self.total
            ));
        }
        let mut done = 0u64;
        while done < len {
            let n = (len - done).min(MAX_CHUNK);
            self.sess
                .send(Message::TransferRequest {
                    content_hash: self.digest.clone(),
                    offset: offset + done,
                    length: n,
                })
                .map_err(|e| e.to_string())?;
            match self.sess.recv().map_err(|e| e.to_string())? {
                Message::ChunkData {
                    content_hash,
                    offset: off,
                    chunk_digest,
                    bytes,
                } => {
                    if content_hash != self.digest || off != offset + done {
                        return Err("chunk reply mismatch (digest/offset)".into());
                    }
                    if sha256(&bytes) != chunk_digest {
                        return Err("CHUNK DIGEST MISMATCH — corrupt transfer".into());
                    }
                    use std::os::unix::fs::FileExt as _;
                    self.file
                        .write_all_at(&bytes, off)
                        .map_err(|e| e.to_string())?;
                    self.fetched += bytes.len() as u64;
                    done += bytes.len() as u64;
                }
                Message::Unavailable { reason, .. } => {
                    return Err(format!("unavailable: {reason}"))
                }
                other => return Err(format!("unexpected {other:?}")),
            }
        }
        Ok(())
    }

    fn try_read_article(zim_path: &str, article: &str) -> Result<String, String> {
        // Progressive open: no eager Xapian preload, so sparse clusters the
        // open path doesn't truly need never block the read.
        let arch = ZimArchive::open_progressive(zim_path).map_err(|e| format!("open: {e}"))?;
        let item = arch.item(article).map_err(|e| format!("entry: {e}"))?;
        let size = item.size();
        let mut out = Vec::new();
        let mut off = 0u64;
        loop {
            let part = item
                .read_range(off, 65536)
                .map_err(|e| format!("read: {e}"))?;
            if part.is_empty() {
                break;
            }
            out.extend_from_slice(&part);
            off += part.len() as u64;
        }
        if out.len() as u64 != size {
            return Err("size mismatch".into());
        }
        Ok(sha256(&out))
    }
}

fn do_dest(
    id_dir: &str,
    out_zim: &str,
    src_addr: &str,
    src_pin: &str,
    bulk_digest: &str,
    total: u64,
    article_path: &str,
) {
    let id = load_or_make_identity(id_dir, "dest");
    let trust = trust_with(src_pin, "source");
    println!("dest: connecting to {src_addr}...");
    let mut sess = Session::connect(src_addr, &id, trust).expect("connect (pin)");
    sess.client_negotiate(vec![]).expect("client negotiate");

    // Empty sparse file of the full size: nothing materialized yet.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(out_zim)
        .expect("create out zim");
    file.set_len(total).expect("set_len (sparse)");
    let mut fetcher = Fetcher {
        sess: &mut sess,
        digest: bulk_digest.to_string(),
        total,
        file,
        fetched: 0,
    };

    // Stage 1: the 80-byte header.
    fetcher.range(0, 80).expect("fetch header");
    let mut hdr = [0u8; 80];
    use std::os::unix::fs::FileExt as _;
    fetcher
        .file
        .read_exact_at(&mut hdr, 0)
        .expect("read header");
    let h = parse_header(&hdr);
    println!(
        "dest: header ok: articles={} clusters={} mime_list@{} title_idx@{} index=[0..{}) clusters@{} checksum@{}",
        h.article_count,
        h.cluster_count,
        h.mime_list_pos,
        h.title_idx_pos,
        h.cluster_ptr_pos + h.cluster_count as u64 * 8,
        h.cluster_ptr_pos,
        h.checksum_pos
    );

    // Stage 2: the genuinely minimal "index" fetch. Real ZIM v6 layout
    // (verified from this file's own header):
    //   [0,80) header; [80,~2K) mime list; clusters at clusterPtr[0]…;
    //   dirents at ~86% of the file; then urlPtr, clusterPtr tables; checksum.
    // First-read needs: header + clusterPtr table + the dirent region
    //   [urlPtr[0], pathPtrPos) + urlPtr table — NOT the clusters.
    let full_mode = article_path == "--full";
    if full_mode {
        println!("dest: FULL mode — fetching the whole library");
        fetcher.range(80, total - 80).expect("fetch full");
    } else {
        // The tiny tables first (they tell us where the dirents start).
        fetcher
            .range(h.cluster_ptr_pos, h.cluster_count as u64 * 8)
            .expect("fetch clusterPtr table");
        fetcher.range(h.path_ptr_pos, 8).expect("fetch first urlPtr");
        let mut b8 = [0u8; 8];
        use std::os::unix::fs::FileExt as _;
        fetcher
            .file
            .read_exact_at(&mut b8, h.path_ptr_pos)
            .expect("read first urlPtr");
        let dirent_start = u64::from_le_bytes(b8);
        // The first cluster begins right after the mime list.
        fetcher
            .file
            .read_exact_at(&mut b8, h.cluster_ptr_pos)
            .expect("read first clusterPtr");
        let first_cluster_start = u64::from_le_bytes(b8);
        println!(
            "dest: mime list [80, {first_cluster_start}); clusters occupy the hole; \
             dirents at [{dirent_start}, {})",
            h.path_ptr_pos
        );
        // Header + mime list only — the clusters between them stay sparse.
        fetcher
            .range(80, first_cluster_start - 80)
            .expect("fetch header+mime");
        fetcher
            .range(dirent_start, h.path_ptr_pos - dirent_start)
            .expect("fetch dirent region");
        fetcher
            .range(h.path_ptr_pos, h.article_count as u64 * 8)
            .expect("fetch urlPtr table");
        if h.title_idx_pos != u64::MAX && h.title_idx_pos > h.cluster_ptr_pos {
            // Old-format title listing lives after the cluster table.
            fetcher
                .range(h.title_idx_pos, h.article_count as u64 * 4)
                .expect("fetch title table");
        }
    }
    println!(
        "dest: index fetched ({}/{} = {:.3}%)",
        fetcher.fetched,
        total,
        fetcher.fetched as f64 * 100.0 / total as f64
    );

    // Stage 3: honest attempt — the archive typically OPENS with the index
    // alone; the article read needs its cluster.
    let article_to_read = if full_mode {
        // Everything was fetched; read the archive's MAIN page entry.
        let read_u32_at = |off: u64| -> u32 {
            let mut b4 = [0u8; 4];
            use std::os::unix::fs::FileExt as _;
            fetcher.file.read_exact_at(&mut b4, off).expect("read u32");
            u32::from_le_bytes(b4)
        };
        let read_u64_at = |off: u64| -> u64 {
            let mut b8 = [0u8; 8];
            use std::os::unix::fs::FileExt as _;
            fetcher.file.read_exact_at(&mut b8, off).expect("read u64");
            u64::from_le_bytes(b8)
        };
        let main_idx = read_u32_at(64);
        let main_off = read_u64_at(h.path_ptr_pos + main_idx as u64 * 8);
        let d = read_dirent_at(&fetcher.file, main_off).expect("parse main dirent");
        println!("dest: FULL mode — reading the main entry '{}'", d.path);
        d.path
    } else {
        article_path.to_string()
    };
    match Fetcher::try_read_article(out_zim, &article_to_read) {
        Ok(sha) => println!(
            "dest: article '{article_to_read}' readable (sha256={sha}); materialized so far = \
             {}/{} ({:.3}%)",
            fetcher.fetched,
            total,
            fetcher.fetched as f64 * 100.0 / total as f64
        ),
        Err(e) => {
            if full_mode {
                panic!("FULL mode must yield a readable article: {e}");
            }
            println!("dest: after index, article not yet readable ({e}) — expected");
        }
    }
    let index_stage_fetched = fetcher.fetched;

    // Stage 4+5: FULL mode is done (everything materialized); otherwise find
    // the article's dirent in the fetched index, fetch ONLY its cluster, and
    // make the read that matters.
    if full_mode {
        println!(
            "dest: FULL transfer complete — materialized {}/{} (100%)",
            fetcher.fetched, total
        );
    } else {
        let read_u64_at = |file: &std::fs::File, off: u64| -> u64 {
            let mut b8 = [0u8; 8];
            use std::os::unix::fs::FileExt as _;
            file.read_exact_at(&mut b8, off).expect("read u64");
            u64::from_le_bytes(b8)
        };
        let find_dirent =
            |file: &std::fs::File, off: u64| -> Option<Dirent> { read_dirent_at(file, off) };
        let mut dirent_offsets = Vec::with_capacity(h.article_count as usize);
        for i in 0..h.article_count as u64 {
            dirent_offsets.push(read_u64_at(&fetcher.file, h.path_ptr_pos + i * 8));
        }
        let mut target: Option<Dirent> = None;
        let mut parsed = 0u64;
        let mut first_samples = Vec::new();
        for off in &dirent_offsets {
            if let Some(d) = find_dirent(&fetcher.file, *off) {
                parsed += 1;
                if first_samples.len() < 3 {
                    first_samples.push(format!("{}:'{}'", off, d.path));
                }
                if d.path == article_path {
                    target = Some(d);
                    break;
                }
            }
        }
        println!(
            "dest: [debug] walked {} urlPtr offsets, parsed {} dirents, samples: {}",
            dirent_offsets.len(),
            parsed,
            first_samples.join(", ")
        );
        // Follow the redirect chain if the article path is a redirect.
        if let Some(d) = target {
            let mut d = d;
            let mut hops = 0;
            while d.is_redirect && hops < 8 {
                let off = dirent_offsets
                    .get(d.redirect_index as usize)
                    .copied()
                    .expect("redirect index in range");
                d = find_dirent(&fetcher.file, off).expect("parse redirect target");
                hops += 1;
            }
            target = Some(d);
        }
        let Some(d) = target else {
            eprintln!("dest: FAIL article '{article_path}' not found in the fetched index");
            std::process::exit(1);
        };
        assert!(!d.is_redirect, "redirect chain must end at a real entry");
        println!(
            "dest: article '{article_path}' lives in cluster {}",
            d.cluster
        );

        let c_start = read_u64_at(&fetcher.file, h.cluster_ptr_pos + d.cluster as u64 * 8);
        let c_end = if d.cluster as u64 + 1 < h.cluster_count as u64 {
            read_u64_at(
                &fetcher.file,
                h.cluster_ptr_pos + (d.cluster as u64 + 1) * 8,
            )
        } else {
            h.checksum_pos
        };
        let c_len = c_end - c_start;
        println!(
            "dest: fetching cluster {} range [{c_start}, {c_end}) = {c_len} B",
            d.cluster
        );
        fetcher.range(c_start, c_len).expect("fetch cluster");

        // The OPEN path additionally reads the title-ordering index entry
        // 'X/listing/titleOrdered/v1' (fileimpl.cpp:264) — its cluster must
        // be materialized too. Xapian stays lazy (progressive open).
        let title_cluster = dirent_offsets.iter().find_map(|off| {
            find_dirent(&fetcher.file, *off).and_then(|x| {
                (x.ns == b'X' && x.path == "listing/titleOrdered/v1" && !x.is_redirect)
                    .then_some(x.cluster)
            })
        });
        if let Some(tc) = title_cluster {
            let t_start = read_u64_at(&fetcher.file, h.cluster_ptr_pos + tc as u64 * 8);
            let t_end = if tc as u64 + 1 < h.cluster_count as u64 {
                read_u64_at(&fetcher.file, h.cluster_ptr_pos + (tc as u64 + 1) * 8)
            } else {
                h.checksum_pos
            };
            println!(
                "dest: fetching title-index cluster {tc} range [{t_start}, {t_end}) = {} B",
                t_end - t_start
            );
            fetcher
                .range(t_start, t_end - t_start)
                .expect("fetch title cluster");
        }

        // Stage 5: the read that matters.
        let sha = Fetcher::try_read_article(out_zim, article_path)
            .expect("article readable from the partially-materialized ZIM");
        let fetched = fetcher.fetched;
        println!(
            "dest: ARTICLE READABLE — materialized {fetched}/{} bytes = {:.4}% of the ZIM \
             (index {index_stage_fetched} + cluster {})",
            total,
            fetched as f64 * 100.0 / total as f64,
            fetched - index_stage_fetched
        );
        println!("dest: article sha256={sha}");
    }

    fetcher
        .sess
        .send(Message::ImportComplete {
            session: "kiwix-materialize".into(),
            cut_number: 0,
            manifest_digest: bulk_digest.into(),
        })
        .expect("send done");
    println!(
        "dest: wire sent={} recv={}",
        sess.bytes_sent(),
        sess.bytes_recv()
    );
    println!("dest: MATERIALIZE_OK");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("pin") if args.len() >= 2 => {
            let id = load_or_make_identity(&args[1], "device");
            println!("{}", id.pin().to_hex());
        }
        Some("source") if args.len() >= 5 => {
            do_source(&args[1], &args[2], &args[3], &args[4], args.get(5).cloned())
        }
        Some("dest") if args.len() >= 8 => do_dest(
            &args[1],
            &args[2],
            &args[3],
            &args[4],
            &args[5],
            args[6].parse().expect("total as u64"),
            &args[7],
        ),
        _ => {
            eprintln!(
                "usage:\n  kiwix_materialize source <id_dir> <zim_path> <bind_addr> <dest_pin_hex> [article_path]\n  kiwix_materialize dest <id_dir> <out_zim> <src_addr> <src_pin_hex> <bulk_digest> <total> <article_path>"
            );
            std::process::exit(2);
        }
    }
}
