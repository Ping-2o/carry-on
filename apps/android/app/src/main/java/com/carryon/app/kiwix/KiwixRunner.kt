/*
 * KiwixRunner.kt — the Kiwix companion reader's engine state machine (stage 10,
 * mobile half). A REAL reading session over a gigantic ZIM:
 *
 *   open      Core.open → register org.carryon.kiwix {zim_path, article_path,
 *             scroll_y, strategy} → create_session (single_writer) →
 *             create_cut → reader.restore (real libzim: article hash) →
 *             read_object (the article bytes, digest-verified by the core).
 *   navigate  reader.navigate {path, scroll_y} (mutating, L4 — owner only) →
 *             create_cut seals the moved position as a NEW cut.
 *   serve     listener_bind("0.0.0.0:0") → accept → negotiate → serve_cut →
 *             may_mutate(true) → serve_authority_transfer → may_mutate(false).
 *             The cut carries the SESSION + ARTICLE (KBs); the ZIM never
 *             crosses (strategy D: the bulk is an unsealed Cache object).
 *   receive   connect → import_cut (the carried position/article) →
 *             reader.restore on the mirror → read_object(article) →
 *             request_authority_transfer → may_mutate flips to the device →
 *             reader.navigate ON THIS DEVICE (the L4 story: the position
 *             physically moved) → evidence bundle + verify.
 *
 * Labels (§2): a run with the mac peer is PHYSICAL (two machines, LAN TLS);
 * with 127.0.0.1 it is LOCAL. Never "supported" (§30 unrun).
 *
 * Threading + lifetime: every blocking engine call runs on Dispatchers.IO.
 * The serve accept is NOT interruptible: the watchdog bounds the UI state
 * machine only; a timed-out run's handles leak with the process (DemoRunner
 * rules — freeing under a blocked call is use-after-free). One run at a time
 * (mutex) + run generations drop zombie writes.
 */
package com.carryon.app.kiwix

import android.content.Context
import com.carryon.app.ffi.Core
import com.carryon.app.ffi.Err
import com.carryon.app.ffi.CoreException
import com.carryon.app.ffi.CutRef
import com.carryon.app.ffi.Identity
import com.carryon.app.ffi.ImportOutcome
import com.carryon.app.ffi.Listener
import com.carryon.app.ffi.Session
import com.carryon.app.ffi.Trust
import com.carryon.app.ffi.importCut
import com.carryon.app.ffi.requestAuthorityTransfer
import com.carryon.app.ffi.serveAuthorityTransfer
import com.carryon.app.ffi.serveCut
import com.carryon.app.ffi.mirrorSessionId
import com.carryon.app.ui.TopologyStore
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import org.json.JSONObject
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.atomic.AtomicLong

class KiwixRunner(
    private val scope: CoroutineScope,
    private val context: Context,
    private val baseDir: File,
    private val topology: TopologyStore,
) {
    private val mutex = Mutex()
    private val _state = MutableStateFlow(KiwixState())
    val state: StateFlow<KiwixState> = _state.asStateFlow()

    private val runGeneration = AtomicLong(0)

    // Handles of the OPENED library (kept for serve/receive). Leaked on a
    // wedged run per the file-header rules; a fresh open replaces them.
    private var core: Core? = null
    private var sessionId: String = ""
    private var cut: CutRef? = null

    fun setZimPath(path: String) = _state.update { if (it.running) it else it.copy(zimPath = path) }

    fun setStrategy(s: KiwixStrategy) = _state.update { if (it.running) it else it.copy(strategy = s) }

    /** Open the ZIM + restore the reading session. `articlePath` is a REAL ZIM
     *  entry (e.g. "index"); `scrollY` is where the previous device left off. */
    fun open(zimPathArg: String, articlePath: String, scrollY: Long) {
        scope.launch {
            mutex.withLock {
                if (_state.value.running) return@withLock
                val gen = runGeneration.incrementAndGet()
                val zimPath = zimPathArg.trim()
                if (zimPath.isEmpty()) {
                    _state.update { it.copy(error = "enter a ZIM path first") }
                    return@withLock
                }
                val started = System.nanoTime()
                _state.update {
                    it.copy(running = true, error = null, articleHtml = null, role = "",
                            zimPath = zimPath,
                            console = listOf("open:  $zimPath"))
                }
                try {
                    val workDir = File(baseDir, "kiwix/${stamp()}").apply { mkdirs() }
                    val c = withContext(Dispatchers.IO) { Core.open(workDir.absolutePath) }
                    core?.let { runCatching { it.close() } }
                    core = c
                    c.setForeground(true)
                    val info = withContext(Dispatchers.IO) {
                        c.registerAdapter(
                            KIWIX_ADAPTER_ID,
                            KiwixParams.adapterParams(
                                zimPath = zimPath,
                                articlePath = articlePath,
                                scrollY = scrollY,
                                strategy = _state.value.strategy,
                            ),
                        )
                    }
                    log(gen, "adapter", "org.carryon.kiwix registered (compiled-in, §3.8)")
                    log(gen, "adapter.info", info.take(180))
                    val sid = withContext(Dispatchers.IO) { c.createSession(KiwixParams.createSessionReq()) }
                    sessionId = sid
                    val sealedCut = withContext(Dispatchers.IO) { c.createCut(sid) }
                    cut = sealedCut
                    log(gen, "session", sid)
                    log(gen, "cut", "sealed number=${sealedCut.number} (two-phase publish, CORE-004)")

                    val restored = withContext(Dispatchers.IO) {
                        restoreAction(c, sid, sealedCut.number)
                    }
                    val html = withContext(Dispatchers.IO) {
                        c.readObjectBytes(restored.articleHash).toString(Charsets.UTF_8)
                    }
                    _state.update {
                        it.copy(
                            running = false,
                            zimOpen = true,
                            zimUuid = restored.zimUuid,
                            zimSizeBytes = restored.zimSize,
                            articlePath = restored.articlePath,
                            articleHtml = html,
                            articleBytes = html.toByteArray(Charsets.UTF_8).size,
                            articleHash = restored.articleHash,
                            scrollY = restored.scrollY,
                            generation = restored.generation,
                            sessionId = sid,
                            cutNumber = sealedCut.number,
                            runElapsedMs = (System.nanoTime() - started) / 1_000_000,
                        )
                    }
                    log(gen, "zim", "${restored.zimUuid}  ${KiwixMetrics.formatBytes(restored.zimSize)}")
                    log(gen, "article", "${restored.articlePath}  ${KiwixMetrics.formatBytes(html.toByteArray().size.toLong())}  sha256 ${restored.articleHash.take(16)}…")
                    log(gen, "strategy", "${_state.value.strategy.code} — the ${KiwixMetrics.formatBytes(restored.zimSize)} ZIM stays local")
                    log(gen, "restored", "generation ${restored.generation} @ scroll ${restored.scrollY}")
                } catch (e: CancellationException) {
                    throw e
                } catch (e: CoreException) {
                    fail(gen, "engine error ${Err.name(e.code)}: ${e.message}")
                } catch (e: Exception) {
                    fail(gen, "open failed: ${e.message}")
                }
            }
        }
    }

    /** Move the reading position on THIS device (owner-only, L4): navigate +
     *  seal a NEW cut so the next serve carries the moved position. */
    fun navigate(path: String, scrollY: Long) {
        scope.launch {
            mutex.withLock {
                val c = core ?: return@withLock
                if (_state.value.running || !_state.value.zimOpen) return@withLock
                val gen = runGeneration.incrementAndGet()
                try {
                    val out = withContext(Dispatchers.IO) {
                        c.executeActionAs(
                            cutRefJson(sessionId, cut?.number ?: 0L),
                            """{"class":"reader.navigate","params":{"path":${jsonStr(path)},"scroll_y":$scrollY}}""",
                            KIWIX_ADAPTER_ID,
                        )
                    }
                    val o = JSONObject(out).getJSONObject("output")
                    val newGen = o.optLong("generation", 0)
                    val newCut = withContext(Dispatchers.IO) { c.createCut(sessionId) }
                    cut = newCut
                    val restored = withContext(Dispatchers.IO) { restoreAction(c, sessionId, newCut.number) }
                    val html = withContext(Dispatchers.IO) {
                        c.readObjectBytes(restored.articleHash).toString(Charsets.UTF_8)
                    }
                    _state.update {
                        it.copy(
                            articlePath = o.optString("article_path", path),
                            articleHtml = html,
                            articleBytes = html.toByteArray().size,
                            articleHash = restored.articleHash,
                            scrollY = scrollY,
                            generation = newGen,
                            cutNumber = newCut.number,
                        )
                    }
                    log(gen, "navigate", "$path @ scroll $scrollY → generation $newGen; new cut ${newCut.number}")
                } catch (e: CancellationException) {
                    throw e
                } catch (e: CoreException) {
                    fail(gen, "navigate failed: ${Err.name(e.code)}: ${e.message}")
                }
            }
        }
    }

    /** Serve the current cut + relinquish authority to ONE peer (§21.2). */
    fun serve(peerPin: String) {
        scope.launch {
            mutex.withLock {
                val c = core ?: return@withLock
                if (_state.value.running || !_state.value.zimOpen) return@withLock
                val pin = peerPin.trim()
                if (pin.isEmpty()) {
                    _state.update { it.copy(error = "enter the peer's pin (hex)") }
                    return@withLock
                }
                val gen = runGeneration.incrementAndGet()
                val started = System.nanoTime()
                _state.update {
                    it.copy(running = true, role = "serve", error = null, mayMutate = null,
                            bytesSent = 0, bytesRecv = 0, bytesBeforeFirstAction = 0,
                            console = it.console + "serve:  arming listener (LAN TLS 1.3)…")
                }
                var listener: Listener? = null
                var identity: Identity? = null
                var trust: Trust? = null
                var sess: Session? = null
                try {
                    val id = topology.durableIdentity()
                        ?: throw IllegalStateException("durable identity not loaded (Keystore)")
                    identity = id
                    val t = Trust.empty()
                    trust = t
                    t.pair("peer", pin, utcNow())
                    val lis = withContext(Dispatchers.IO) { Listener.bind("0.0.0.0:0") }
                    listener = lis
                    val wire = withContext(Dispatchers.IO) { lis.addr() }
                    val port = wire.substringAfterLast(':')
                    val display = "${com.carryon.app.handoff.HandoffFlow.siteLocalIpv4() ?: wire.substringBefore(':')}:$port"
                    val ownPin = withContext(Dispatchers.IO) { id.pinHex() }
                    _state.update {
                        it.copy(
                            listenerWireAddr = wire,
                            displayAddr = display,
                            ownPinHex = ownPin,
                            sessionId = sessionId,
                            cutNumber = cut?.number ?: 0L,
                        )
                    }
                    log(gen, "listening", "$display (wire $wire)")
                    log(gen, "own pin", ownPin)
                    log(gen, "session", "$sessionId  cut=${cut?.number}")
                    log(gen, "peer pin", "$pin — waiting for the destination…")

                    val s = withContext(Dispatchers.IO) { Session.accept(lis, id, t) }
                    sess = s
                    withContext(Dispatchers.IO) { s.serverNegotiate() }
                    log(gen, "peer", "TLS 1.3 up (mutual pin)")
                    withContext(Dispatchers.IO) { c.serveCut(s) }
                    log(gen, "serve", "cut served (position + article; the ZIM never crossed)")

                    val pre = withContext(Dispatchers.IO) { c.mayMutate(sessionId) }
                    log(gen, "may_mutate.pre", pre.toString())
                    if (!pre) throw CoreException(Err.E_AUTH_BASE + 2, "source is not the owner")
                    withContext(Dispatchers.IO) {
                        c.serveAuthorityTransfer(s, sessionId, cut?.number ?: 0L)
                    }
                    log(gen, "authority", "relinquished; this device is read-only (§21.2)")
                    val post = withContext(Dispatchers.IO) { c.mayMutate(sessionId) }
                    log(gen, "may_mutate.post", post.toString())
                    if (post) throw CoreException(Err.E_AUTH_BASE, "single-writer flip violated (§21.2)")

                    val (bs, br) = withContext(Dispatchers.IO) { s.bytes() }
                    _state.update { it.copy(bytesSent = bs, bytesRecv = br, mayMutate = post) }
                    log(gen, "wire", "sent=$bs recv=$br (MEASURED, §5.6)")

                    // Source-side evidence: the session's own journal (the
                    // carried position + the relinquishment receipt).
                    runCatching {
                        val extra = JSONObject().apply {
                            put("role", "source")
                            put("level", "L3-reading-position")
                            put("strategy", _state.value.strategy.code)
                            put("zim_size", _state.value.zimSizeBytes)
                            put("article_path", _state.value.articlePath)
                            put("scroll_y", _state.value.scrollY)
                            put("bytes_transferred_sent", bs)
                            put("bytes_transferred_recv", br)
                        }
                        val bundle = withContext(Dispatchers.IO) {
                            c.exportEvidenceWith(sessionId, extra.toString())
                        }
                        val file = File(baseDir, "kiwix/${stamp()}/src/evidence.json")
                        file.parentFile?.mkdirs()
                        file.writeText(bundle)
                        log(gen, "evidence", "source bundle ${bundle.length} B -> ${file.absolutePath}")
                    }
                    _state.update {
                        it.copy(
                            running = false,
                            runElapsedMs = (System.nanoTime() - started) / 1_000_000,
                        )
                    }
                    log(gen, "result", "served — reading position moved to the peer (PHYSICAL evidence when the peer is the mac)")
                } catch (e: CancellationException) {
                    throw e
                } catch (e: CoreException) {
                    fail(gen, "serve failed: ${Err.name(e.code)}: ${e.message}")
                } catch (e: Exception) {
                    fail(gen, "serve failed: ${e.message}")
                } finally {
                    // Best-effort closes; a wedged accept keeps them open (leak
                    // by the documented rules) — runCatching keeps that silent.
                    runCatching { sess?.close() }
                    runCatching { listener?.close() }
                    runCatching { trust?.close() }
                }
            }
        }
    }

    /** Receive a reading position from a peer and restore it HERE.
     *
     *  `carriedArticle` / `carriedScroll` are the peer's position: the adapter
     *  reconstructs the destination-side session state from them, and the
     *  engine's authority-transfer gate compares that reconstruction against
     *  the IMPORTED session object's digest — a mismatch fails closed
     *  (ADAPTER_Incompatible "authoritative-state hash does not match imported
     *  state"). This is the same discipline the physical shell uses (the
     *  digest is the proof the reconstruction is the carried state). */
    fun receive(
        addr: String,
        sourcePin: String,
        remoteSession: String,
        remoteCut: Long,
        carriedArticle: String,
        carriedScroll: Long,
    ) {
        scope.launch {
            mutex.withLock {
                if (_state.value.running || !_state.value.zimOpen) return@withLock
                val a = addr.trim()
                val pin = sourcePin.trim()
                val rs = remoteSession.trim()
                if (a.isEmpty() || pin.isEmpty() || rs.isEmpty()) {
                    _state.update { it.copy(error = "addr, source pin and remote session are required") }
                    return@withLock
                }
                val gen = runGeneration.incrementAndGet()
                val started = System.nanoTime()
                _state.update {
                    it.copy(running = true, role = "receive", error = null, mayMutate = null,
                            bytesSent = 0, bytesRecv = 0, bytesBeforeFirstAction = 0,
                            console = it.console + "receive:  connecting to $a…")
                }
                val workDir = File(baseDir, "kiwix/${stamp()}/dst").apply { mkdirs() }
                var dst: Core? = null
                var identity: Identity? = null
                var trust: Trust? = null
                var sess: Session? = null
                try {
                    val c = withContext(Dispatchers.IO) { Core.open(workDir.absolutePath) }
                    dst = c
                    c.setForeground(true)
                    withContext(Dispatchers.IO) {
                        c.registerAdapter(
                            KIWIX_ADAPTER_ID,
                            KiwixParams.adapterParams(
                                zimPath = _state.value.zimPath.trim(),
                                articlePath = carriedArticle.ifBlank { "index" },
                                scrollY = carriedScroll,
                                strategy = _state.value.strategy,
                                sessionName = "kiwix-dst",
                            ),
                        )
                    }
                    val id = topology.durableIdentity()
                        ?: throw IllegalStateException("durable identity not loaded (Keystore)")
                    identity = id
                    val t = Trust.empty()
                    trust = t
                    t.pair("source", pin, utcNow())
                    val s = withContext(Dispatchers.IO) { Session.connect(a, id, t) }
                    sess = s
                    withContext(Dispatchers.IO) { s.clientNegotiate() }
                    log(gen, "peer", "TLS 1.3 up (mutual pin) — importing the cut…")
                    val outcome = withContext(Dispatchers.IO) { c.importCut(s, rs, remoteCut) }
                    val (bs, br) = withContext(Dispatchers.IO) { s.bytes() }
                    // The DESTINATION-side wire recv up to the first action.
                    _state.update { it.copy(bytesSent = bs, bytesRecv = br, bytesBeforeFirstAction = br) }
                    when (val o = outcome) {
                        is ImportOutcome.Completed ->
                            log(gen, "import", "completed (mirror_cut=${o.mirrorCut})")
                        is ImportOutcome.Suspended ->
                            throw CoreException(Err.E_TRANSFER_BASE, "suspended (no resume in this flow)")
                        is ImportOutcome.PartialBudget ->
                            log(gen, "import", "PARTIAL-BUDGET published=${o.published} deferred=${o.deferred} (${o.deferredBytes} B) — §6.8")
                    }
                    log(gen, "wire", "sent=$bs recv=$br — bytes_before_first_action")
                    log(gen, "ratio", "${KiwixMetrics.formatRatio(KiwixMetrics.ratioPpm(br, _state.value.zimSizeBytes))} of the ${KiwixMetrics.formatBytes(_state.value.zimSizeBytes)} ZIM")

                    val mirror = withContext(Dispatchers.IO) { mirrorSessionId(rs) }
                    val tRestore0 = System.nanoTime()
                    val restored = withContext(Dispatchers.IO) { restoreAction(c, mirror, remoteCut) }
                    // time_to_action_ready: connect→negotiate→import→restore
                    // (the FIRST useful action, §5.6 measured).
                    val ttaMs = (System.nanoTime() - started) / 1e6
                    val html = withContext(Dispatchers.IO) {
                        c.readObjectBytes(restored.articleHash).toString(Charsets.UTF_8)
                    }
                    _state.update {
                        it.copy(
                            restoredArticlePath = restored.articlePath,
                            restoredScrollY = restored.scrollY,
                            articlePath = restored.articlePath,
                            articleHtml = html,
                            articleBytes = html.toByteArray().size,
                            articleHash = restored.articleHash,
                            scrollY = restored.scrollY,
                            generation = restored.generation,
                            cutNumber = remoteCut,
                            sessionId = mirror,
                        )
                    }
                    log(gen, "restore", "${restored.articlePath} @ scroll ${restored.scrollY} (generation ${restored.generation})")
                    log(gen, "article", "${KiwixMetrics.formatBytes(html.toByteArray().size.toLong())} rendered from the carried object")

                    val pre = withContext(Dispatchers.IO) { c.mayMutate(mirror) }
                    log(gen, "may_mutate.pre", pre.toString())
                    if (pre) throw CoreException(Err.E_AUTH_BASE + 2, "mirror must not start authoritative")
                    withContext(Dispatchers.IO) {
                        c.requestAuthorityTransfer(s, mirror, KIWIX_ADAPTER_ID)
                    }
                    val post = withContext(Dispatchers.IO) { c.mayMutate(mirror) }
                    log(gen, "may_mutate.post", post.toString())
                    if (!post) throw CoreException(Err.E_AUTH_BASE, "dest did not become the owner")
                    _state.update { it.copy(mayMutate = post) }

                    // THE L4 STORY: move the position on THIS device's CPU now
                    // that it owns the session. The article bytes come from the
                    // local ZIM through the libzim backend.
                    val nav = withContext(Dispatchers.IO) {
                        c.executeActionAs(
                            cutRefJson(mirror, remoteCut),
                            """{"class":"reader.navigate","params":{"path":"${KiwixParams.jsonEscape(restored.articlePath)}","scroll_y":${restored.scrollY + 1}}}""",
                            KIWIX_ADAPTER_ID,
                        )
                    }
                    val navGen = JSONObject(nav).getJSONObject("output").optLong("generation", 0)
                    log(gen, "navigate", "${restored.articlePath} @ scroll ${restored.scrollY + 1} → generation $navGen (on-device, owner)")

                    // Evidence bundle + verify (rendered, never executed — EVD-005).
                    val extra = JSONObject().apply {
                        put("role", "destination")
                        put("level", "L3-reading-position")
                        put("strategy", _state.value.strategy.code)
                        put("zim_size", _state.value.zimSizeBytes)
                        put("zim_uuid", _state.value.zimUuid)
                        put("article_path", restored.articlePath)
                        put("scroll_y", restored.scrollY)
                        put("article_bytes", html.toByteArray().size)
                        put("bytes_before_first_action", br)
                        put("time_to_action_ready_ms", ttaMs)
                        put("bytes_transferred_sent", bs)
                        put("bytes_transferred_recv", br)
                        put("ratio_ppm", KiwixMetrics.ratioPpm(br, _state.value.zimSizeBytes))
                        put("authority_epoch_after", navGen)
                    }
                    val bundle = withContext(Dispatchers.IO) { c.exportEvidenceWith(mirror, extra.toString()) }
                    val file = File(workDir, "evidence.json").apply { writeText(bundle) }
                    val report = withContext(Dispatchers.IO) { c.verifyEvidence(file.absolutePath) }
                    val ok = JSONObject(report).optBoolean("ok", false)
                    log(gen, "evidence", "exported ${bundle.length} B -> ${file.absolutePath}")
                    log(gen, "verify_evidence", report.take(160))
                    if (!ok) throw CoreException(Err.E_INTERNAL_BASE, "evidence verification did not report ok")
                    _state.update {
                        it.copy(
                            running = false,
                            evidenceBundlePath = file.absolutePath,
                            verifyOk = ok,
                            runElapsedMs = (System.nanoTime() - started) / 1_000_000,
                        )
                    }
                    log(gen, "result", "restored + authoritative — reading continued on THIS device (OBSERVED per role)")
                } catch (e: CancellationException) {
                    throw e
                } catch (e: CoreException) {
                    fail(gen, "receive failed: ${Err.name(e.code)}: ${e.message}")
                } catch (e: Exception) {
                    fail(gen, "receive failed: ${e.message}")
                } finally {
                    runCatching { sess?.close() }
                    runCatching { trust?.close() }
                }
            }
        }
    }

    /* ---- helpers ---- */

    private fun restoreAction(c: Core, session: String, cutNumber: Long): RestoredSession =
        RestoredSession.parseActionResult(
            c.executeActionAs(
                cutRefJson(session, cutNumber),
                """{"class":"reader.restore","params":{}}""",
                KIWIX_ADAPTER_ID,
            ),
        )

    private fun cutRefJson(session: String, number: Long): String =
        "{\"session\":\"${KiwixParams.jsonEscape(session)}\",\"number\":$number}"

    private fun jsonStr(s: String): String = "\"${KiwixParams.jsonEscape(s)}\""

    private fun utcNow(): String =
        java.time.format.DateTimeFormatter.ISO_INSTANT
            .format(java.time.Instant.now().truncatedTo(java.time.temporal.ChronoUnit.SECONDS))

    private fun stamp(): String = SimpleDateFormat("yyyyMMdd-HHmmss", Locale.US).format(Date())

    private fun log(gen: Long, key: String, value: String) {
        if (gen != runGeneration.get()) return
        val line = "$key:  $value"
        _state.update { it.copy(console = (it.console + line).takeLast(MAX_CONSOLE_LINES)) }
    }

    private fun fail(gen: Long, why: String) {
        if (gen != runGeneration.get()) return
        _state.update {
            it.copy(
                running = false,
                error = why,
                console = (it.console + "FAIL:  $why").takeLast(MAX_CONSOLE_LINES),
            )
        }
    }

    companion object {
        private const val MAX_CONSOLE_LINES = 400
    }
}
