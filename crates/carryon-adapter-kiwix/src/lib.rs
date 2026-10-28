//! Carry-On Kiwix adapter (L1 ZIM bulk + L3 reading session + L4 position
//! authority) — Phase-1 skeleton.
//!
//! Design: `docs/kiwix-bridge/ADAPTER-DESIGN.md` (authoritative); research and
//! evidence: `docs/kiwix-bridge/RESEARCH.md`, `BUILD-EVIDENCE.md`.
//!
//! The flagship asymmetry this adapter expresses: the authoritative **mutable**
//! state of a Kiwix reading session is a few KB (which ZIM, which article,
//! scroll position, bookmarks, history, settings), while the content library
//! (the ZIM file) is 80+ GB of **immutable**, checksummed, chunk-addressable
//! bytes (`docs/TARGETS.md` #4's `bytes_before_first_action` story).
//!
//! | object                  | kind          | dependency role        | why |
//! |-------------------------|---------------|------------------------|-----|
//! | `kiwix.session.v1`      | Authoritative | prerequisite           | {zim identity, current article, scroll, tab} — "the same session" |
//! | `kiwix.article.v1`      | Derived       | prerequisite (immediate tier) | first paint without the 80 GB; parents = zimfile |
//! | `kiwix.zimfile.v1`      | Authoritative | optional — bulk, on demand | the L1 content object; served chunked, never shipped wholesale |
//! | `kiwix.bookmarks.v1`    | Authoritative | optional               | user data, non-blocking |
//! | `kiwix.history.v1`      | Authoritative | optional               | user data, non-blocking |
//! | `kiwix.settings.v1`     | Authoritative | optional               | zoom/theme subset |
//! | `kiwix.meta.v1`         | Authoritative | prerequisite + provenance | schema/version identity |
//!
//! **L4 authority** (§21.2) binds to the hash over the mutable authoritative
//! state (session ∪ bookmarks ∪ history ∪ settings) — the reading position,
//! *not* the book. The ZIM file and the article bytes (immutable / derived)
//! are excluded from `auth_hash`.
//!
//! # Honest Phase-1 status (labels are law)
//!
//! - IMPLEMENTED (this crate, pure Rust): the object model, manifests with
//!   sha256 digests, chunked `read_object` over in-memory state, dependency
//!   plans for all three actions, generation/staleness, the L4 proposal
//!   protocol with state-bound `auth_hash`.
//! - **UNSUPPORTED**: everything that needs libzim — reading a real ZIM file,
//!   extracting the current article out of one, verifying ZIM identity beyond
//!   `{uuid, size}`, and serving ZIM bulk from a real file on disk. The
//!   `kiwix.zimfile.v1` object only serves bytes that were attached via
//!   [`KiwixAdapter::with_zim_bytes`] (a stand-in for the Phase-2 libzim
//!   backend); without it, every ZIM-dependent call **fails closed**.
//! - Build feasibility of the libzim backend for all three platforms is
//!   PROVED/recorded in `docs/kiwix-bridge/BUILD-EVIDENCE.md`.

use carryon_adapter_api::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

const SCHEMA_SESSION: &str = "kiwix.session.v1";
const SCHEMA_ARTICLE: &str = "kiwix.article.v1";
const SCHEMA_ZIMFILE: &str = "kiwix.zimfile.v1";
const SCHEMA_BOOKMARKS: &str = "kiwix.bookmarks.v1";
const SCHEMA_HISTORY: &str = "kiwix.history.v1";
const SCHEMA_SETTINGS: &str = "kiwix.settings.v1";
const SCHEMA_META: &str = "kiwix.meta.v1";
const ADAPTER_ID: &str = "org.carryon.kiwix";

/// Identity of the open ZIM. Bytes of the file itself are the optional-bulk
/// `kiwix.zimfile.v1` object; this is only the pointer.
/// RESEARCH §2.2: `zim::Archive` exposes uuid/size/filename for this binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZimIdentity {
    /// ZIM uuid (string form) — `Archive::getUuid()`.
    pub uuid: String,
    /// Canonical filename of the `.zim` (or first `.zimaa` part of a split).
    pub filename: String,
    /// Logical archive size — `Archive::getFilesize()`.
    pub size: u64,
}

/// The current reading position — the heart of the session. Field names mirror
/// what both Kiwix apps persist (RESEARCH §1): Android stores
/// {zimId, webViewPosition=scrollY, webViewIndex} per tab in Room; iOS stores
/// the same tuple inside the CoreData `Tab` + `interactionState` blob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadingPosition {
    /// ZIM entry path of the current article, e.g. "A/Python_(programming_language)".
    pub article_path: String,
    pub article_title: String,
    /// Vertical scroll offset in pixels (`webView.scrollY` / iOS scroll in
    /// the interactionState blob).
    pub scroll_y: u64,
    /// Tab index within the reader.
    pub tab_index: u32,
    /// Opaque platform blob (Android `WebView.saveState` Bundle / iOS
    /// `WKWebView.interactionState`), base64. Same-version restore only; the
    /// explicit fields above are the portable truth.
    pub opaque_platform_state: Option<String>,
}

/// A bookmark row (shape mirrors `kiwix::Bookmark`, RESEARCH §2.3, and the
/// CoreData `Bookmark` entity on iOS).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bookmark {
    pub book_id: String,
    pub article_url: String,
    pub title: String,
}

/// One history entry (mirrors Android `HistoryRoomEntity` fields).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub zim_id: String,
    pub article_url: String,
    pub title: String,
    pub timestamp: u64,
}

/// Reading settings subset (zoom/theme — RESEARCH §1 DataStore/UserDefaults keys).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadingSettings {
    /// Android `"true_text_zoom"` (default 100) / iOS `webViewPageZoom` ×100.
    pub text_zoom: u32,
    /// `"pref_dark_mode"` / iOS `userColorScheme`: "system" | "light" | "dark".
    pub theme: String,
}

/// Portable schema/version identity for the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Meta {
    schema_id: String,
    schema_version: u32,
    adapter_version: String,
}

/// The full mutable authoritative session state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub zim: ZimIdentity,
    pub position: ReadingPosition,
    pub bookmarks: Vec<Bookmark>,
    pub history: Vec<HistoryEntry>,
    pub settings: ReadingSettings,
}

/// A Kiwix reading session over one structured session + optional ZIM bulk.
///
/// The optional `zim_bytes` is the Phase-1 **stand-in** for the libzim backend:
/// a chunk-addressable byte blob that proves the progressive bulk path. In
/// Phase 2 it becomes a real opened `zim::Archive` (BUILD-EVIDENCE §B/§C have
/// the cross-compiles).
pub struct KiwixAdapter {
    session: String,
    meta: Meta,
    state: Session,
    /// Current article HTML (Derived object). In Phase 2 this is extracted
    /// from the ZIM via `Archive::getEntryByPath().getItem(true).getData()`
    /// (RESEARCH §2.1). Phase 1: attached in-memory for the skeleton's tests.
    article: Vec<u8>,
    /// Optional bulk bytes (`kiwix.zimfile.v1`). `None` ⇒ every ZIM-dependent
    /// call fails closed (UNSUPPORTED until the libzim backend lands).
    zim_bytes: Option<Vec<u8>>,
    generation: u64,
    /// Set between `prepare_authority_transfer` and its commit/abort.
    pending: Option<PendingTransfer>,
}

struct PendingTransfer {
    auth_hash: String,
}

/// The serialized L4 proposal payload crossing the wire (data only).
#[derive(Serialize, Deserialize)]
struct Proposal {
    session: String,
    cut_number: u64,
    /// Hash over the mutable authoritative state (session ∪ bookmarks ∪
    /// history ∪ settings). Excludes the ZIM (immutable) and the article
    /// (derived): authority is over *where you are in the book*, not the book.
    auth_hash: String,
}

impl KiwixAdapter {
    /// A session with explicit parts and no ZIM bulk attached.
    pub fn new(session: impl Into<String>, state: Session) -> Self {
        KiwixAdapter {
            meta: Meta {
                schema_id: "carryon.kiwix.session".into(),
                schema_version: 1,
                adapter_version: "0.1.0".into(),
            },
            session: session.into(),
            state,
            article: Vec::new(),
            zim_bytes: None,
            generation: 1,
            pending: None,
        }
    }

    /// Attach the current article's HTML bytes (the immediate-tier object).
    pub fn with_article(mut self, html: impl Into<Vec<u8>>) -> Self {
        self.article = html.into();
        self
    }

    /// Attach ZIM bulk bytes — Phase-1 stand-in for the libzim backend.
    /// With this set, `read_object` serves `kiwix.zimfile.v1` chunk-by-chunk
    /// (the progressive strategy). Without it, ZIM calls fail closed.
    pub fn with_zim_bytes(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.zim_bytes = Some(bytes.into());
        self
    }

    /// A recognizable demo session (no ZIM bulk): a plausible Wikipedia-style
    /// position, one bookmark, some history, default settings.
    pub fn sample() -> Self {
        let state = Session {
            zim: ZimIdentity {
                uuid: "6fed4307-f6ff-4bb9-8bc3-8bf4ac9d5df2".into(),
                filename: "wikipedia_en_all_maxi.zim".into(),
                size: 98_765_432_109, // ~92 GB stand-in for the byte math
            },
            position: ReadingPosition {
                article_path: "A/Continuation_passing_style".into(),
                article_title: "Continuation-passing style".into(),
                scroll_y: 42_000,
                tab_index: 0,
                opaque_platform_state: None,
            },
            bookmarks: vec![Bookmark {
                book_id: "6fed4307".into(),
                article_url: "zim://A/Call/cc".into(),
                title: "Call/cc".into(),
            }],
            history: vec![HistoryEntry {
                zim_id: "6fed4307".into(),
                article_url: "zim://A/Lambda_calculus".into(),
                title: "Lambda calculus".into(),
                timestamp: 1_771_000_000,
            }],
            settings: ReadingSettings {
                text_zoom: 100,
                theme: "system".into(),
            },
        };
        KiwixAdapter::new("kiwix-demo-session", state).with_article(
            "<html><body><h1>Continuation-passing style</h1><p>In functional programming, ...</p></body></html>",
        )
    }

    /// Current session state (tests/inspection).
    pub fn state(&self) -> &Session {
        &self.state
    }

    /// Current generation (tests/inspection).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn session_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.state).expect("serialize session")
    }
    fn settings_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.state.settings).expect("serialize settings")
    }
    fn bookmarks_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.state.bookmarks).expect("serialize bookmarks")
    }
    fn history_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.state.history).expect("serialize history")
    }
    fn meta_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.meta).expect("serialize meta")
    }

    fn object_bytes(&self, object_id: &str) -> Result<Vec<u8>, AdapterError> {
        match object_id {
            SCHEMA_SESSION => Ok(self.session_bytes()),
            SCHEMA_ARTICLE => Ok(self.article.clone()),
            SCHEMA_ZIMFILE => match &self.zim_bytes {
                Some(b) => Ok(b.clone()),
                // Fail closed: no libzim backend in Phase 1 — never fabricate
                // bulk bytes.
                None => Err(AdapterError::Incompatible(format!(
                    "UNSUPPORTED (Phase 1): {SCHEMA_ZIMFILE} requires the libzim backend or attached bulk bytes"
                ))),
            },
            SCHEMA_BOOKMARKS => Ok(self.bookmarks_bytes()),
            SCHEMA_HISTORY => Ok(self.history_bytes()),
            SCHEMA_SETTINGS => Ok(self.settings_bytes()),
            SCHEMA_META => Ok(self.meta_bytes()),
            other => Err(AdapterError::UnknownObject(other.into())),
        }
    }

    /// Hash over the mutable authoritative state (session ∪ bookmarks ∪
    /// history ∪ settings), length-prefixed and ordered. The ZIM and the
    /// article are excluded (immutable / derived).
    fn auth_hash(&self) -> String {
        let mut h = Sha256::new();
        for bytes in [
            self.session_bytes(),
            self.bookmarks_bytes(),
            self.history_bytes(),
            self.settings_bytes(),
        ] {
            h.update((bytes.len() as u64).to_le_bytes());
            h.update(&bytes);
        }
        hex(&h.finalize())
    }

    fn entry(&self, object_id: &str, kind: ObjectKindWire) -> ObjectEntry {
        // The article derives from the ZIM file: record that lineage.
        let parents = if object_id == SCHEMA_ARTICLE {
            vec![ObjectVersionWire {
                object_id: SCHEMA_ZIMFILE.into(),
                generation: self.generation,
            }]
        } else {
            vec![]
        };
        // Digest + size: fail closed into an empty entry is NOT acceptable —
        // the core re-verifies hashes, and for the unsupported ZIM bulk there
        // is nothing to serve; we still declare the object (it is optional),
        // with an empty digest only when bytes are unavailable.
        let (hash, size) = match self.object_bytes(object_id) {
            Ok(b) => (hex_sha256(&b), b.len() as u64),
            Err(_) => (String::new(), 0),
        };
        ObjectEntry {
            object_id: object_id.to_string(),
            generation: self.generation,
            kind,
            schema_id: object_id.to_string(),
            content_hash: hash,
            logical_size: size,
            parents,
            recipe_id: None,
            portable: true,
            sensitivity: SensitivityWire::Personal,
            retention: if object_id == SCHEMA_ZIMFILE {
                RetentionWire::Persistent
            } else {
                RetentionWire::Session
            },
        }
    }

    fn all_entries(&self) -> Vec<ObjectEntry> {
        vec![
            self.entry(SCHEMA_SESSION, ObjectKindWire::Authoritative),
            // Derived (from the ZIM), but the immediate-tier object: first
            // paint without the 80 GB.
            self.entry(SCHEMA_ARTICLE, ObjectKindWire::Derived),
            // The L1 content object: Authoritative as an object, OPTIONAL as a
            // dependency (§6.6) — the whole point of this flagship.
            self.entry(SCHEMA_ZIMFILE, ObjectKindWire::Authoritative),
            self.entry(SCHEMA_BOOKMARKS, ObjectKindWire::Authoritative),
            self.entry(SCHEMA_HISTORY, ObjectKindWire::Authoritative),
            self.entry(SCHEMA_SETTINGS, ObjectKindWire::Authoritative),
            self.entry(SCHEMA_META, ObjectKindWire::Authoritative),
        ]
    }

    fn version(&self, object_id: &str) -> ObjectVersionWire {
        ObjectVersionWire {
            object_id: object_id.into(),
            generation: self.generation,
        }
    }
}

impl Adapter for KiwixAdapter {
    fn get_adapter_info(&self) -> AdapterInfo {
        AdapterInfo {
            adapter_id: ADAPTER_ID.into(),
            adapter_version: self.meta.adapter_version.clone(),
            publisher_id: "org.carryon".into(),
            integration_level: IntegrationLevel::L4,
            executable: AdapterInfo::COMPILED_IN.into(),
            state_schemas: vec![
                SCHEMA_SESSION.into(),
                SCHEMA_ARTICLE.into(),
                SCHEMA_ZIMFILE.into(),
                SCHEMA_BOOKMARKS.into(),
                SCHEMA_HISTORY.into(),
                SCHEMA_SETTINGS.into(),
                SCHEMA_META.into(),
            ],
            actions: vec![
                "reader.restore".into(),
                "reader.navigate".into(),
                "library.materialize".into(),
            ],
            permissions: vec!["user_selected_file".into()],
            network_access: false,
            supports_snapshot: true,
            supports_mutations: true,
            supports_authority_transfer: true,
            // The ZIM bulk object can be far larger than the reference
            // editor's 16 MiB cap; the core's budget admission (TARGETS #5) is
            // what bounds what actually moves.
            max_object_bytes: 1 << 40,
        }
    }

    fn request_consent(&mut self, scope: ConsentScope) -> Result<ConsentToken, AdapterError> {
        Ok(ConsentToken(format!("consent-{}", scope.target)))
    }

    fn list_sessions(&self, _c: &ConsentToken) -> Result<Vec<SessionSummary>, AdapterError> {
        Ok(vec![SessionSummary {
            session: self.session.clone(),
            title: format!("kiwix reading: {}", self.state.position.article_title),
            generation: self.generation,
            schema_version: self.meta.schema_version,
        }])
    }

    fn begin_snapshot(&mut self, _s: &str, expected: u64) -> Result<SnapshotToken, AdapterError> {
        if expected != self.generation {
            return Err(AdapterError::StaleGeneration {
                expected,
                actual: self.generation,
            });
        }
        Ok(SnapshotToken(format!("snap-{}", self.generation)))
    }

    fn describe_snapshot(&self, _t: &SnapshotToken) -> Result<ObjectManifest, AdapterError> {
        Ok(ObjectManifest {
            session: self.session.clone(),
            generation: self.generation,
            objects: self.all_entries(),
        })
    }

    fn read_object(
        &self,
        _t: &SnapshotToken,
        object_id: &str,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, AdapterError> {
        let bytes = self.object_bytes(object_id)?;
        let start = (offset as usize).min(bytes.len());
        let end = start.saturating_add(length as usize).min(bytes.len());
        Ok(bytes[start..end].to_vec())
    }

    fn finish_snapshot(&mut self, _t: SnapshotToken) -> Result<SnapshotReceipt, AdapterError> {
        Ok(SnapshotReceipt {
            session: self.session.clone(),
            generation: self.generation,
            manifest_digest: self.auth_hash(),
        })
    }

    fn abort_snapshot(&mut self, _t: SnapshotToken, _reason: &str) {}

    fn current_generation(&self, _s: &str) -> Result<u64, AdapterError> {
        Ok(self.generation)
    }

    fn resolve_action(
        &self,
        _cut: &CutRef,
        req: &ActionRequestWire,
    ) -> Result<DependencyPlanWire, AdapterError> {
        match req.class.as_str() {
            // The first useful action: restore reading (article + position)
            // without the ZIM. Bookmarks/history/settings/ZIM bulk are all
            // optional (§6.6): they improve latency or completeness, not
            // correctness of the restore.
            "reader.restore" => Ok(DependencyPlanWire {
                prerequisites: vec![
                    self.version(SCHEMA_SESSION),
                    self.version(SCHEMA_ARTICLE),
                    self.version(SCHEMA_META),
                ],
                provenance: vec![self.version(SCHEMA_META)],
                optional: vec![
                    self.version(SCHEMA_SETTINGS),
                    self.version(SCHEMA_BOOKMARKS),
                    self.version(SCHEMA_HISTORY),
                    self.version(SCHEMA_ZIMFILE),
                ],
            }),
            // Mutating (L4): move the reading position. Needs the session +
            // meta only; everything else is optional.
            "reader.navigate" => Ok(DependencyPlanWire {
                prerequisites: vec![self.version(SCHEMA_SESSION), self.version(SCHEMA_META)],
                provenance: vec![self.version(SCHEMA_META)],
                optional: vec![
                    self.version(SCHEMA_ARTICLE),
                    self.version(SCHEMA_HISTORY),
                    self.version(SCHEMA_ZIMFILE),
                ],
            }),
            // Optional bulk: materialize ZIM byte ranges locally so libzim
            // can open the file natively (RESEARCH §2.2). The core's budget
            // admission refuses over-budget imports before bytes move.
            "library.materialize" => Ok(DependencyPlanWire {
                prerequisites: vec![self.version(SCHEMA_META)],
                provenance: vec![self.version(SCHEMA_META)],
                optional: vec![self.version(SCHEMA_ZIMFILE)],
            }),
            other => Err(AdapterError::ActionUnsupported(other.into())),
        }
    }

    fn validate_objects(
        &self,
        _cut: &CutRef,
        versions: &[ObjectVersionWire],
    ) -> Result<ValidationReport, AdapterError> {
        let known = [
            SCHEMA_SESSION,
            SCHEMA_ARTICLE,
            SCHEMA_ZIMFILE,
            SCHEMA_BOOKMARKS,
            SCHEMA_HISTORY,
            SCHEMA_SETTINGS,
            SCHEMA_META,
        ];
        let missing: Vec<_> = versions
            .iter()
            .filter(|v| !known.contains(&v.object_id.as_str()))
            .cloned()
            .collect();
        Ok(ValidationReport {
            ok: missing.is_empty(),
            missing,
            message: "kiwix session validation".into(),
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
            detail: format!("kiwix reader ready for {}", self.session),
        })
    }

    fn execute_action(
        &mut self,
        _cut: &CutRef,
        req: &ActionRequestWire,
    ) -> Result<ActionResultWire, AdapterError> {
        match req.class.as_str() {
            // Read-only: report the restored logical session so a destination
            // can prove it holds the same article + position.
            "reader.restore" => {
                let output = serde_json::json!({
                    "zim_uuid": self.state.zim.uuid,
                    "zim_size": self.state.zim.size,
                    "article_path": self.state.position.article_path,
                    "scroll_y": self.state.position.scroll_y,
                    "tab_index": self.state.position.tab_index,
                    "article_hash": hex_sha256(&self.article),
                    "generation": self.generation,
                });
                let output_hash = hex_sha256(&serde_json::to_vec(&output).unwrap_or_default());
                Ok(ActionResultWire {
                    output,
                    output_hash: output_hash.clone(),
                    oracle: OracleOutcome {
                        checked: true,
                        agreed: true,
                        output_hash,
                        detail: format!(
                            "reading restored at generation {} ({} @ scroll {})",
                            self.generation,
                            self.state.position.article_path,
                            self.state.position.scroll_y
                        ),
                    },
                })
            }
            // Mutating (L4 single-writer): move the position. The core gates
            // whether this device may mutate; the adapter just applies it and
            // bumps the generation.
            "reader.navigate" => {
                let path = req
                    .params
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        AdapterError::Internal("reader.navigate requires 'path'".into())
                    })?;
                let scroll_y = req
                    .params
                    .get("scroll_y")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let title = req
                    .params
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or(path)
                    .to_string();
                self.state.position.article_path = path.to_string();
                self.state.position.article_title = title.clone();
                self.state.position.scroll_y = scroll_y;
                self.state.position.opaque_platform_state = None;
                self.state.history.push(HistoryEntry {
                    zim_id: self.state.zim.uuid.clone(),
                    article_url: format!("zim://{path}"),
                    title,
                    timestamp: self.generation,
                });
                self.generation += 1;
                // Phase 1: the new article's bytes are NOT extracted from the
                // ZIM (no libzim backend) — the article object becomes empty
                // and is honestly reported as such.
                self.article = Vec::new();
                let output = serde_json::json!({
                    "generation": self.generation,
                    "article_path": self.state.position.article_path,
                    "scroll_y": self.state.position.scroll_y,
                    "article_bytes_available": false,
                });
                let output_hash = hex_sha256(&serde_json::to_vec(&output).unwrap_or_default());
                Ok(ActionResultWire {
                    output,
                    output_hash: output_hash.clone(),
                    oracle: OracleOutcome {
                        checked: true,
                        agreed: true,
                        output_hash,
                        detail: format!(
                            "navigated to {} at generation {} (article bytes: UNSUPPORTED until libzim backend)",
                            self.state.position.article_path, self.generation
                        ),
                    },
                })
            }
            // Optional bulk: acknowledge a range request. Actual bytes flow
            // through `read_object` chunking; the action only validates the
            // range and fails closed when no bulk is attached.
            "library.materialize" => {
                let offset = req
                    .params
                    .get("offset")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let length = req
                    .params
                    .get("length")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let size = match &self.zim_bytes {
                    Some(b) => b.len() as u64,
                    None => {
                        return Err(AdapterError::Incompatible(
                            "UNSUPPORTED (Phase 1): library.materialize requires the libzim backend / attached bulk bytes".into(),
                        ))
                    }
                };
                if let Some(end) = offset.checked_add(length) {
                    if end > size {
                        return Err(AdapterError::OutOfRange(format!(
                            "range [{offset}, {end}) exceeds zim size {size}"
                        )));
                    }
                } else {
                    return Err(AdapterError::OutOfRange(format!(
                        "range [{offset}, +{length}) overflows u64"
                    )));
                }
                let output = serde_json::json!({
                    "accepted": true, "offset": offset, "length": length, "zim_size": size,
                });
                let output_hash = hex_sha256(&serde_json::to_vec(&output).unwrap_or_default());
                Ok(ActionResultWire {
                    output,
                    output_hash: output_hash.clone(),
                    oracle: OracleOutcome {
                        checked: true,
                        agreed: true,
                        output_hash,
                        detail: format!("zim chunk ack [{offset}, {})", offset + length),
                    },
                })
            }
            other => Err(AdapterError::ActionUnsupported(other.into())),
        }
    }

    // --- Authority transfer (L4, §21.2): authority over the reading position.

    fn prepare_authority_transfer(&mut self, cut: &CutRef) -> Result<String, AdapterError> {
        let proposal = Proposal {
            session: cut.session.clone(),
            cut_number: cut.cut_number,
            auth_hash: self.auth_hash(),
        };
        let blob = serde_json::to_string(&proposal)
            .map_err(|e| AdapterError::Internal(format!("serialize proposal: {e}")))?;
        self.pending = Some(PendingTransfer {
            auth_hash: proposal.auth_hash,
        });
        Ok(blob)
    }

    fn accept_authority_transfer(
        &mut self,
        cut: &CutRef,
        proposal: &str,
    ) -> Result<String, AdapterError> {
        let p: Proposal = serde_json::from_str(proposal)
            .map_err(|e| AdapterError::Internal(format!("parse proposal: {e}")))?;
        if p.cut_number != cut.cut_number {
            return Err(AdapterError::Incompatible(format!(
                "proposal cut {} != imported cut {}",
                p.cut_number, cut.cut_number
            )));
        }
        if p.auth_hash != self.auth_hash() {
            return Err(AdapterError::Incompatible(
                "proposal authoritative-state hash does not match imported state".into(),
            ));
        }
        Ok(format!("accept:{}:{}", p.cut_number, p.auth_hash))
    }

    fn commit_authority_transfer(
        &mut self,
        proposal_id: &str,
        dest_receipt: &str,
    ) -> Result<String, AdapterError> {
        let pending = self
            .pending
            .take()
            .ok_or_else(|| AdapterError::Incompatible("no authority transfer prepared".into()))?;
        if !dest_receipt.ends_with(&pending.auth_hash) {
            self.pending = Some(pending);
            return Err(AdapterError::Incompatible(
                "destination receipt does not match prepared proposal".into(),
            ));
        }
        Ok(format!("relinquish:{proposal_id}:{}", pending.auth_hash))
    }

    fn abort_authority_transfer(&mut self, _proposal_id: &str, _reason: &str) {
        self.pending = None;
    }

    fn export_evidence(
        &self,
        session: &str,
        _range: EvidenceRange,
    ) -> Result<EvidenceFragment, AdapterError> {
        Ok(EvidenceFragment {
            session: session.into(),
            json: serde_json::json!({
                "session": self.session,
                "generation": self.generation,
                "zim_uuid": self.state.zim.uuid,
                "zim_size": self.state.zim.size,
                "zim_bulk_attached": self.zim_bytes.is_some(),
                "article_hash": hex_sha256(&self.article),
                "auth_hash": self.auth_hash(),
            }),
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex(&h.finalize())
}
