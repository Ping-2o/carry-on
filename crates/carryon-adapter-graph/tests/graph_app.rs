//! Graph application test (§26.5.1): snapshot → cut → publish, then run
//! shortest-path with the oracle agreeing. Objects live in the content-addressed
//! store independent of the adapter (source-off evidence).

use carryon_adapter_graph::GraphAdapter;
use carryon_core::ids::CutId;
use carryon_core::model::AuthorityMode;
use carryon_core::model::{ActionRequest, Sensitivity};
use carryon_core::{Core, CreateSessionReq};
use tempfile::tempdir;

fn open_with_graph() -> (tempfile::TempDir, Core, CutId) {
    let dir = tempdir().unwrap();
    let mut core = Core::open(dir.path()).unwrap();
    let info = core
        .register_adapter(Box::new(GraphAdapter::sample()))
        .unwrap();

    let session = core
        .create_session(CreateSessionReq {
            adapter_id: info.adapter_id.clone(),
            title: "graph".into(),
            privacy: Sensitivity::Public,
            authority_mode: AuthorityMode::ReadOnlyReplica,
        })
        .unwrap();
    let cut = core.create_cut(session).unwrap();
    (dir, core, cut)
}

#[test]
fn seals_cut_and_publishes_authoritative_objects() {
    let (_dir, core, cut) = open_with_graph();
    // Both authoritative objects are present in the store (source-off: the store,
    // not the adapter, holds them).
    let missing = core.list_available_actions(cut, &["graph.shortest_path".into()]);
    assert!(
        missing[0].ready,
        "all authoritative objects must be present after the cut"
    );
    assert!(missing[0].missing.is_empty());
}

#[test]
fn shortest_path_runs_and_oracle_agrees() {
    let (_dir, mut core, cut) = open_with_graph();
    let req = ActionRequest {
        class: "graph.shortest_path".into(),
        params: serde_json::json!({ "start": 0, "end": 4 }),
    };
    let result = core.execute_action(cut, req).unwrap();
    assert!(result.oracle.checked);
    assert!(result.oracle.agreed, "Dijkstra and Bellman-Ford must agree");
    // 0→2(9)→3(11)→4(6) = 26 is the known shortest path cost in the sample graph.
    assert_eq!(result.output.get("cost").and_then(|v| v.as_u64()), Some(26));
}

#[test]
fn unsupported_action_is_refused() {
    let (_dir, mut core, cut) = open_with_graph();
    let req = ActionRequest {
        class: "graph.pagerank".into(),
        params: serde_json::json!({}),
    };
    let err = core.execute_action(cut, req).unwrap_err();
    assert_eq!(err.family(), "ACTION");
}
