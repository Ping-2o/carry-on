//! Real-backend tests: the C++ shim + libzim FFI against REAL ZIM files.
//!
//! These run only when a real ZIM is available:
//! - `CARRYON_KIWIX_TEST_ZIM` set explicitly ⇒ the file MUST exist (fail on
//!   absence — never silently claim a pass);
//! - unset ⇒ defaults to the SSD copy of kiwix's testing-suite
//!   `wikipedia_en_climate_change_mini_2024-06.zim`; skipped with a printed
//!   note if that path is absent.
//!
//! Label: these tests are the IMPLEMENTED evidence for the macOS-host backend
//! (log tails in docs/kiwix-bridge/BUILD-EVIDENCE.md §G).

use carryon_adapter_api::*;
use carryon_adapter_kiwix::*;

const DEFAULT_ZIM: &str = "/Volumes/kbwork/out/SOURCE/zim-testing-suite-0.10.0/nons/wikipedia_en_climate_change_mini_2024-06.zim";

fn test_zim() -> Option<String> {
    match std::env::var("CARRYON_KIWIX_TEST_ZIM") {
        Ok(p) => {
            assert!(
                std::path::Path::new(&p).exists(),
                "CARRYON_KIWIX_TEST_ZIM={p} does not exist — refusing to fake a pass"
            );
            Some(p)
        }
        Err(_) => {
            if std::path::Path::new(DEFAULT_ZIM).exists() {
                Some(DEFAULT_ZIM.to_string())
            } else {
                eprintln!("SKIP: no real ZIM available ({DEFAULT_ZIM} absent, CARRYON_KIWIX_TEST_ZIM unset)");
                None
            }
        }
    }
}

#[test]
fn opens_real_zim_and_reads_identity() {
    let Some(path) = test_zim() else { return };
    let arch = zim_backend::ZimArchive::open(&path).expect("open real zim");
    let info = arch.info().expect("info");
    assert!(!info.uuid.is_empty(), "real zims carry a uuid");
    assert!(info.filesize > 0);
    assert!(info.entry_count > 0);
    assert!(info.main_path.is_some(), "main page resolvable");
    let fs_size = std::fs::metadata(&path).unwrap().len();
    assert_eq!(info.filesize, fs_size, "archive size == file size");
}

#[test]
fn chunked_article_reads_reassemble_to_full_item() {
    let Some(path) = test_zim() else { return };
    let arch = zim_backend::ZimArchive::open(&path).expect("open");
    let info = arch.info().expect("info");
    let main = info.main_path.expect("main path");

    let item = arch.item(&main).expect("resolve main");
    let size = item.size();
    assert!(size > 0, "the main page has content");

    // Chunked: 1000-byte windows via the offset/length primitive.
    let mut reassembled = Vec::new();
    let mut off = 0u64;
    loop {
        let part = item.read_range(off, 1000).expect("chunk read");
        if part.is_empty() {
            break;
        }
        reassembled.extend_from_slice(&part);
        off += part.len() as u64;
    }
    assert_eq!(reassembled.len() as u64, size);

    // Whole read agrees with the chunked reassembly.
    let whole = item.read_range(0, size).expect("whole read");
    assert_eq!(whole, reassembled);

    // It is HTML.
    let mime = item.mimetype().expect("mimetype");
    assert!(mime.contains("html"), "main page is html: {mime}");
}

#[test]
fn authority_transfers_across_machines_when_the_local_zim_path_differs() {
    // §21.2: the destination re-derives the proposal hash from ITS OWN state.
    // Two machines hold the SAME ZIM at DIFFERENT local paths — the portable
    // state (uuid+size+sha256, position) must agree even though the session
    // object's `filename` differs. Fail closed on mismatch would otherwise
    // make every real cross-machine transfer impossible.
    let Some(path_a) = test_zim() else { return };
    let path_b = std::env::var("CARRYON_KIWIX_TEST_ZIM_ALT").ok();
    let Some(path_b) = path_b else {
        eprintln!(
            "SKIP: set CARRYON_KIWIX_TEST_ZIM_ALT to a second copy/path of the same \
             ZIM to exercise the cross-path authority case"
        );
        return;
    };
    assert!(
        std::path::Path::new(&path_b).exists(),
        "CARRYON_KIWIX_TEST_ZIM_ALT={path_b} does not exist — refusing to fake a pass"
    );

    let cut = CutRef {
        session: "s".into(),
        cut_number: 1,
    };
    let mut source = KiwixAdapter::from_zim("x", &path_a, "index", 500)
        .expect("from_zim at path A");
    let mut dest = KiwixAdapter::from_zim("x", &path_b, "index", 500)
        .expect("from_zim at path B");
    // The local paths differ but the identity must not.
    assert_ne!(source.state().zim.filename, dest.state().zim.filename);
    assert_eq!(source.state().zim.uuid, dest.state().zim.uuid);
    assert_eq!(source.state().zim.bulk_sha256, dest.state().zim.bulk_sha256);

    let proposal = source
        .prepare_authority_transfer(&cut)
        .expect("prepare at path A");
    let receipt = dest
        .accept_authority_transfer(&cut, &proposal)
        .expect("accept at path B — portable hash must agree across paths");
    let relinquish = source
        .commit_authority_transfer("p", &receipt)
        .expect("commit");
    assert!(relinquish.starts_with("relinquish:"));
}

#[test]
fn unknown_entry_fails_closed() {
    let Some(path) = test_zim() else { return };
    let arch = zim_backend::ZimArchive::open(&path).expect("open");
    let err = match arch.item("A/This_Entry_Does_Not_Exist_At_All") {
        Err(e) => e,
        Ok(_) => panic!("must fail closed, got an item"),
    };
    assert!(matches!(
        err,
        zim_backend::ZimBackendError::EntryNotFound(_)
    ));
}

#[test]
fn from_zim_builds_a_real_session_and_serves_bulk_at_file_offsets() {
    let Some(path) = test_zim() else { return };

    // Resolve the main page path first (it is the natural "current article").
    let arch = zim_backend::ZimArchive::open(&path).expect("open");
    let main = arch.info().expect("info").main_path.expect("main");
    drop(arch);

    let mut adapter =
        KiwixAdapter::from_zim("kiwix-real", &path, &main, 12_345).expect("from_zim on a real zim");

    // The session object carries the real identity.
    assert_eq!(adapter.state().zim.filename, path);
    assert_eq!(adapter.state().position.article_path, main);
    assert_eq!(adapter.state().position.scroll_y, 12_345);

    // The article object holds the real main-page bytes.
    let gen = adapter.current_generation("s").expect("gen");
    let token = adapter.begin_snapshot("s", gen).expect("begin");
    let manifest = adapter.describe_snapshot(&token).expect("manifest");
    let article = manifest
        .objects
        .iter()
        .find(|o| o.object_id == "kiwix.article.v1")
        .expect("article entry");
    assert!(article.logical_size > 0);

    let mut article_bytes = Vec::new();
    let mut off = 0u64;
    loop {
        let part = adapter
            .read_object(&token, "kiwix.article.v1", off, 512)
            .expect("article chunk");
        if part.is_empty() {
            break;
        }
        article_bytes.extend_from_slice(&part);
        off += part.len() as u64;
    }
    assert_eq!(article_bytes.len() as u64, article.logical_size);

    // The bulk object is declared with the real file size and serves at raw
    // file offsets (what a destination materializes cluster-by-cluster).
    let bulk = manifest
        .objects
        .iter()
        .find(|o| o.object_id == "kiwix.zimfile.v1")
        .expect("bulk entry");
    let fs_size = std::fs::metadata(&path).unwrap().len();
    assert_eq!(bulk.logical_size, fs_size);
    assert_eq!(bulk.content_hash.len(), 64, "streaming sha256 of the file");

    // First 80 bytes = the ZIM header (magic + version) — chunked from disk.
    let head = adapter
        .read_object(&token, "kiwix.zimfile.v1", 0, 80)
        .expect("zim header chunk");
    assert_eq!(head.len(), 80);
    assert_eq!(&head[0..4], b"ZIM\x04", "zim magic at offset 0");

    // A mid-file chunk is readable and non-empty.
    let mid_off = fs_size / 2;
    let mid = adapter
        .read_object(&token, "kiwix.zimfile.v1", mid_off, 4096)
        .expect("mid-file chunk");
    assert_eq!(mid.len(), 4096);

    // Whole-bulk reassembly matches the streaming digest declared in the
    // manifest (1 MiB chunks, same as the bench path).
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    let mut off = 0u64;
    loop {
        let part = adapter
            .read_object(&token, "kiwix.zimfile.v1", off, 1 << 20)
            .expect("bulk chunk");
        if part.is_empty() {
            break;
        }
        h.update(&part);
        off += part.len() as u64;
    }
    let mut hex = String::new();
    for b in h.finalize() {
        hex.push_str(&format!("{b:02x}"));
    }
    assert_eq!(hex, bulk.content_hash, "chunked bulk == declared digest");

    // reader.restore reports the real session (oracle-checked).
    let cut = CutRef {
        session: "kiwix-real".into(),
        cut_number: 1,
    };
    let restore = adapter
        .execute_action(
            &cut,
            &ActionRequestWire {
                class: "reader.restore".into(),
                params: serde_json::json!({}),
            },
        )
        .expect("restore");
    assert!(restore.oracle.agreed);
    assert_eq!(restore.output["scroll_y"], serde_json::json!(12_345));
    assert!(restore.output["article_hash"].as_str().unwrap().len() == 64);

    // reader.navigate extracts the NEW article's bytes from the real ZIM.
    let second = adapter
        .execute_action(
            &cut,
            &ActionRequestWire {
                class: "reader.navigate".into(),
                params: serde_json::json!({"path": main, "scroll_y": 999}),
            },
        )
        .expect("navigate on real zim");
    assert_eq!(
        second.output["article_bytes_available"],
        serde_json::json!(true),
        "real backend extracts article bytes on navigate"
    );

    // Navigating to a nonexistent entry refuses the whole action (fail
    // closed) and leaves the session untouched.
    let gen_before = adapter.current_generation("s").unwrap();
    let err = adapter
        .execute_action(
            &cut,
            &ActionRequestWire {
                class: "reader.navigate".into(),
                params: serde_json::json!({"path": "A/No_Such_Entry_ZZZ"}),
            },
        )
        .expect_err("must refuse");
    assert!(matches!(err, AdapterError::Incompatible(_)));
    assert_eq!(adapter.current_generation("s").unwrap(), gen_before);
}
