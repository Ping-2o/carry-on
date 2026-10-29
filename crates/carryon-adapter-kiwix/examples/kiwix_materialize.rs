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
//! Destination algorithm (the progressive story, made physical) — three
//! honest, comparable fetch plans (see docs/kiwix-bridge/PHASE4-EVIDENCE.md):
//!  * sparse (default): header + mime + clusterPtr table; then the EXACT
//!    dirent bytes libzim's lazy open reads (its binary searches, ported 1:1
//!    from src/dirent_lookup.h at the pinned ad888b4 — the probe machinery
//!    fetches each touched dirent as an 8 B urlPtr slot + a 256 B window);
//!    the title-listing cluster HEADER only (the blob is never read at
//!    open); and the article blob alone when its cluster is uncompressed;
//!  * --dense-blob: dense index + the listing BLOB (~85 KB) instead of its
//!    whole mis-ranged 3.23 MB "cluster";
//!  * --dense: the EXACT Phase-3 fetch plan (regression reproduction).
//!
//! On any sparse-path failure the destination falls back to a dense rescue
//! and retries — fail-closed, never a silent false success.
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
    minor_version: u16,
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
    let u16_at = |o: usize| u16::from_le_bytes(b[o..o + 2].try_into().unwrap());
    let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let u64_at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    ZimHeader {
        minor_version: u16_at(6),
        article_count: u32_at(24),
        cluster_count: u32_at(28),
        path_ptr_pos: u64_at(32),
        title_idx_pos: u64_at(40),
        cluster_ptr_pos: u64_at(48),
        mime_list_pos: u64_at(56),
        checksum_pos: u64_at(72),
    }
}

/// One parsed dirent (layout from libzim src/dirent.cpp DirentReader::initDirent).
#[derive(Debug, Clone)]
struct Dirent {
    is_redirect: bool,
    redirect_index: u32,
    cluster: u32,
    blob: u32,
    ns: u8,
    path: String,
    /// Raw bytes of the URL: libzim's lookups compare raw bytes, so does the
    /// probe emulation below (String::from_utf8_lossy would not be exact).
    path_raw: Vec<u8>,
}

/// Read + parse the dirent stored at `off` in the materialized file.
fn read_dirent_at(file: &std::fs::File, off: u64) -> Option<Dirent> {
    let mut db = [0u8; 512];
    use std::os::unix::fs::FileExt as _;
    let n = file.read_at(&mut db, off).expect("read dirent");
    parse_dirent(&db[..n])
}

/// Strict dirent parser mirroring `DirentReader::initDirent`: returns None
/// when the window is too small to hold path + title + extra parameters (the
/// caller then grows the window in 256 B steps, mirroring the C++ retry loop
/// in `DirentReader::readDirent`).
fn parse_dirent(b: &[u8]) -> Option<Dirent> {
    let u16_at = |o: usize| u16::from_le_bytes(b[o..o + 2].try_into().unwrap());
    let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let mime = u16_at(0);
    let redirect = mime == 0xffff;
    let extra_len = b[2] as usize;
    let ns = b[3];
    let mut o = 8usize; // mime(2) + extraLen(1) + ns(1) + version(4)
    let need = o + if redirect { 4 } else { 8 };
    if b.len() < need {
        return None;
    }
    let (redirect_index, cluster, blob) = if redirect {
        (u32_at(o), 0, 0)
    } else {
        let c = u32_at(o);
        let bl = u32_at(o + 4);
        (0, c, bl)
    };
    o += if redirect { 4 } else { 8 };
    let path_end = o + b[o..].iter().position(|&x| x == 0)?;
    let title_start = path_end + 1;
    let rest = b.get(title_start..)?;
    let title_end = title_start + rest.iter().position(|&x| x == 0)?;
    if title_end + 1 + extra_len > b.len() {
        return None; // window too small — caller grows it (+256 B)
    }
    let path_raw = b[o..path_end].to_vec();
    Some(Dirent {
        is_redirect: redirect,
        redirect_index,
        cluster,
        blob,
        ns,
        path: String::from_utf8_lossy(&path_raw).into_owned(),
        path_raw,
    })
}

struct Fetcher<'a> {
    sess: &'a mut Session,
    digest: String,
    total: u64,
    file: std::fs::File,
    fetched: u64,
    /// Honest unique coverage of the materialized file (re-fetched overlap
    /// counts once here, unlike `fetched`).
    unique: MergedRanges,
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
                    self.unique.add(off, off + bytes.len() as u64);
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

/// Merged byte ranges — honest unique-byte accounting. The WIRE counter
/// (Fetcher::fetched) counts re-fetched overlap twice (Phase 3's 23.787%
/// "title cluster" range overlapped the dirent region fetched earlier);
/// this tracks the actual coverage of the materialized file.
#[derive(Default)]
struct MergedRanges {
    // start -> end, non-overlapping, sorted by start.
    ranges: std::collections::BTreeMap<u64, u64>,
}

impl MergedRanges {
    fn add(&mut self, start: u64, end: u64) {
        let (mut s, mut e) = (start, end);
        // absorb overlapping/adjacent existing ranges
        let to_merge: Vec<(u64, u64)> = self
            .ranges
            .range(..=e)
            .filter(|(_s0, e0)| **e0 >= s)
            .map(|(s0, e0)| (*s0, *e0))
            .collect();
        for (s0, e0) in to_merge {
            self.ranges.remove(&s0);
            s = s.min(s0);
            e = e.max(e0);
        }
        self.ranges.insert(s, e);
    }
    fn unique_bytes(&self) -> u64 {
        self.ranges
            .values()
            .zip(self.ranges.keys())
            .map(|(e, s)| e - s)
            .sum()
    }
}

// ---------------------------------------------------------------- probes --
// Sparse-index machinery: fetch, on demand, exactly the bytes libzim's LAZY
// open/lookup path reads (pinned libzim ad888b4):
//   - DirectDirentAccessor::getDirent(i) = urlPtr slot (8 B) + one dirent
//     window, 256 B grown in 256 B steps (DirentReader::readDirent's retry);
//   - DirentLookup::find / getNamespaceBeginOffset = the deterministic
//     binary searches in src/dirent_lookup.h (ported 1:1 below). The SAME
//     dirent set libzim will read is fetched here — nothing more is guessed
//     and nothing less will satisfy the real open. If libzim ever probes a
//     dirent we did not fetch, the open/read fails VISIBLE (zeros → parse
//     error / digest mismatch) and the dense fallback rescues; never a
//     silent false success.

#[derive(Default)]
struct ProbeState {
    dirents: std::collections::HashMap<u64, Dirent>,
    /// dirent index -> its file offset (urlPtr slot value)
    offsets: std::collections::HashMap<u64, u64>,
    /// wire bytes spent on probes (urlPtr slots + dirent windows)
    wire: u64,
    requests: u64,
}

fn probe_dirent(f: &mut Fetcher, h: &ZimHeader, st: &mut ProbeState, idx: u64) -> Dirent {
    if let Some(d) = st.dirents.get(&idx) {
        return d.clone();
    }
    // getOffset(idx): the urlPtr slot.
    let slot = h.path_ptr_pos + idx * 8;
    f.range(slot, 8).expect("fetch urlPtr slot");
    st.wire += 8;
    st.requests += 1;
    let mut off8 = [0u8; 8];
    use std::os::unix::fs::FileExt as _;
    f.file
        .read_exact_at(&mut off8, slot)
        .expect("read urlPtr slot");
    let dirent_off = u64::from_le_bytes(off8);
    assert!(
        dirent_off < f.total,
        "dirent pointer outside the bulk object"
    );
    st.offsets.insert(idx, dirent_off);
    // readDirent(offset): 256 B window, grown by 256 until it parses.
    let mut window = 256u64;
    loop {
        let len = window.min(f.total - dirent_off).max(1);
        f.range(dirent_off, len).expect("fetch dirent window");
        st.wire += len;
        st.requests += 1;
        let mut buf = vec![0u8; len as usize];
        f.file
            .read_exact_at(&mut buf, dirent_off)
            .expect("read dirent window");
        if let Some(d) = parse_dirent(&buf) {
            st.dirents.insert(idx, d.clone());
            return d;
        }
        window += 256;
        assert!(
            window <= 4096,
            "dirent window overflow — refusing (fail closed)"
        );
    }
}

/// DirentLookup::compareWithDirentAt: ns first, then raw path bytes.
fn cmp_at(
    f: &mut Fetcher,
    h: &ZimHeader,
    st: &mut ProbeState,
    ns: u8,
    key: &[u8],
    idx: u64,
) -> std::cmp::Ordering {
    let d = probe_dirent(f, h, st, idx);
    match ns.cmp(&d.ns) {
        std::cmp::Ordering::Equal => key.cmp(&d.path_raw),
        other => other,
    }
}

/// Faithful port of DirentLookup::findInRange + binarySearchInRange.
#[allow(clippy::comparison_chain)]
fn find_in_range(
    f: &mut Fetcher,
    h: &ZimHeader,
    st: &mut ProbeState,
    l: u64,
    u: u64,
    ns: u8,
    key: &[u8],
) -> (bool, u64) {
    use std::cmp::Ordering::*;
    if l == u {
        return (false, l);
    }
    match cmp_at(f, h, st, ns, key, l) {
        Less => return (false, l),
        Equal => return (true, l),
        Greater => {}
    }
    if cmp_at(f, h, st, ns, key, u - 1) == Greater {
        return (false, u);
    }
    // binarySearchInRange(l, u-1)
    let mut l = l;
    let mut u = u - 1;
    loop {
        let p = l + (u - l).div_ceil(2); // upward-rounded average, as in C++
        let c = cmp_at(f, h, st, ns, key, p);
        if c != Greater {
            if u == p {
                return (c == Equal, u);
            }
            u = p;
        } else {
            l = p;
        }
    }
}

/// Faithful port of getNamespaceBeginOffset (dirent_lookup.h).
fn namespace_begin(f: &mut Fetcher, h: &ZimHeader, st: &mut ProbeState, ch: u8) -> u64 {
    if h.article_count == 0 {
        return 0;
    }
    let d0 = probe_dirent(f, h, st, 0);
    let mut lower = 0u64;
    let mut upper = h.article_count as u64;
    while upper - lower > 1 {
        let m = lower + (upper - lower) / 2;
        if probe_dirent(f, h, st, m).ns >= ch {
            upper = m;
        } else {
            lower = m;
        }
    }
    if d0.ns < ch {
        upper
    } else {
        lower
    }
}

// --------------------------------------------------------------- clusters --

/// One parsed cluster header: info byte + the whole blob-offset table
/// (Cluster::read + read_header at the pinned ad888b4). Valid only for
/// UNCOMPRESSED clusters — a compressed cluster's offset table lives inside
/// the compressed stream, so such clusters are fetched whole.
struct ClusterHead {
    start: u64,
    offsets: Vec<u64>,
}

fn fetch_cluster_head(f: &mut Fetcher, start: u64) -> ClusterHead {
    // info byte first (it tells us the offset width)
    f.range(start, 1).expect("fetch cluster info byte");
    let mut info_b = [0u8; 1];
    use std::os::unix::fs::FileExt as _;
    f.file
        .read_exact_at(&mut info_b, start)
        .expect("read cluster info byte");
    let info = info_b[0];
    let extended = info & 0x10 != 0;
    let w = if extended { 8u64 } else { 4u64 };
    // offsets[0] — the size of the whole table, in bytes
    f.range(start + 1, w).expect("fetch offsets[0]");
    let mut b0 = [0u8; 8];
    f.file
        .read_exact_at(&mut b0[..w as usize], start + 1)
        .expect("read offsets[0]");
    let off0 = if extended {
        u64::from_le_bytes(b0)
    } else {
        u32::from_le_bytes(b0[..4].try_into().unwrap()) as u64
    };
    assert_eq!(off0 % w, 0, "offset table not aligned");
    // the rest of the offsets table (read_header reads ALL of it)
    f.range(start + 1 + w, off0 - w)
        .expect("fetch offsets table");
    let mut raw = vec![0u8; (off0 - w) as usize];
    f.file
        .read_exact_at(&mut raw, start + 1 + w)
        .expect("read offsets table");
    let mut offsets = vec![off0];
    for chunk in raw.chunks_exact(w as usize) {
        let v = if extended {
            u64::from_le_bytes(chunk.try_into().unwrap())
        } else {
            u32::from_le_bytes(chunk.try_into().unwrap()) as u64
        };
        offsets.push(v);
    }
    // sanity: offsets[0] = the size of the whole table = entries * width
    assert_eq!(offsets.len() as u64 * w, off0, "offsets table shape");
    ClusterHead { start, offsets }
}

impl ClusterHead {
    /// Byte range of blob `b` within the ZIM file (blob data starts after the
    /// info byte; blob offsets are relative to it — cluster.h).
    fn blob_range(&self, b: u32) -> (u64, u64) {
        let b = b as usize;
        assert!(b + 1 < self.offsets.len(), "blob index out of range");
        (
            self.start + 1 + self.offsets[b],
            self.start + 1 + self.offsets[b + 1],
        )
    }
}

/// Fetch plans, honest and comparable:
///
/// - `Dense`     — the EXACT Phase-3 fetch plan (regression reproduction):
///   dense index (full urlPtr table + whole dirent region) and the whole
///   "title cluster" RANGE, taken up to checksum_pos for the last cluster —
///   which overlaps the dirent region already fetched (re-fetched, counted
///   twice on the wire).
/// - `DenseBlob` — the Phase-4 refinement: dense index, but the title listing
///   fetched as its BLOB (~85 KB here), not the 3.23 MB range.
/// - `Sparse`    — full refinement: libzim's lazy open (preloadDirentRanges(0)
///   — shim/czim.cpp) reads only the dirents its binary searches actually
///   touch; the probe machinery fetches exactly those (8 B slot + 256 B
///   window each), the listing cluster HEADER only (9 B here — the blob is
///   never read at open), and the article blob alone when its cluster is
///   uncompressed.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Dense,
    DenseBlob,
    Sparse,
}

/// Destination arguments for do_dest (clippy: keep the arg list ≤ 7).
struct DestArgs {
    id_dir: String,
    out_zim: String,
    src_addr: String,
    src_pin: String,
    bulk_digest: String,
    total: u64,
    article_path: String,
    full_article: Option<String>,
    mode: Mode,
}

fn read_u32_at(file: &std::fs::File, off: u64) -> u32 {
    let mut b4 = [0u8; 4];
    use std::os::unix::fs::FileExt as _;
    file.read_exact_at(&mut b4, off).expect("read u32");
    u32::from_le_bytes(b4)
}

fn read_u64_at(file: &std::fs::File, off: u64) -> u64 {
    let mut b8 = [0u8; 8];
    use std::os::unix::fs::FileExt as _;
    file.read_exact_at(&mut b8, off).expect("read u64");
    u64::from_le_bytes(b8)
}

/// Cluster range [start, end): the next clusterPtr entry, or — for the LAST
/// cluster — the start of the dirent region (urlPtr[0]). Phase 3 wrongly used
/// checksum_pos for the last cluster, whose "range" then swallowed the dirent
/// region; the real last cluster ends where the dirents begin.
fn cluster_range(f: &Fetcher, h: &ZimHeader, c: u32, dirent_start: u64) -> (u64, u64) {
    let s = read_u64_at(&f.file, h.cluster_ptr_pos + c as u64 * 8);
    let e = if c as u64 + 1 < h.cluster_count as u64 {
        read_u64_at(&f.file, h.cluster_ptr_pos + (c as u64 + 1) * 8)
    } else {
        dirent_start
    };
    (s, e)
}

/// Dense rescue: the fail-closed fallback for Sparse/DenseBlob. Fetches the
/// whole dense index (and the V0 title table for old-format ZIMs, and the
/// listing blob if the sparse plan skipped it) so the retry either works or
/// fails loudly — never a silent false success.
fn dense_rescue(f: &mut Fetcher, h: &ZimHeader, listing: Option<&Dirent>) {
    println!("dest: dense rescue — fetching the whole dense index");
    let dirent_start = if h.article_count > 0 {
        read_u64_at(&f.file, h.path_ptr_pos)
    } else {
        h.cluster_ptr_pos
    };
    f.range(dirent_start, h.path_ptr_pos - dirent_start)
        .expect("rescue: fetch dirent region");
    f.range(h.path_ptr_pos, h.article_count as u64 * 8)
        .expect("rescue: fetch urlPtr table");
    if h.title_idx_pos != u64::MAX && h.title_idx_pos > h.cluster_ptr_pos {
        f.range(h.title_idx_pos, h.article_count as u64 * 4)
            .expect("rescue: fetch V0 title table");
    }
    if let Some(listing) = listing {
        // guarantee the listing blob too (its header may already be there)
        let (t_start, t_end) = cluster_range(f, h, listing.cluster, dirent_start);
        f.range(t_start, t_end - t_start)
            .expect("rescue: fetch listing cluster");
    }
}

/// Fetch the title listing per plan: whole buggy range (Phase-3 exact), the
/// listing blob, or just the cluster header (Sparse — open never reads the
/// blob; `getTitleAccessorV1` builds the index accessor over the blob RANGE
/// but reads only the cluster's info byte + offsets table).
fn fetch_listing(f: &mut Fetcher, h: &ZimHeader, listing: &Dirent, dirent_start: u64, mode: Mode) {
    let (t_start, t_end) = cluster_range(f, h, listing.cluster, dirent_start);
    match mode {
        Mode::Dense => {
            // Phase-3 exact: the whole range up to checksum_pos — including
            // the dirent-region overlap (kept verbatim for the honest
            // regression number; see PHASE4 evidence).
            let t_len = h.checksum_pos - t_start;
            println!(
                "dest: fetching title 'cluster' {} range [{t_start}, {}) = {t_len} B (Phase-3 exact)",
                listing.cluster, h.checksum_pos
            );
            f.range(t_start, t_len).expect("fetch title cluster");
        }
        Mode::DenseBlob | Mode::Sparse => {
            let info = {
                f.range(t_start, 1).expect("fetch listing info byte");
                let mut b1 = [0u8; 1];
                use std::os::unix::fs::FileExt as _;
                f.file
                    .read_exact_at(&mut b1, t_start)
                    .expect("read info byte");
                b1[0]
            };
            let comp = info & 0x0F;
            if comp != 0 && comp != 1 {
                println!(
                    "dest: listing cluster {} is COMPRESSED (info 0x{info:02x}) — fetching it whole (blob-range fetch impossible)",
                    listing.cluster
                );
                f.range(t_start, t_end - t_start)
                    .expect("fetch compressed listing cluster");
                return;
            }
            let head = fetch_cluster_head(f, t_start);
            match mode {
                Mode::DenseBlob => {
                    let (b0, b1) = head.blob_range(listing.blob);
                    println!(
                        "dest: listing cluster {} header {} B + blob {} B (blob-range fetch)",
                        listing.cluster,
                        1 + head.offsets[0],
                        b1 - b0
                    );
                    f.range(b0, b1 - b0).expect("fetch listing blob");
                }
                Mode::Sparse => {
                    println!(
                        "dest: listing cluster {} header only: {} B (info + offsets table; blob {} B NOT read at open)",
                        listing.cluster,
                        1 + head.offsets[0],
                        head.offsets.get(1).copied().unwrap_or(0)
                    );
                }
                Mode::Dense => unreachable!(),
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn do_dest(a: &DestArgs) {
    let id = load_or_make_identity(&a.id_dir, "dest");
    let trust = trust_with(&a.src_pin, "source");
    println!("dest: connecting to {}...", a.src_addr);
    let mut sess = Session::connect(&a.src_addr, &id, trust).expect("connect (pin)");
    sess.client_negotiate(vec![]).expect("client negotiate");

    // Empty sparse file of the full size: nothing materialized yet.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&a.out_zim)
        .expect("create out zim");
    file.set_len(a.total).expect("set_len (sparse)");
    let mut fetcher = Fetcher {
        sess: &mut sess,
        digest: a.bulk_digest.to_string(),
        total: a.total,
        file,
        fetched: 0,
        unique: MergedRanges::default(),
    };

    // Stage 1: the 80-byte header.
    fetcher.range(0, 80).expect("fetch header");
    let mut hdr = [0u8; 80];
    {
        use std::os::unix::fs::FileExt as _;
        fetcher
            .file
            .read_exact_at(&mut hdr, 0)
            .expect("read header");
    }
    let h = parse_header(&hdr);
    println!(
        "dest: header ok: v{}.{} articles={} clusters={} mime_list@{} title_idx@{} clusters@{} checksum@{}",
        6,
        h.minor_version,
        h.article_count,
        h.cluster_count,
        h.mime_list_pos,
        h.title_idx_pos,
        h.cluster_ptr_pos,
        h.checksum_pos
    );

    let full_mode = a.article_path == "--full";
    if full_mode {
        println!("dest: FULL mode — fetching the whole library");
        fetcher.range(80, a.total - 80).expect("fetch full");
    }

    // Stage 2: the index, per mode.
    // Dirent start = urlPtr[0] (where the dirent region begins).
    let dirent_start;
    let mut st = ProbeState::default();
    let dense_mode = matches!(a.mode, Mode::Dense | Mode::DenseBlob);
    // Located dirents (the sparse emulation fills these during stage 2; the
    // dense modes find them by walking the fetched region afterwards).
    let mut listing: Option<Dirent> = None; // 'X/listing/titleOrdered/v1'
    let mut target: Option<Dirent> = None; // article dirent, redirects resolved
    let mut rescued = false;
    if full_mode {
        dirent_start = if h.article_count > 0 {
            read_u64_at(&fetcher.file, h.path_ptr_pos)
        } else {
            h.cluster_ptr_pos
        };
    } else if dense_mode {
        // The Phase-3 fetch plan, kept verbatim.
        fetcher
            .range(h.cluster_ptr_pos, h.cluster_count as u64 * 8)
            .expect("fetch clusterPtr table");
        fetcher
            .range(h.path_ptr_pos, 8)
            .expect("fetch first urlPtr");
        dirent_start = read_u64_at(&fetcher.file, h.path_ptr_pos);
        let first_cluster_start = read_u64_at(&fetcher.file, h.cluster_ptr_pos);
        println!(
            "dest: mime list [80, {first_cluster_start}); clusters occupy the hole; dirents at [{dirent_start}, {})",
            h.path_ptr_pos
        );
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
    } else {
        // Sparse: fixed metadata (header + mime + clusterPtr table), then the
        // dirent bytes libzim's lazy open will actually read — nothing more.
        assert!(
            h.minor_version >= 1,
            "sparse mode handles new-namespace-scheme ZIMs only — use --dense for old-format files"
        );
        fetcher
            .range(h.cluster_ptr_pos, h.cluster_count as u64 * 8)
            .expect("fetch clusterPtr table");
        let first_cluster_start = read_u64_at(&fetcher.file, h.cluster_ptr_pos);
        fetcher
            .range(80, first_cluster_start - 80)
            .expect("fetch header+mime");
        // getNamespaceRangeBegin/End('C') — run by FileImpl's constructor for
        // new-scheme files (fileimpl.cpp:255-257). Probes dirent 0 first.
        let c_begin = namespace_begin(&mut fetcher, &h, &mut st, b'C');
        let c_end = namespace_begin(&mut fetcher, &h, &mut st, b'D');
        dirent_start = st.offsets[&0];
        println!(
            "dest: probed namespace range 'C' = [{c_begin}, {c_end}); dirent region starts at {dirent_start}"
        );
        // find('X', "listing/titleOrdered/v1") — the open path's title index
        // lookup (fileimpl.cpp:265).
        let (found, lidx) = find_in_range(
            &mut fetcher,
            &h,
            &mut st,
            0,
            h.article_count as u64,
            b'X',
            b"listing/titleOrdered/v1",
        );
        listing = found.then(|| st.dirents[&lidx].clone());
        if listing.is_none() && h.title_idx_pos != u64::MAX && h.title_idx_pos > h.cluster_ptr_pos {
            // Old-format title listing table — the open path needs it.
            fetcher
                .range(h.title_idx_pos, h.article_count as u64 * 4)
                .expect("fetch V0 title table");
        }
        // find('C', article) — what getEntryByPath does for new-scheme files
        // (archive.cpp:290), redirect chain resolved (getItem(true)).
        let (found, aidx) = find_in_range(
            &mut fetcher,
            &h,
            &mut st,
            0,
            h.article_count as u64,
            b'C',
            a.article_path.as_bytes(),
        );
        if found {
            let mut d = st.dirents[&aidx].clone();
            let mut hops = 0;
            while d.is_redirect && hops < 8 {
                d = probe_dirent(&mut fetcher, &h, &mut st, d.redirect_index as u64);
                hops += 1;
            }
            assert!(!d.is_redirect, "redirect chain must end at a real entry");
            target = Some(d);
        }
        println!(
            "dest: sparse index: {} dirent probes ({} wire bytes, {} requests) — vs {} B dense region + {} B urlPtr table",
            st.dirents.len(),
            st.wire,
            st.requests,
            h.path_ptr_pos - dirent_start,
            h.article_count as u64 * 8
        );
    }
    println!(
        "dest: index fetched (wire {}/{} = {:.3}%, unique {}/{} = {:.3}%)",
        fetcher.fetched,
        a.total,
        fetcher.fetched as f64 * 100.0 / a.total as f64,
        fetcher.unique.unique_bytes(),
        a.total,
        fetcher.unique.unique_bytes() as f64 * 100.0 / a.total as f64
    );
    let index_stage_fetched = fetcher.fetched;

    // Stage 3: the read that matters (FULL mode — everything is local).
    if full_mode {
        let article_to_read = if let Some(real) = a.full_article.clone() {
            println!("dest: FULL mode — reading '{real}' after the complete fetch");
            real
        } else {
            let main_idx = read_u32_at(&fetcher.file, 64);
            let main_off = read_u64_at(&fetcher.file, h.path_ptr_pos + main_idx as u64 * 8);
            let d = read_dirent_at(&fetcher.file, main_off).expect("parse main dirent");
            println!("dest: FULL mode — reading the main entry '{}'", d.path);
            d.path
        };
        let sha = Fetcher::try_read_article(&a.out_zim, &article_to_read)
            .unwrap_or_else(|e| panic!("FULL mode must yield a readable article: {e}"));
        println!("dest: article '{article_to_read}' readable (sha256={sha})");
        finish(&mut fetcher, a);
        return;
    }

    // Stage 4: locate the article + listing dirents (dense modes: walk the
    // fetched region; sparse: already done by the emulation above).
    if dense_mode {
        let (t, l) = dense_scan(&fetcher, &h, &a.article_path);
        target = t;
        listing = l;
    }
    if target.is_none() {
        // Fail-closed rescue: sparse emulation missed (or the walk found
        // nothing in a partial region) — fetch the dense index and re-scan.
        println!("dest: article not located yet — dense rescue");
        dense_rescue(&mut fetcher, &h, listing.as_ref());
        rescued = true;
        let (t, l) = dense_scan(&fetcher, &h, &a.article_path);
        target = t;
        listing = listing.or(l);
    }
    let Some(d) = target else {
        eprintln!(
            "dest: FAIL article '{}' not found in the fetched index",
            a.article_path
        );
        std::process::exit(1);
    };
    assert!(!d.is_redirect, "redirect chain must end at a real entry");
    println!(
        "dest: article '{}' lives in cluster {} blob {}",
        a.article_path, d.cluster, d.blob
    );

    // Stage 5: the title listing, per plan.
    if let Some(l) = listing.as_ref() {
        fetch_listing(&mut fetcher, &h, l, dirent_start, a.mode);
    }

    // Honest attempt before the article's bytes arrive: the archive OPENS
    // with index + listing in place; the article read needs its cluster.
    match Fetcher::try_read_article(&a.out_zim, &a.article_path) {
        Ok(sha) => {
            println!(
                "dest: article ALREADY readable (sha256={sha}) — cluster was in the index fetch"
            )
        }
        Err(e) => println!(
            "dest: after index+listing, article not yet readable ({e}) — expected (cluster hole)"
        ),
    }

    // Stage 6: the article's bytes. Compressed clusters come whole (a blob
    // cannot be range-read out of a zstd/lzma stream); an UNCOMPRESSED
    // cluster is fetched as header + the article's blob range only.
    let (c_start, c_end) = cluster_range(&fetcher, &h, d.cluster, dirent_start);
    let info = {
        fetcher.range(c_start, 1).expect("fetch cluster info byte");
        let mut b1 = [0u8; 1];
        use std::os::unix::fs::FileExt as _;
        fetcher
            .file
            .read_exact_at(&mut b1, c_start)
            .expect("read cluster info byte");
        b1[0]
    };
    let comp = info & 0x0F;
    if !rescued && matches!(a.mode, Mode::Sparse) && (comp == 0 || comp == 1) {
        let head = fetch_cluster_head(&mut fetcher, c_start);
        let (b0, b1) = head.blob_range(d.blob);
        println!(
            "dest: cluster {} is UNCOMPRESSED — fetching header {} B + blob range [{b0}, {b1}) = {} B",
            d.cluster,
            1 + head.offsets[0],
            b1 - b0
        );
        fetcher.range(b0, b1 - b0).expect("fetch article blob");
    } else {
        println!(
            "dest: fetching cluster {} whole: range [{c_start}, {c_end}) = {} B (compression 0x{info:02x})",
            d.cluster,
            c_end - c_start
        );
        fetcher
            .range(c_start, c_end - c_start)
            .expect("fetch cluster");
    }

    // Stage 7: the read that matters.
    let sha = match Fetcher::try_read_article(&a.out_zim, &a.article_path) {
        Ok(sha) => sha,
        Err(e) if !rescued => {
            // Fail-closed rescue: the sparse plan's assumptions were wrong
            // somewhere (libzim read a byte we did not fetch). Fetch the
            // dense index and retry — loudly, never a silent false success.
            println!("dest: read failed after the fetch ({e}) — dense rescue + retry");
            dense_rescue(&mut fetcher, &h, listing.as_ref());
            rescued = true;
            Fetcher::try_read_article(&a.out_zim, &a.article_path)
                .unwrap_or_else(|e2| panic!("FAIL read even after dense rescue: {e2}"))
        }
        Err(e) => panic!("FAIL read even after dense rescue: {e}"),
    };

    let fetched = fetcher.fetched;
    let unique = fetcher.unique.unique_bytes();
    println!(
        "dest: ARTICLE READABLE — wire {fetched}/{} bytes = {:.4}% | unique {unique}/{} bytes = {:.4}% of the ZIM (index wire {index_stage_fetched})",
        a.total,
        fetched as f64 * 100.0 / a.total as f64,
        a.total,
        unique as f64 * 100.0 / a.total as f64,
    );
    if rescued {
        println!("dest: note — dense rescue was needed (see above)");
    }
    println!("dest: article sha256={sha}");
    finish(&mut fetcher, a);
}

/// Epilogue shared by every successful path: tell the source we are done,
/// print the real wire counters.
fn finish(fetcher: &mut Fetcher, a: &DestArgs) {
    fetcher
        .sess
        .send(Message::ImportComplete {
            session: "kiwix-materialize".into(),
            cut_number: 0,
            manifest_digest: a.bulk_digest.clone(),
        })
        .expect("send done");
    println!(
        "dest: wire sent={} recv={}",
        fetcher.sess.bytes_sent(),
        fetcher.sess.bytes_recv()
    );
    println!("dest: MATERIALIZE_OK");
}

/// Dense walk over the materialized region (the Phase-3 way): find the
/// article dirent (redirect chain resolved) and the title-listing dirent.
fn dense_scan(f: &Fetcher, h: &ZimHeader, article: &str) -> (Option<Dirent>, Option<Dirent>) {
    let mut offsets = Vec::with_capacity(h.article_count as usize);
    for i in 0..h.article_count as u64 {
        offsets.push(read_u64_at(&f.file, h.path_ptr_pos + i * 8));
    }
    let mut target: Option<Dirent> = None;
    let mut listing: Option<Dirent> = None;
    let mut parsed = 0u64;
    let mut hops = 0;
    for off in &offsets {
        let Some(x) = read_dirent_at(&f.file, *off) else {
            continue;
        };
        parsed += 1;
        if listing.is_none()
            && x.ns == b'X'
            && x.path == "listing/titleOrdered/v1"
            && !x.is_redirect
        {
            listing = Some(x.clone());
        }
        if target.is_none() && x.path == article {
            let mut d = x;
            while d.is_redirect && hops < 8 {
                let o = offsets
                    .get(d.redirect_index as usize)
                    .copied()
                    .expect("redirect index in range");
                d = read_dirent_at(&f.file, o).expect("parse redirect target");
                hops += 1;
            }
            target = Some(d);
        }
    }
    println!(
        "dest: dense scan walked {} urlPtr offsets, parsed {parsed} dirents",
        offsets.len()
    );
    (target, listing)
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
        Some("dest") if args.len() >= 8 => {
            let full_article = if args[7] == "--full" {
                args.get(8).cloned() // real article to read after a full fetch
            } else {
                None
            };
            let mode = if args[7] == "--full" {
                Mode::Sparse // irrelevant in full mode
            } else {
                match args.get(8).map(String::as_str) {
                    Some("--dense") => Mode::Dense,
                    Some("--dense-blob") => Mode::DenseBlob,
                    _ => Mode::Sparse,
                }
            };
            do_dest(&DestArgs {
                id_dir: args[1].clone(),
                out_zim: args[2].clone(),
                src_addr: args[3].clone(),
                src_pin: args[4].clone(),
                bulk_digest: args[5].clone(),
                total: args[6].parse().expect("total as u64"),
                article_path: args[7].clone(),
                full_article,
                mode,
            })
        }
        _ => {
            eprintln!(
                "usage:\n  kiwix_materialize source <id_dir> <zim_path> <bind_addr> <dest_pin_hex> [article_path]\n  kiwix_materialize dest <id_dir> <out_zim> <src_addr> <src_pin_hex> <bulk_digest> <total> <article_path> [--dense|--dense-blob]\n  modes: sparse (default: lazy dirent probes + listing cluster header + article blob), --dense-blob (dense index + listing blob), --dense (exact Phase-3 fetch plan)"
            );
            std::process::exit(2);
        }
    }
}
