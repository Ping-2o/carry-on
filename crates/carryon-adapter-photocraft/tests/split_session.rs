//! Split-document sessions (TILE-SPLITTING.md): the saved document carried as
//! ONE manifest object + parent-linked content-addressed tile/blob objects.
//!
//! Default-feature tests use a **synthetic** bundle to pin the adapter contract
//! (entry order, parent links, auth-hash determinism, closure validation) with
//! no PhotoCraft dependency. The `inprocess` module at the bottom repeats the
//! critical path against a REAL PhotoCraft directory bundle (pin `1f7fc0f5…`).

use carryon_adapter_api::Adapter as _;
use carryon_adapter_photocraft::model::{
    DocumentBundle, DocumentEncoding, PhotocraftMeta, Workspace, SCHEMA_DOC_MANIFEST, SCHEMA_META,
    SCHEMA_UNSAVED, SCHEMA_WORKSPACE,
};
use carryon_adapter_photocraft::PhotocraftAdapter;
use std::collections::BTreeMap;

/// A synthetic split bundle: manifest referencing two tiles + one blob by
/// bare BLAKE3 hash (the real manifest shape, `manifest.rs`), plus one part
/// the manifest does not reference (extra parts are ignored by PhotoCraft's
/// loader on open and by the carry's GC discipline only on save).
fn synthetic_bundle() -> DocumentBundle {
    let manifest = serde_json::json!({
        "format_version": 1,
        "document": {
            "name": "synthetic.pcraft",
            "size": { "width": 512, "height": 512 },
            "layers": [
                { "name": "base", "tiles": ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"] },
                { "name": "meta", "blob": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc" }
            ]
        }
    });
    let mut parts = BTreeMap::new();
    parts.insert(
        "tiles/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.zst".into(),
        b"tile-aaaa".to_vec(),
    );
    parts.insert(
        "tiles/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.zst".into(),
        b"tile-bbbb".to_vec(),
    );
    parts.insert(
        "blobs/cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc.zst".into(),
        b"blob-cccc".to_vec(),
    );
    // Present but NOT referenced: harmless on open (loader ignores it); the
    // canonical hash covers all parts deterministically on BOTH sides.
    parts.insert(
        "tiles/dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd.zst".into(),
        b"tile-unreferenced".to_vec(),
    );
    DocumentBundle {
        manifest: serde_json::to_vec(&manifest).unwrap(),
        parts,
    }
}

fn meta() -> PhotocraftMeta {
    PhotocraftMeta {
        schema_id: "carryon.photocraft.session".into(),
        schema_version: 1,
        adapter_version: "0.1.0".into(),
        original_path: None,
        document_name: "synthetic.pcraft".into(),
        revision: 4,
        saved_revision: 3,
        dirty: true,
        photocraft_pin: "1f7fc0f53696e54da79b95015abb37d7c22cdd1a".into(),
        unsaved_mode: carryon_adapter_photocraft::model::UnsavedMode::Journal,
        document_encoding: DocumentEncoding::Split,
    }
}

fn split_adapter() -> PhotocraftAdapter {
    PhotocraftAdapter::with_split_state(
        "split-session",
        synthetic_bundle(),
        br#"[["paint.stroke",{"size":8}]]"#.to_vec(),
        Workspace::default(),
        meta(),
    )
}

#[test]
fn split_entries_are_manifest_plus_parent_linked_parts() {
    let mut a = split_adapter();
    let token = a.begin_snapshot("split-session", 1).expect("generation 1");
    let manifest = a.describe_snapshot(&token).expect("describe");
    let ids: Vec<(String, Vec<(String, u64)>)> = manifest
        .objects
        .iter()
        .map(|e| {
            (
                e.object_id.clone(),
                e.parents
                    .iter()
                    .map(|p| (p.object_id.clone(), p.generation))
                    .collect(),
            )
        })
        .collect();
    // Admission-priority order: meta, unsaved, manifest, parts, workspace last
    // (TILE-SPLITTING.md §5: a small budget window banks identity + journal +
    // manifest before any pixels).
    assert_eq!(ids[0].0, SCHEMA_META);
    assert_eq!(ids[1].0, SCHEMA_UNSAVED);
    assert_eq!(ids[2].0, SCHEMA_DOC_MANIFEST);
    // Parts sorted by object id: `blob.*` < `tile.*` (deterministic manifest order).
    assert_eq!(
        ids[3].0,
        "photocraft.blob.cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc.v1"
    );
    assert_eq!(
        ids[4].0,
        "photocraft.tile.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.v1"
    );
    assert_eq!(
        ids[5].0,
        "photocraft.tile.bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.v1"
    );
    assert_eq!(
        ids[6].0,
        "photocraft.tile.dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd.v1"
    );
    assert_eq!(ids[7].0, SCHEMA_WORKSPACE);
    // Parent linkage: the unsaved journal and every part hang off the manifest.
    assert_eq!(ids[1].1, vec![(SCHEMA_DOC_MANIFEST.to_string(), 1)]);
    for entry in ids.iter().take(7).skip(3) {
        assert_eq!(
            entry.1,
            vec![(SCHEMA_DOC_MANIFEST.to_string(), 1)],
            "object {} must be parent-linked to the document manifest",
            entry.0
        );
    }
    assert!(ids[2].1.is_empty(), "manifest is the root");
    // Every split object except the workspace is authoritative.
    for e in &manifest.objects {
        let expect_auth = e.object_id != SCHEMA_WORKSPACE;
        assert_eq!(
            matches!(e.kind, carryon_adapter_api::ObjectKindWire::Authoritative),
            expect_auth,
            "{}",
            e.object_id
        );
    }
}

#[test]
fn referenced_hashes_are_the_manifests_bare_blake3_strings() {
    let bundle = synthetic_bundle();
    assert_eq!(
        bundle.referenced_hashes(),
        vec![
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_string()
        ]
    );
}

#[test]
fn split_auth_hash_is_deterministic_and_covers_all_parts() {
    let a = synthetic_bundle();
    let h1 = {
        // Same logical content, different part-map insertion order: the
        // canonical bytes must not move (sorted-path order is the fixpoint).
        let mut parts = BTreeMap::new();
        parts.insert(
            "blobs/cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc.zst".into(),
            b"blob-cccc".to_vec(),
        );
        parts.insert(
            "tiles/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.zst".into(),
            b"tile-bbbb".to_vec(),
        );
        parts.insert(
            "tiles/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.zst".into(),
            b"tile-aaaa".to_vec(),
        );
        parts.insert(
            "tiles/dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd.zst".into(),
            b"tile-unreferenced".to_vec(),
        );
        let b = DocumentBundle {
            manifest: a.manifest.clone(),
            parts,
        };
        b.canonical_bytes()
    };
    let h2 = synthetic_bundle().canonical_bytes();
    assert_eq!(h1, h2, "canonical bundle bytes must be order-stable");
    // Changing ANY part content must move the hash (authority is state-bound).
    let mut changed = synthetic_bundle();
    changed.parts.insert(
        "tiles/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.zst".into(),
        b"tile-changed".to_vec(),
    );
    assert_ne!(changed.canonical_bytes(), h2);
    // A different manifest moves the hash too.
    let mut other = synthetic_bundle();
    other.manifest = serde_json::to_vec(&serde_json::json!({"format_version": 2})).unwrap();
    assert_ne!(other.canonical_bytes(), h2);
}

#[test]
fn split_mode_validates_part_object_ids_and_resolves_the_full_closure() {
    let a = split_adapter();
    use carryon_adapter_api::{ActionRequestWire, CutRef};
    let cut = CutRef {
        session: "split-session".into(),
        cut_number: 0,
    };
    let req = ActionRequestWire {
        class: "document.edit".into(),
        params: serde_json::json!({"steps": []}),
    };
    let plan = a.resolve_action(&cut, &req).expect("resolve");
    let prereq: Vec<&str> = plan
        .prerequisites
        .iter()
        .map(|p| p.object_id.as_str())
        .collect();
    assert!(prereq.contains(
        &"photocraft.tile.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.v1"
    ));
    assert!(prereq.contains(
        &"photocraft.blob.cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc.v1"
    ));
    assert!(prereq.contains(&SCHEMA_DOC_MANIFEST));
    // Every part object validates.
    let versions: Vec<carryon_adapter_api::ObjectVersionWire> = plan.prerequisites.clone();
    let report = a.validate_objects(&cut, &versions).expect("validate");
    assert!(report.ok, "{:?}", report.missing);
    // An unknown id still fails closed.
    let mut bad = versions;
    bad.push(carryon_adapter_api::ObjectVersionWire {
        object_id: "photocraft.tile.not-declared.zst".into(),
        generation: 1,
    });
    let report = a.validate_objects(&cut, &bad).expect("validate");
    assert!(!report.ok);
}

#[test]
fn constructors_pin_the_meta_encoding_to_the_byte_shape() {
    let m = meta();
    // with_state (monolithic constructor) relabels Split → Monolithic: the
    // meta can never claim an encoding the bytes do not have.
    let mut mono = PhotocraftAdapter::with_state(
        "s",
        b"zip-bytes".to_vec(),
        b"unsaved".to_vec(),
        Workspace::default(),
        m.clone(),
    );
    let token = mono.begin_snapshot("s", 1).unwrap();
    let entries = mono.describe_snapshot(&token).unwrap();
    assert!(entries
        .objects
        .iter()
        .any(|e| e.object_id == "photocraft.document.v1"));
    assert!(entries
        .objects
        .iter()
        .all(|e| e.object_id != SCHEMA_DOC_MANIFEST));
    // Round-trip the meta through JSON: the default (absent field) is
    // Monolithic — earlier-phase carried state still reads.
    let mut stripped = serde_json::to_value(&m).unwrap();
    stripped
        .as_object_mut()
        .unwrap()
        .remove("document_encoding");
    let back: PhotocraftMeta = serde_json::from_value(stripped).unwrap();
    assert_eq!(back.document_encoding, DocumentEncoding::Monolithic);
}

#[cfg(feature = "inprocess")]
mod inprocess {
    use carryon_adapter_photocraft::inprocess::{InProcessConfig, InProcessDriver};
    use carryon_adapter_photocraft::model::UnsavedMode;
    use carryon_adapter_photocraft::{PhotocraftAdapter, PhotocraftDriver as _};
    use serde_json::json;
    use std::path::PathBuf;

    /// ONE shared work root per test process (the driver's fail-closed root is
    /// a process-wide OnceLock, so drivers in one process must share it; each
    /// test uses its own SUBTREE).
    fn root() -> PathBuf {
        let d = std::env::temp_dir().join(format!("pcraft-split-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// REAL round-trip: layered document saved as a directory bundle, read
    /// back as a `DocumentBundle`, reconstructed on a second driver, opened,
    /// journal replayed — composite pixels identical (the TILE-SPLITTING.md
    /// §3 oracle). A holed bundle must fail closed at open: PhotoCraft's own
    /// loader is the closure check.
    #[test]
    fn bundle_round_trip_through_real_photocraft() {
        let work = root();
        let mut src = InProcessDriver::new(InProcessConfig {
            work_root: work.clone(),
        })
        .unwrap();
        src.session_mut()
            .execute(
                "file.new",
                json!({"width": 600, "height": 400, "mode": "rgb", "depth": 8, "background": "white"}),
            )
            .unwrap();
        src.session_mut()
            .execute("layer.new.layer", json!({"name": "L1"}))
            .unwrap();
        src.session_mut()
            .execute("layer.select", json!({"name": "L1"}))
            .unwrap();
        src.session_mut()
            .execute(
                "paint.stroke",
                json!({"points": [[100, 100, 1.0]], "size": 40, "color": "#0a0"}),
            )
            .unwrap();
        // Split-mode base save: a directory bundle.
        src.save_active_bundle_under_root("roundtrip/base").unwrap();
        // Post-save dirty edits (what the journal carries).
        src.session_mut()
            .execute(
                "paint.stroke",
                json!({"points": [[300, 200, 1.0]], "size": 20, "color": "#c33"}),
            )
            .unwrap();
        assert!(src.meta().unwrap().dirty);

        let bundle = src.document_bundle_of_active().unwrap();
        assert!(!bundle.manifest.is_empty());
        assert!(
            bundle.parts.len() >= 2,
            "a 600×400 layered doc must have at least a few tiles, got {}",
            bundle.parts.len()
        );
        assert!(
            bundle.referenced_hashes().len() >= 2,
            "the manifest must reference its tiles by bare hash"
        );
        let journal = src.unsaved_bytes_as(UnsavedMode::Journal).unwrap();
        assert!(!journal.is_empty());

        // "Destination": write the bundle out, open it, replay the journal.
        let mut dst = InProcessDriver::new(InProcessConfig {
            work_root: work.clone(),
        })
        .unwrap();
        dst.write_bundle_under_root("roundtrip/carried", &bundle)
            .unwrap();
        let opened = dst.open_under_root("roundtrip/carried").unwrap();
        assert_eq!(opened, 0);
        let steps: Vec<carryon_adapter_photocraft::Step> =
            serde_json::from_slice::<Vec<(String, serde_json::Value)>>(&journal)
                .unwrap()
                .into_iter()
                .map(|(id, params)| carryon_adapter_photocraft::Step { id, params })
                .collect();
        dst.apply_steps(&steps).unwrap();

        // The oracle: composite pixels identical across the reconstruction.
        let a = carryon_adapter_photocraft::model::hex_sha256(
            &src.composite_bytes_of_active().unwrap(),
        );
        let b = carryon_adapter_photocraft::model::hex_sha256(
            &dst.composite_bytes_of_active().unwrap(),
        );
        assert_eq!(a, b, "reconstructed bundle must render identical pixels");

        // Fail-closed: drop a referenced part → the WRITE still succeeds
        // (extra/missing files are not the writer's call), but the OPEN must
        // refuse — PhotoCraft's loader errors on the first missing fetch.
        let mut holey = bundle.clone();
        let hole = holey.referenced_hashes()[0].clone();
        holey.parts.remove(&format!("tiles/{hole}.zst"));
        holey.parts.remove(&format!("blobs/{hole}.zst"));
        dst.write_bundle_under_root("roundtrip/holey", &holey)
            .unwrap();
        assert!(
            dst.open_under_root("roundtrip/holey").is_err(),
            "a bundle with a missing referenced part must fail closed at open"
        );
    }

    /// The adapter + core end to end in split mode: seal a cut whose document
    /// closure is manifest + tiles, publish (two-phase per tile), then a
    /// destination imports under budgets — progressive windows bank tiles and
    /// a replenished window completes the carry (§5 of the design).
    #[test]
    fn split_cut_seals_and_imports_progressively() {
        use carryon_adapter_photocraft::model::PhotocraftMeta;
        use carryon_core::carryon_net::{pair_devices, DeviceIdentity, Session};
        use carryon_core::model::{AuthorityMode, Budget, Sensitivity};
        use carryon_core::{Core, CreateSessionReq, ImportOutcome};
        use std::net::TcpListener;
        use std::sync::Arc;

        let work = root();
        let mut d = InProcessDriver::new(InProcessConfig {
            work_root: work.clone(),
        })
        .unwrap();
        d.session_mut()
            .execute(
                "file.new",
                json!({"width": 800, "height": 600, "mode": "rgb", "depth": 8, "background": "white"}),
            )
            .unwrap();
        d.session_mut()
            .execute("layer.new.layer", json!({"name": "Paint"}))
            .unwrap();
        d.session_mut()
            .execute("layer.select", json!({"name": "Paint"}))
            .unwrap();
        d.session_mut()
            .execute(
                "paint.stroke",
                json!({"points": [[400, 300, 1.0]], "size": 90, "color": "#36c"}),
            )
            .unwrap();
        d.save_active_bundle_under_root("e2e/base").unwrap();
        d.session_mut()
            .execute(
                "paint.stroke",
                json!({"points": [[200, 150, 1.0]], "size": 30, "color": "#fc3"}),
            )
            .unwrap();
        let bundle = d.document_bundle_of_active().unwrap();
        let journal = d.unsaved_bytes_as(UnsavedMode::Journal).unwrap();
        let meta: PhotocraftMeta = d.meta().unwrap();
        assert_eq!(
            meta.document_encoding,
            carryon_adapter_photocraft::model::DocumentEncoding::Split
        );

        // Source core: seal the split cut.
        let mut source = Core::open(&work.join("e2e/src-core")).unwrap();
        let info = source
            .register_adapter(Box::new(PhotocraftAdapter::with_split_state(
                "split-e2e",
                bundle.clone(),
                journal.clone(),
                Default::default(),
                meta.clone(),
            )))
            .unwrap();
        let session = source
            .create_session(CreateSessionReq {
                adapter_id: info.adapter_id,
                title: "split-e2e".into(),
                privacy: Sensitivity::Public,
                authority_mode: AuthorityMode::SingleWriter,
            })
            .unwrap();
        let cut = source.create_cut(session).unwrap();
        let sess_str = session.to_string();

        // The cut seals the full document closure: manifest + every part.
        let sealed = source.cut_authoritative_objects(cut);
        assert!(sealed.contains(&"photocraft.document.manifest.v1".to_string()));
        assert!(sealed.iter().any(|id| id.starts_with("photocraft.tile.")));
        assert!(sealed.iter().all(|id| id != "photocraft.document.v1"));

        // Destination: import under budgets (progressive windows).
        let mut dest = Core::open(&work.join("e2e/dst-core")).unwrap();
        let (sid, did, st, dt) = {
            let s = DeviceIdentity::generate("s").unwrap();
            let dd = DeviceIdentity::generate("d").unwrap();
            let (st, dt) = pair_devices(&s, &dd, "e2e").unwrap();
            (s, dd, Arc::new(st), Arc::new(dt))
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let srv = std::thread::spawn(move || {
            for _ in 0..3 {
                if let Ok(mut s) = Session::accept(&listener, &sid, st.clone()) {
                    let _ = s.server_negotiate(vec![]);
                    let _ = source.serve_cut(&mut s);
                }
            }
        });

        // Total closure bytes: the adapter's declared object sizes.
        let total: u64 = {
            let meta_len = serde_json::to_vec(&meta).unwrap().len() as u64;
            let mut t = bundle.manifest.len() as u64 + journal.len() as u64 + meta_len;
            for b in bundle.parts.values() {
                t += b.len() as u64;
            }
            t
        };
        // Window 1: a third of the closure.
        let mut b1 = Budget::local_default();
        b1.total_net_bytes = total / 3;
        dest.set_budget(b1);
        let mut s1 = Session::connect(&addr, &did, dt.clone()).unwrap();
        s1.client_negotiate(vec![]).unwrap();
        let out1 = dest.import_cut(&mut s1, &sess_str, cut.number).unwrap();
        let partial = match out1 {
            ImportOutcome::PartialBudget(p) => p,
            other => panic!("window 1 must be budget-partial, got {other:?}"),
        };
        assert!(!partial.published.is_empty());
        assert!(!partial.deferred.is_empty());
        // Admission order: the tiny critical objects move first.
        assert!(partial
            .published
            .iter()
            .any(|(id, _)| id == "photocraft.meta.v1"));
        assert!(partial
            .deferred
            .iter()
            .any(|id| id.starts_with("photocraft.tile.")));
        drop(s1);

        // Window 2: same small budget — present objects skip (dedup), the
        // deferred tail must shrink.
        let mut b2 = Budget::local_default();
        b2.total_net_bytes = total / 3;
        dest.set_budget(b2);
        let mut s2 = Session::connect(&addr, &did, dt.clone()).unwrap();
        s2.client_negotiate(vec![]).unwrap();
        let out2 = dest.import_cut(&mut s2, &sess_str, cut.number).unwrap();
        match &out2 {
            ImportOutcome::PartialBudget(p) => {
                assert!(
                    p.deferred.len() < partial.deferred.len(),
                    "window 2 must shrink the deferred tail (dedup)"
                );
            }
            ImportOutcome::Completed(_) => {}
            ImportOutcome::Suspended(_) => panic!("no suspend requested"),
        }
        drop(s2);

        // Window 3: replenished — completes.
        dest.set_budget(Budget::local_default());
        let mut s3 = Session::connect(&addr, &did, dt).unwrap();
        s3.client_negotiate(vec![]).unwrap();
        let out3 = dest.import_cut(&mut s3, &sess_str, cut.number).unwrap();
        out3.completed_cut()
            .expect("window 3 completes the closure");
        let _ = srv.join();

        // Reconstruct from the DESTINATION's own store by digest and compare
        // every byte (the two-phase publish verified the digests at import;
        // this re-reads what the destination actually stored).
        use carryon_adapter_photocraft::model::{hex_sha256, part_object_id};
        let mut restored = carryon_adapter_photocraft::model::DocumentBundle {
            manifest: dest.read_object_hex(&hex_sha256(&bundle.manifest)).unwrap(),
            parts: Default::default(),
        };
        for (path, bytes) in &bundle.parts {
            assert!(part_object_id(path).is_some());
            restored.parts.insert(
                path.clone(),
                dest.read_object_hex(&hex_sha256(bytes)).unwrap(),
            );
        }
        assert_eq!(restored.manifest, bundle.manifest);
        for (path, bytes) in &restored.parts {
            assert_eq!(bytes, &bundle.parts[path], "part {path} round-trips");
        }

        // And the reconstructed bundle opens + replays identically (the full
        // destination-side story, on the same shared root).
        let mut dst = InProcessDriver::new(InProcessConfig {
            work_root: work.clone(),
        })
        .unwrap();
        dst.write_bundle_under_root("e2e/reconstructed", &restored)
            .unwrap();
        dst.open_under_root("e2e/reconstructed").unwrap();
        let steps: Vec<carryon_adapter_photocraft::Step> =
            serde_json::from_slice::<Vec<(String, serde_json::Value)>>(&journal)
                .unwrap()
                .into_iter()
                .map(|(id, params)| carryon_adapter_photocraft::Step { id, params })
                .collect();
        dst.apply_steps(&steps).unwrap();
        let a =
            carryon_adapter_photocraft::model::hex_sha256(&d.composite_bytes_of_active().unwrap());
        let b = carryon_adapter_photocraft::model::hex_sha256(
            &dst.composite_bytes_of_active().unwrap(),
        );
        assert_eq!(a, b, "carried-then-replayed pixels must match the source's");
    }
}
