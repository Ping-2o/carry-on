//! Graph reference adapter (L3, spec §10.7/§26.5.1).
//!
//! State is a weighted directed graph. Authoritative objects `graph.nodes.v1`
//! and `graph.edges.v1` carry the nodes and edges; a Derived `graph.adjacency_index.v1`
//! is a reproducible CSR cache. The action `graph.shortest_path(start,end,...)`
//! is checked by a dual-algorithm oracle (Dijkstra vs Bellman-Ford). This is a
//! demonstration of the engine, not proof of arbitrary-app support.

use carryon_adapter_api::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BinaryHeap;

/// A node with integer coordinates and a counter (counter feeds the block-sum view).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub id: u32,
    pub x: i64,
    pub y: i64,
    pub counter: u64,
}

/// A directed weighted edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: u32,
    pub to: u32,
    pub weight: u64,
}

/// The graph adapter.
pub struct GraphAdapter {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    generation: u64,
    consent: Option<String>,
}

const SCHEMA_NODES: &str = "graph.nodes.v1";
const SCHEMA_EDGES: &str = "graph.edges.v1";
const SCHEMA_ADJ: &str = "graph.adjacency_index.v1";
const ADAPTER_ID: &str = "org.carryon.graph";

impl GraphAdapter {
    /// Build with an explicit node/edge set.
    pub fn new(nodes: Vec<Node>, edges: Vec<Edge>) -> Self {
        GraphAdapter {
            nodes,
            edges,
            generation: 1,
            consent: None,
        }
    }

    /// A small deterministic sample graph for tests/demos.
    pub fn sample() -> Self {
        let nodes = (0..6)
            .map(|i| Node {
                id: i,
                x: i as i64,
                y: (i * i) as i64,
                counter: i as u64,
            })
            .collect();
        let edges = vec![
            Edge {
                from: 0,
                to: 1,
                weight: 7,
            },
            Edge {
                from: 0,
                to: 2,
                weight: 9,
            },
            Edge {
                from: 0,
                to: 5,
                weight: 14,
            },
            Edge {
                from: 1,
                to: 2,
                weight: 10,
            },
            Edge {
                from: 1,
                to: 3,
                weight: 15,
            },
            Edge {
                from: 2,
                to: 3,
                weight: 11,
            },
            Edge {
                from: 2,
                to: 5,
                weight: 2,
            },
            Edge {
                from: 3,
                to: 4,
                weight: 6,
            },
            Edge {
                from: 4,
                to: 5,
                weight: 9,
            },
        ];
        GraphAdapter::new(nodes, edges)
    }

    /// Per-node counters as a `(Z/kZ)^n` vector, for the block-sum view.
    pub fn counters(&self) -> Vec<u64> {
        self.nodes.iter().map(|n| n.counter).collect()
    }

    fn nodes_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.nodes).expect("serialize nodes")
    }
    fn edges_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.edges).expect("serialize edges")
    }

    fn object_bytes(&self, object_id: &str) -> Option<Vec<u8>> {
        match object_id {
            SCHEMA_NODES => Some(self.nodes_bytes()),
            SCHEMA_EDGES => Some(self.edges_bytes()),
            _ => None,
        }
    }

    fn entry(&self, object_id: &str, kind: ObjectKindWire, schema: &str) -> ObjectEntry {
        let bytes = self.object_bytes(object_id).unwrap_or_default();
        ObjectEntry {
            object_id: object_id.to_string(),
            generation: self.generation,
            kind,
            schema_id: schema.to_string(),
            content_hash: hex_sha256(&bytes),
            logical_size: bytes.len() as u64,
            parents: vec![],
            recipe_id: None,
            portable: true,
            sensitivity: SensitivityWire::Public,
            retention: RetentionWire::Session,
        }
    }

    /// Dijkstra shortest path cost from `start` to `end`.
    fn dijkstra(&self, start: u32, end: u32) -> Option<u64> {
        let n = self.nodes.len();
        let idx = |id: u32| id as usize;
        let mut adj: Vec<Vec<(usize, u64)>> = vec![Vec::new(); n];
        for e in &self.edges {
            if (e.from as usize) < n && (e.to as usize) < n {
                adj[idx(e.from)].push((idx(e.to), e.weight));
            }
        }
        let mut dist = vec![u64::MAX; n];
        dist[idx(start)] = 0;
        // min-heap via Reverse on (dist, node)
        let mut heap = BinaryHeap::new();
        heap.push(std::cmp::Reverse((0u64, idx(start))));
        while let Some(std::cmp::Reverse((d, u))) = heap.pop() {
            if d > dist[u] {
                continue;
            }
            for &(v, w) in &adj[u] {
                let nd = d.saturating_add(w);
                if nd < dist[v] {
                    dist[v] = nd;
                    heap.push(std::cmp::Reverse((nd, v)));
                }
            }
        }
        let c = dist[idx(end)];
        if c == u64::MAX {
            None
        } else {
            Some(c)
        }
    }

    /// Bellman-Ford shortest path cost — the independent oracle.
    fn bellman_ford(&self, start: u32, end: u32) -> Option<u64> {
        let n = self.nodes.len();
        let idx = |id: u32| id as usize;
        let mut dist = vec![u64::MAX; n];
        dist[idx(start)] = 0;
        for _ in 0..n.saturating_sub(1) {
            let mut changed = false;
            for e in &self.edges {
                let (u, v) = (idx(e.from), idx(e.to));
                if dist[u] != u64::MAX && dist[u].saturating_add(e.weight) < dist[v] {
                    dist[v] = dist[u] + e.weight;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let c = dist[idx(end)];
        if c == u64::MAX {
            None
        } else {
            Some(c)
        }
    }
}

impl Adapter for GraphAdapter {
    fn get_adapter_info(&self) -> AdapterInfo {
        AdapterInfo {
            adapter_id: ADAPTER_ID.into(),
            adapter_version: "1.0.0".into(),
            publisher_id: "org.carryon".into(),
            integration_level: IntegrationLevel::L3,
            executable: AdapterInfo::COMPILED_IN.into(),
            state_schemas: vec![SCHEMA_NODES.into(), SCHEMA_EDGES.into(), SCHEMA_ADJ.into()],
            actions: vec!["graph.shortest_path".into()],
            permissions: vec!["read_selected_workspace".into()],
            network_access: false,
            supports_snapshot: true,
            supports_mutations: false,
            supports_authority_transfer: false,
            max_object_bytes: 1024 * 1024,
        }
    }

    fn request_consent(&mut self, scope: ConsentScope) -> Result<ConsentToken, AdapterError> {
        let token = format!("consent-{}", scope.target);
        self.consent = Some(token.clone());
        Ok(ConsentToken(token))
    }

    fn list_sessions(&self, _c: &ConsentToken) -> Result<Vec<SessionSummary>, AdapterError> {
        Ok(vec![SessionSummary {
            session: "graph-session".into(),
            title: "Sample graph".into(),
            generation: self.generation,
            schema_version: 1,
        }])
    }

    fn begin_snapshot(
        &mut self,
        _session: &str,
        expected_generation: u64,
    ) -> Result<SnapshotToken, AdapterError> {
        if expected_generation != self.generation {
            return Err(AdapterError::StaleGeneration {
                expected: expected_generation,
                actual: self.generation,
            });
        }
        Ok(SnapshotToken(format!("snap-{}", self.generation)))
    }

    fn describe_snapshot(&self, _t: &SnapshotToken) -> Result<ObjectManifest, AdapterError> {
        Ok(ObjectManifest {
            session: "graph-session".into(),
            generation: self.generation,
            objects: vec![
                self.entry(SCHEMA_NODES, ObjectKindWire::Authoritative, SCHEMA_NODES),
                self.entry(SCHEMA_EDGES, ObjectKindWire::Authoritative, SCHEMA_EDGES),
            ],
        })
    }

    fn read_object(
        &self,
        _t: &SnapshotToken,
        object_id: &str,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, AdapterError> {
        let bytes = self
            .object_bytes(object_id)
            .ok_or_else(|| AdapterError::UnknownObject(object_id.into()))?;
        let start = (offset as usize).min(bytes.len());
        let end = (start + length as usize).min(bytes.len());
        Ok(bytes[start..end].to_vec())
    }

    fn finish_snapshot(&mut self, _t: SnapshotToken) -> Result<SnapshotReceipt, AdapterError> {
        Ok(SnapshotReceipt {
            session: "graph-session".into(),
            generation: self.generation,
            manifest_digest: hex_sha256(&self.nodes_bytes()),
        })
    }

    fn abort_snapshot(&mut self, _t: SnapshotToken, _reason: &str) {}

    fn current_generation(&self, _session: &str) -> Result<u64, AdapterError> {
        Ok(self.generation)
    }

    fn resolve_action(
        &self,
        cut: &CutRef,
        req: &ActionRequestWire,
    ) -> Result<DependencyPlanWire, AdapterError> {
        if req.class != "graph.shortest_path" {
            return Err(AdapterError::ActionUnsupported(req.class.clone()));
        }
        let _ = cut;
        Ok(DependencyPlanWire {
            prerequisites: vec![
                ObjectVersionWire {
                    object_id: SCHEMA_NODES.into(),
                    generation: self.generation,
                },
                ObjectVersionWire {
                    object_id: SCHEMA_EDGES.into(),
                    generation: self.generation,
                },
            ],
            provenance: vec![],
            optional: vec![ObjectVersionWire {
                object_id: SCHEMA_ADJ.into(),
                generation: self.generation,
            }],
        })
    }

    fn validate_objects(
        &self,
        _cut: &CutRef,
        versions: &[ObjectVersionWire],
    ) -> Result<ValidationReport, AdapterError> {
        let missing: Vec<_> = versions
            .iter()
            .filter(|v| v.object_id != SCHEMA_NODES && v.object_id != SCHEMA_EDGES)
            .cloned()
            .collect();
        Ok(ValidationReport {
            ok: missing.is_empty(),
            missing,
            message: "graph validation".into(),
        })
    }

    fn import_objects(
        &mut self,
        _cut: &CutRef,
        locations: &[ObjectLocation],
    ) -> Result<ImportReceipt, AdapterError> {
        Ok(ImportReceipt {
            imported: locations
                .iter()
                .map(|l| ObjectVersionWire {
                    object_id: l.object_id.clone(),
                    generation: l.generation,
                })
                .collect(),
        })
    }

    fn activate(
        &mut self,
        _cut: &CutRef,
        _req: &ActionRequestWire,
    ) -> Result<ActivationReceipt, AdapterError> {
        Ok(ActivationReceipt {
            activated: true,
            detail: "graph ready".into(),
        })
    }

    fn execute_action(
        &mut self,
        _cut: &CutRef,
        req: &ActionRequestWire,
    ) -> Result<ActionResultWire, AdapterError> {
        if req.class != "graph.shortest_path" {
            return Err(AdapterError::ActionUnsupported(req.class.clone()));
        }
        let start = req
            .params
            .get("start")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let end = req.params.get("end").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

        let primary = self.dijkstra(start, end);
        let oracle = self.bellman_ford(start, end);
        let agreed = primary == oracle;

        let output = serde_json::json!({ "start": start, "end": end, "cost": primary });
        let output_hash = hex_sha256(&serde_json::to_vec(&output).unwrap_or_default());
        Ok(ActionResultWire {
            output,
            output_hash,
            oracle: OracleOutcome {
                checked: true,
                agreed,
                output_hash: hex_sha256(format!("{oracle:?}").as_bytes()),
                detail: format!("dijkstra={primary:?} bellman_ford={oracle:?}"),
            },
        })
    }

    fn export_evidence(
        &self,
        session: &str,
        _range: EvidenceRange,
    ) -> Result<EvidenceFragment, AdapterError> {
        Ok(EvidenceFragment {
            session: session.into(),
            json: serde_json::json!({ "nodes": self.nodes.len(), "edges": self.edges.len() }),
        })
    }
}

/// Lowercase hex sha256.
fn hex_sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut s = String::with_capacity(64);
    for b in out {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
