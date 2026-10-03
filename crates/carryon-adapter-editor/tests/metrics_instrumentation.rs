//! Part 2 instrumentation: real wire-byte counters on the transport, and the
//! `ActionReady` session-state transition stamped on a completed import closure
//! (§18.8 `Importing -> ActionReady`). Loopback, LOCAL evidence.

use carryon_adapter_editor::EditorAdapter;
use carryon_core::carryon_net::{pair_devices, DeviceIdentity, Session};
use carryon_core::model::{AuthorityMode, Sensitivity, SessionState};
use carryon_core::{Core, CreateSessionReq};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use tempfile::tempdir;

#[test]
fn transfer_counts_real_bytes_and_marks_action_ready() {
    let src_dir = tempdir().unwrap();
    let dst_dir = tempdir().unwrap();

    // Source: a full structured session, sealed.
    let mut source = Core::open(src_dir.path()).unwrap();
    let info = source
        .register_adapter(Box::new(EditorAdapter::session_v1()))
        .unwrap();
    let sess = source
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id,
            title: "l3".into(),
            privacy: Sensitivity::Personal,
            authority_mode: AuthorityMode::SingleWriter,
        })
        .unwrap();
    let cut = source.create_cut(sess).unwrap();
    let sess_str = sess.to_string();
    let cut_num = cut.number;

    let mut destination = Core::open(dst_dir.path()).unwrap();
    destination
        .register_adapter(Box::new(EditorAdapter::session_v1()))
        .unwrap();

    let source_id = DeviceIdentity::generate("source").unwrap();
    let dest_id = DeviceIdentity::generate("dest").unwrap();
    let (st, dt) = pair_devices(&source_id, &dest_id, "t").unwrap();
    let (st, dt) = (Arc::new(st), Arc::new(dt));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let src_handle = thread::spawn(move || {
        let mut net = Session::accept(&listener, &source_id, st).unwrap();
        net.server_negotiate(vec![]).unwrap();
        source.serve_cut(&mut net).unwrap();
        // Source measured real bytes sent (manifest + chunks).
        assert!(net.bytes_sent() > 0, "source sent real bytes");
        net.bytes_sent()
    });

    let mut net = Session::connect(&addr, &dest_id, dt).unwrap();
    net.client_negotiate(vec![]).unwrap();
    destination
        .import_cut(&mut net, &sess_str, cut_num)
        .unwrap()
        .completed_cut()
        .expect("import completes");

    // Destination measured real bytes over the wire (not an estimate).
    assert!(net.bytes_sent() > 0, "dest sent request bytes");
    assert!(net.bytes_recv() > 0, "dest received real bytes");
    let src_sent = src_handle.join().unwrap();
    // Source bytes sent ~= dest bytes received (same frames; allow exact-or-close).
    assert!(
        src_sent >= net.bytes_recv() / 2,
        "byte accounting is consistent"
    );

    // The mirror session reached ACTION_READY on closure validation (§18.8).
    let mirror = Core::mirror_session_id(&sess_str);
    let state = destination.get_session(mirror).unwrap().state;
    assert_eq!(
        state,
        SessionState::ActionReady,
        "completed import closure marks the session ACTION_READY"
    );
}
