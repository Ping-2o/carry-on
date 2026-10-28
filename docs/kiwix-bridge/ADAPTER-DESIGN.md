# carryon-adapter-kiwix — adapter design (Phase 1 design; code skeleton is honest stubs)

Design for the Kiwix mobile-mass flagship (`docs/FRONTIER.md` §3). Follows
`crates/carryon-adapter-api` (the `Adapter` trait) and the structure of
`crates/carryon-adapter-editor` (the reference L3/L4 adapter). Labels: the
*design* is settled; every capability the skeleton does not actually implement
is labeled UNSUPPORTED in code. Evidence citations live in
[`RESEARCH.md`](RESEARCH.md) and [`BUILD-EVIDENCE.md`](BUILD-EVIDENCE.md).

## 1. What "the session" is

A Kiwix reading session, from both apps' storage models (RESEARCH §1):

- which ZIM is open (identity, not bytes: uuid + filename + size, checksum when available)
- the current article (entry path/URL + title)
- the reading position (scroll offset, tab index)
- the platform blob (Android `WebView.saveState` Bundle / iOS
  `WKWebView.interactionState`) — opaque, same-version restore only
- user data: bookmarks, history, reading settings

The one asymmetry this flagship exists to show: the *authoritative mutable
state* is a few KB, while the *content library* (ZIM) is 80+ GB, immutable,
checksummed, and chunk-addressable (RESEARCH §2.2, §3).

## 2. Object model

| object id | kind | dependency role | portable | why |
|---|---|---|---|---|
| `kiwix.session.v1` | Authoritative | **prerequisite** | yes | {zim id/filename/size, article path+title, scroll_y, tab_index, opaque blob} — without it there is no "same session" to restore |
| `kiwix.article.v1` | Derived | **prerequisite** (immediate tier) | yes | current article bytes (HTML) so first paint does NOT need the ZIM; derived from the ZIM, so its parents = the zimfile object |
| `kiwix.zimfile.v1` | Authoritative (L1 content object) | **optional** — bulk, on demand | yes | the 80+ GB ZIM itself; served in `read_object(offset, length)` chunks (cluster-granularity fetch happens under it, RESEARCH §3) |
| `kiwix.bookmarks.v1` | Authoritative | optional | yes | user data; losing it is user-visible but not blocking |
| `kiwix.history.v1` | Authoritative | optional | yes | same |
| `kiwix.settings.v1` | Authoritative | optional | yes | zoom/theme subset |

Object kinds follow `carryon_adapter_api::ObjectKindWire`. Only
Authoritative objects seal into cuts; the core enforces that (as in the editor
adapter). The ZIM is *Authoritative* as an object but *optional* as a
dependency — that combination is exactly what lets the engine's §6.6
three-way dependency split express "carry the 3 KB session, not the 80 GB
library".

`kiwix.article.v1` is Derived: if the destination already owns the same ZIM
(uuid+size match), the plan marks it optional (it can be re-derived locally by
the destination's own libzim) and the immediate tier shrinks to the session
object alone. That decision is the adapter's in `resolve_action`.

## 3. Actions and dependency plans (§6.6)

- `reader.restore` (read-only, the first useful action):
  - prerequisites: `kiwix.session.v1`, `kiwix.article.v1` (only when the
    destination lacks the ZIM — see above), `kiwix.meta.v1`
  - provenance: `kiwix.meta.v1`
  - optional: `kiwix.settings.v1`, `kiwix.bookmarks.v1`, `kiwix.history.v1`,
    `kiwix.zimfile.v1`
- `reader.navigate` (mutating, L4 single-writer): like `document.edit` in the
  reference adapter — updates current article + scroll (and history);
  prerequisites = session + meta; the destination must hold authority.
- `library.materialize` (bulk, optional): asks the destination to materialize
  ZIM byte ranges into its local file so libzim can open it natively;
  prerequisites = `kiwix.meta.v1`; optional = `kiwix.zimfile.v1` chunks. The
  budget admission (TARGETS #5) refuses an over-budget bulk import *before
  bytes move* — the 80 GB never crosses unless the user explicitly pays.

## 4. Progressive strategy — the byte math (recap of RESEARCH §3)

- Prerequisite tier: `kiwix.session.v1` (~10²–10³ B) + `kiwix.article.v1`
  (~10⁴–10⁶ B). First action (`reader.restore`) happens at KB–MB scale.
- Optional tier: ZIM bulk. Cluster-granularity on-demand fetch is bounded by
  the ZIM format (80-byte header, 8-byte pointer-table slots, cluster offsets
  in the header — RESEARCH §3): header + tables first, then only the clusters
  the user actually visits.
- This is the flagship demo of `bytes_before_first_action` (TARGETS #4):
  strategy A ("send prerequisites + optional") would ship the 80 GB; strategy
  D ships KBs first. The measured ratio on a real 80 GB ZIM is a Phase 2
  benchmark item (label: TARGET).

## 5. L4 authority transfer (§21.2), for the reading position

Same binding discipline as the reference adapter: the proposal is plain
JSON `{session, cut_number, auth_hash}` where `auth_hash` = sha256 over the
**mutable authoritative state** — `session ∪ bookmarks ∪ history ∪ settings`
(length-prefixed, ordered). The destination re-derives it from imported state
and must agree before the epoch opens. The ZIM file is **immutable content,
excluded from `auth_hash`** — authority is over *where you are in the book*,
not over the book.

Single-writer meaning here: after transfer, only the destination may run
`reader.navigate` (move the position, extend history, add bookmarks); the
source, killed or alive, cannot. `library.materialize` (immutable bulk) stays
legal on both sides because it does not touch authoritative mutable state.

## 6. Fail-closed rules (§24)

- No libzim backend available (the Phase-1 skeleton): every byte-serving or
  ZIM-dependent call returns `ADAPTER_INCOMPATIBLE`/`ACTION_UNSUPPORTED`
  with a reason string — never fabricated bytes. (In the skeleton the ZIM
  object is UNSUPPORTED: there is no digest to serve it from yet.)
- ZIM identity mismatch (uuid+size ≠ imported state): the destination's
  `accept_authority_transfer` returns Incompatible.
- Generation moves between `begin_snapshot` and `finish`: StaleGeneration.
- Over-budget `library.materialize`: refused by the *core's* budget admission
  before any bytes move (TARGETS #5); the adapter additionally refuses when
  the requested range is outside the ZIM.

## 7. Integration surface — recommendation

Three options considered:

1. **Inside the Kiwix apps (fork both apps).** Deepest UX (position is a
   first-class tab attribute; restore is seamless), but GPL-3.0 fork
   maintenance across two codebases, and every Kiwix release drifts. The
   stores also carry Kiwix's own signing identity. REJECTED for Phase 2.
2. **Standalone Carry-On companion app opening the same ZIM (no fork).**
   Carry-On ships its own small reader shell (libzim + a web view) that reads
   the *same ZIM files* and — critically — the same continuation state. This
   needs no Kiwix cooperation, keeps Carry-On's license surface to libzim
   (GPL with linking review), works on all three platforms from one Rust
   core, and matches the adapter design above 1:1 (session object is
   Carry-On's own format, not Kiwix's CoreData/Room schema). Cost: the user
   reads in the companion, not in Kiwix itself (L0 launcher can still open
   Kiwix for library management). **RECOMMENDED for Phase 2.**
3. **Bindings at the libkiwix level.** Weakest: libkiwix at this pin has no
   reading API (RESEARCH §2.3); it would give bookmark-XML compatibility
   with the Android app but no position model. Keep as an *optional*
   compat shim inside option 2 later.

Rationale: the flagship's point is the *continuation engine over massive
immutable content*, not modifying Kiwix. Option 2 proves it without asking
Kiwix for anything.

## 8. Phase-1 skeleton (this branch)

`crates/carryon-adapter-kiwix` implements the full `Adapter` trait shape with:

- real, working: object model, manifests with sha256 digests, chunked
  `read_object` over in-memory state, dependency plans for the three actions,
  session/hash computation, generation/staleness, L4 proposal
  prepare/accept/commit with state-bound `auth_hash` (all in pure Rust,
  test-covered);
- UNSUPPORTED (labeled in code): anything needing libzim — ZIM serving,
  article extraction from a real ZIM, ZIM identity verification beyond
  uuid+size. The libzim backend lands in Phase 2 behind the same trait,
  built from the cross-files proven in BUILD-EVIDENCE §B/§C.
