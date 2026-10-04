# Carry-On vs. other ways to "move your work to another device"

Carry-On is a **cooperative cross-platform continuation engine**: you begin supported work
on one device and continue a *declared, typed action* on another. It is deliberately **not**
process/memory migration and **not** a way to run foreign application binaries — an
application is supported only through files, standard activation, or an explicit typed
adapter (see [`AGENTS.md`](../AGENTS.md) and the spec).

This table compares Carry-On with the common alternatives. Every Carry-On claim is tied to
a spec section / requirement id or a measured result; see [`TARGETS.md`](TARGETS.md) for the
claim labels (`PROVED` / `IMPLEMENTED` / `OBSERVED` / `TARGET`) and evidence pointers.

| Capability | **Carry-On** | Cloud save / reopen | rsync | CRIU (checkpoint/restore) | VM / live migration | Ordinary demand loading |
|---|---|---|---|---|---|---|
| Continue a *working session* (not just a file) incl. unsaved edits + view state | **Yes** — typed multi-object session (document + unsaved_edits + meta authoritative, navigation optional), L3 structured continuation | No (only what was saved) | No (file bytes only) | Yes, but as an opaque process image | Yes, whole machine image | No |
| Works across **different OS / CPU arch** (e.g. macOS→Android arm64) | **Yes** — content + typed adapters, verified physically mac→Galaxy A04 | Yes (app-specific) | Yes (bytes) | **No** (same kernel/arch) | **No** (same hypervisor/arch) | Yes |
| Moves **no executable code** from state (no RCE-from-state) | **Yes** — manifests name content digests only (§3.8) | Yes | Yes | **No** — restores code pages | **No** — runs the migrated image | Yes |
| **Single-writer authority** handoff (never two writers) | **Yes** — epoch handoff; interrupted commit → ambiguous, writes blocked, recoverable (AUTH-001..005) | No (last-write-wins / conflicts) | No | N/A (one instance) | N/A (one instance) | No |
| **Source-off execution** (destination proceeds after the source is gone) | **Yes** — proven by killing the mac source after authority moves | Depends on the cloud, not the source | N/A | Source stops by definition | Source stops by definition | No (source is the loader) |
| **Progressive** pre-action transfer (move only what the first action needs) | **Yes** — authoritative prerequisites up front, optional deferred | No (whole document) | No (whole tree, or whole changed files) | No (whole image) | No (whole memory) | Yes (but nothing until first use) |
| **Bytes before first useful action** | **Lowest measured** — progressive moves the authoritative closure only; measured < 50% of a full selected-state transfer | Whole file | Whole changed set | Whole image | Whole memory | Zero up front, then blocks on first use |
| **Content-addressed dedup + two-phase verify** of every byte | **Yes** — stage → whole-object digest verify → atomic publish (CORE-004); never exposes a partial as valid | Varies | Delta/rolling-hash, no typed verify | No | No | No |
| **Fail-closed on corruption / loss** | **Yes** — corrupt chunk, lost connection, stale generation, over-budget, malicious manifest all rejected (see fault-injection suite) | Varies | Retries/partial | Restore fails opaque | Migration aborts | Partial/opaque |
| **Bounded preparation overhead** when *no* handoff happens | **Yes** — action-conditioned planning + budget admission (§6.8) | None (but no continuation either) | None | None | None | None |
| Not arbitrary process/memory migration | **By design** | — | — | **Is** process migration | **Is** machine migration | — |

## Reading the table

- **Cloud save/reopen** is the everyday baseline: it moves a *saved* file and reopens it.
  It loses the unsaved buffer and the view state, offers no single-writer guarantee, and
  moves the whole document. Carry-On carries the live session and moves only what the first
  action needs.
- **rsync** is an excellent *file* mover with delta transfer, but it has no notion of a
  session, authority, or a typed action — it is a lower layer, not a continuation engine.
- **CRIU** and **VM/live migration** are the powerful "move the running thing" tools, but
  they move opaque process/memory images, require the *same kernel/architecture*, and carry
  executable state. Carry-On explicitly refuses that model: it is typed continuation across
  heterogeneous devices with no code moved from state.
- **Ordinary demand loading** moves nothing up front and blocks on first use. Carry-On's
  progressive strategy is the middle path: reach ACTION_READY by moving the authoritative
  prerequisites, then defer the optional payload — measured to beat both "move everything"
  (fewer pre-action bytes) and "move nothing" (no first-action stall for required state).

The quantitative side of the last three rows is in the benchmark:
[`BENCHMARK.md`](BENCHMARK.md) (loopback study, independent A/C/D) and the real
Mac→Android campaign (`RESULTS-xdevice.md` in an evidence archive under
[`evidence/`](evidence/)).
