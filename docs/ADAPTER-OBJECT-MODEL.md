# PhotoCraft and Kiwix adapter object models — reconciliation note

**Status: `TARGET`** (issue #28). Design note only: no shared types land with it,
and no label changes. Sources: `crates/carryon-adapter-api/src/lib.rs`,
`crates/carryon-adapter-photocraft/src/{lib,model}.rs`,
`crates/carryon-adapter-kiwix/src/lib.rs`,
[`artcraft-bridge/TILE-SPLITTING.md`](artcraft-bridge/TILE-SPLITTING.md),
[`kiwix-bridge/ADAPTER-DESIGN.md`](kiwix-bridge/ADAPTER-DESIGN.md).

## 1. The two models side by side

| Concern | PhotoCraft (`org.carryon.photocraft`) | Kiwix |
|---|---|---|
| What "the session" is | the **document** (pixels + layer tree) | the **reading position** in an immutable book |
| Root object | `photocraft.document.v1` (monolithic) or `photocraft.document.manifest.v1` (split) | `kiwix.session.v1` (ZIM identity + `ReadingPosition`) |
| Parts | `photocraft.tile.<blake3>.v1`, `photocraft.blob.<blake3>.v1`; parents = manifest | `kiwix.article.v1`; parents = zimfile |
| Mutable user state | `photocraft.unsaved_edits.v1` (journal or snapshot); parents = document | `kiwix.bookmarks.v1`, `kiwix.history.v1`, `kiwix.settings.v1` |
| Position | `photocraft.workspace.v1`, **Ephemeral**, optional (the selection lives in the document) | inside `kiwix.session.v1`, **Authoritative**, prerequisite |
| Bulk | none beyond the document closure | `kiwix.zimfile.v1`: Cache (progressive) / Authoritative (full up front) when ≤ 16 MiB; above the cap it is **not a cut object**, it is addressed by `{uuid, size, bulk_sha256}` in the session and materialized on demand |
| Identity / provenance | `photocraft.meta.v1`: prerequisite + provenance, records the PhotoCraft pin | `kiwix.meta.v1`: prerequisite + provenance |
| Actions | `document.edit`, `session.restore` | `reader.restore`, `reader.navigate` (L4), `library.materialize` |
| Prerequisites | document closure (manifest + every part) + unsaved + meta, in **admission-priority order** | session + article + meta (restore); session + meta (navigate); meta (materialize) |
| Optional | workspace | settings, bookmarks, history, zimfile (when declared), article (navigate) |
| Closure check | PhotoCraft's own loader refuses a bundle with a missing part (fail closed) | fixed object set; ZIM-dependent paths fail closed without the backend |
| L4 `auth_hash` binds | document logical bytes ‖ unsaved ‖ meta (length-prefixed) | session ∪ bookmarks ∪ history ∪ settings; excludes the article and the bulk |

## 2. What is genuinely shared

These are the same concept in both adapters, already expressed with existing API
types:

1. **Kind drives transfer; the plan drives readiness.** Only sealed Authoritative
   objects ride the wire cut. `DependencyPlanWire` decides what an action waits
   for, not what moves. Kiwix's progressive/full-up-front switch and PhotoCraft's
   Ephemeral workspace both rely on this rule. It should be written down once in
   the API crate docs instead of in each adapter.
2. **Prerequisite / optional / provenance tiers** (§6.6). Both use
   `DependencyPlanWire` the same way, and both put `meta` in prerequisites and
   provenance at once.
3. **Root + parent-linked parts.** PhotoCraft tiles → manifest and Kiwix article →
   zimfile are the same pattern: a part's `parents` name the object version it
   belongs to (§6.3).
4. **Admission priority = order.** PhotoCraft orders prerequisites small and
   critical first because progressive budget admission takes the longest prefix
   that fits (TILE-SPLITTING §5). Kiwix lists session/article/meta first, which
   happens to have the same effect. That is a convention today, not a contract.
5. **State-bound authority.** Both bind L4 authority to a SHA-256 over
   length-prefixed mutable state, never to a token (§21.2). Neither binds bulk or
   derived bytes.

## 3. What must stay different

- **The position's kind.** In Kiwix the position *is* the session (Authoritative,
  prerequisite). In PhotoCraft it is a latency hint (Ephemeral, optional).
  Unifying the kind would either make PhotoCraft restores fail on a missing
  viewport or let Kiwix restores succeed without the position. Share the
  vocabulary, not the kind.
- **Who walks the closure.** PhotoCraft deliberately delegates the closure check to
  PhotoCraft's loader (a second walk would be a weaker copy). Kiwix has a fixed
  set. A shared closure walker in `carryon-adapter-api` would be wrong for PhotoCraft.
- **External bulk.** Content outside the cut is Kiwix-only today; PhotoCraft has no
  equivalent. See [`LARGE-SESSIONS.md`](LARGE-SESSIONS.md) for the large-object question.

## 4. Candidate shared types (proposed signatures, not landed)

All additive to `carryon-adapter-api`. They change neither the `Adapter` trait nor
the C ABI:

```rust
/// The role an entry plays for an adapter's actions (§6.6). Descriptive only:
/// the core still reads roles from DependencyPlanWire, never from this tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyRole {
    Prerequisite,
    Optional,
    Provenance,
}

/// Content that is addressed by digest but is NOT a cut object (bulk above the
/// core object cap). Kiwix: the ZIM library. Fail closed when sha256 is empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalContentRef {
    pub schema_id: String,
    /// Lowercase hex sha256 of the whole content.
    pub sha256: String,
    pub size: u64,
}
```

A shared `auth_hash` helper (length-prefixed SHA-256 over parts) is also a
candidate, but it would add `sha2` to the API crate's dependencies. First check
that both adapters already use exactly the same framing; if they don't, a shared
helper would change one adapter's hash and break L4 continuity across versions.

## 5. Proposal

1. **Now (docs only):** move rules 1 and 4 from §2 into the `carryon-adapter-api`
   crate docs: kind drives transfer; prerequisites are listed in admission-priority
   order.
2. **Later, behind tests:** add `ExternalContentRef` when a second adapter needs
   content outside the cut. Kiwix's `ZimIdentity` would then embed it. Run both
   adapters' test suites (`cargo test -p carryon-adapter-photocraft` and
   `cargo test -p carryon-adapter-kiwix --no-default-features --lib`, plus the
   `zim-backend` tests locally) before merging.
3. **Not proposed:** a shared position kind, a shared closure walker, or any change
   to the `Adapter` trait.

Label stays `TARGET` until something from §4 lands with both adapters' tests green.
