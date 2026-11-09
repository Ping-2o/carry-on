/*
 * PairingScreen.kt — the stage-5 PIN/QR pairing UI (spec §18.4):
 * destination arm (QR + six-digit code), source join (camera QR scan, #26,
 * or payload paste / typed code as the fallback), verification string
 * confirmation on both sides, and the §18.4.9 revoke UX.
 *
 * Labels (§2 law): the run's evidence class derives from the address the
 * source dials (loopback = LOCAL; LAN = PHYSICAL). The camera scan is TARGET
 * until a device run is recorded. A scanned code is only a candidate: the
 * engine parses it (fail closed) and the join then takes exactly the
 * pasted-text path. CAMERA is requested only when the user taps "Scan QR".
 */
package com.carryon.app.ui.screens

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.ContextCompat
import androidx.compose.ui.unit.dp
import com.carryon.app.pairing.PairingArmed
import com.carryon.app.pairing.PairingFlow
import com.carryon.app.pairing.PairingState
import com.carryon.app.pairing.QrScanGate
import com.carryon.app.pairing.QrScanResult
import com.carryon.app.ui.AppGraph
import com.carryon.app.ui.components.AsciiBox
import com.carryon.app.ui.components.AsciiRule
import com.carryon.app.ui.theme.ConsoleTextStyle
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import dev.carryon.kit.pairingPayloadParse
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.launch

/** zxing BitMatrix drawn as Compose Canvas — no bitmaps, no permissions. */
@Composable
private fun QrImage(payload: String, sizeDp: Int = 260) {
    val matrix = remember(payload) {
        try {
            QRCodeWriter().encode(
                payload, BarcodeFormat.QR_CODE, 33, 33, mapOf(
                    com.google.zxing.EncodeHintType.MARGIN to 1,
                ),
            )
        } catch (e: Exception) {
            null
        }
    }
    if (matrix == null) {
        Text("QR render failed — use the payload text below", style = ConsoleTextStyle)
        return
    }
    Canvas(modifier = Modifier.size(sizeDp.dp).padding(8.dp)) {
        val cell = size.width / matrix.width.toFloat()
        for (x in 0 until matrix.width) {
            for (y in 0 until matrix.height) {
                if (matrix.get(x, y)) {
                    drawRect(
                        color = androidx.compose.ui.graphics.Color.Black,
                        topLeft = Offset(x * cell, y * cell),
                        size = Size(cell, cell),
                    )
                }
            }
        }
    }
}

@Composable
fun PairingScreen() {
    val topology = AppGraph.topology
    val scope = remember { kotlinx.coroutines.MainScope() }
    val identityCard by topology.identity.collectAsState()

    LaunchedEffect(Unit) { topology.ensureLoaded() }

    var armed by remember { mutableStateOf<PairingArmed?>(null) }
    var awaiting by remember { mutableStateOf<PairingState.AwaitingConfirm?>(null) }
    var result by remember { mutableStateOf<String?>(null) }
    var events by remember { mutableStateOf(listOf<String>()) }
    var role by remember { mutableStateOf<String?>(null) }
    var srcAddr by remember { mutableStateOf("") }
    var srcCode by remember { mutableStateOf("") }
    var srcPayload by remember { mutableStateOf("") }
    var scanning by remember { mutableStateOf(false) }
    val context = LocalContext.current

    fun log(s: String) { events = (events + s).takeLast(30) }

    // §18.4.2 camera path (#26): the scanned text is a CANDIDATE only. The
    // engine parses it (fail closed), the fields are prefilled for review, and
    // "Connect & pair" then takes exactly the pasted-text path (pinned connect).
    fun acceptScan(raw: String) {
        when (val screened = QrScanGate.screen(raw)) {
            is QrScanResult.Rejected -> log("scan rejected: ${screened.why} (fail closed, nothing filled)")
            is QrScanResult.Candidate -> try {
                val p = pairingPayloadParse(screened.payload)
                srcPayload = screened.payload
                srcAddr = p.addr
                srcCode = p.code
                log("scanned ${p.deviceName} @ ${p.addr}: engine-validated; check it, then Connect & pair")
            } catch (e: Exception) {
                srcPayload = ""
                log("scanned code refused by the engine (${e.message}): fail closed, nothing filled")
            }
        }
    }

    val cameraPermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) {
            scanning = true
        } else {
            log("camera permission denied: type the code or paste the payload instead")
        }
    }

    Column(
        modifier = Modifier.verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text("Device Pairing (§18.4)", style = MaterialTheme.typography.headlineSmall)
        Text(
            "QR + six-digit code on the destination; the source enters/pastes pairing data.\n" +
                "Both screens must show the SAME verification string — confirm on BOTH.\n" +
                "DISCLOSURE: pilot UI; camera scan is TARGET (no device run recorded), typing/pasting stays as the fallback;\n" +
                "no platform is supported (§30 unrun).",
            style = ConsoleTextStyle,
            color = MaterialTheme.colorScheme.secondary,
        )
        AsciiRule()

        // ---- role choose ----
        if (role == null) {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Button(onClick = {
                    role = "destination"
                    scope.launch {
                        val card = topology.ensureLoaded() ?: return@launch
                        // The durable identity is the engine handle behind the
                        // card; arm with it (PLAT-002 restore).
                        val id = topology.durableIdentity() ?: return@launch
                        try {
                            val a = PairingFlow.armDestination(id, card.deviceName)
                            armed = a
                            log("armed: ${a.displayAddr}  code ${a.code}")
                        } catch (e: Exception) {
                            log("arm failed: ${e.message}")
                            role = null
                        }
                    }
                }) { Text("Show my QR (destination)") }
                OutlinedButton(onClick = { role = "source" }) { Text("Join a device (source)") }
            }
        }

        // ---- destination ----
        armed?.let { a ->
            AsciiBox(
                title = "DESTINATION — SCAN OR READ",
                lines = listOf(
                    "name:  ${identityCard?.deviceName ?: "?"}",
                    "pin:   ${a.pinHex.take(16)}…",
                    "addr:  ${a.displayAddr}",
                    "code:  ${a.code}",
                ),
            )
            QrImage(payload = a.payloadUri)
            Text(a.payloadUri, style = ConsoleTextStyle)
            if (awaiting == null) {
                Text("waiting for a source to connect… (provisional channel §18.4.3)",
                    style = ConsoleTextStyle, color = MaterialTheme.colorScheme.secondary)
                Button(onClick = {
                    scope.launch {
                        try {
                            val state = PairingFlow.awaitPairing(
                                armed = a,
                                confirm = { step ->
                                    awaiting = step
                                    // Resolve when the user taps Confirm/Cancel below.
                                    val decision = CompletableDeferred<Boolean>()
                                    confirmDecision = decision
                                    decision.await()
                                },
                                onEvent = ::log,
                            )
                            awaiting = null
                            when (state) {
                                is PairingState.Paired -> {
                                    topology.recordPairing(state.peerName, state.peerPin, PairingFlow.utcNow())
                                    result = "paired: ${state.peerName} (${state.peerPin.take(16)}…) — trust persisted (sealed)"
                                    log("paired + persisted (§18.4.6-7)")
                                }
                                is PairingState.Refused -> { result = "REFUSED: ${state.why}"; log("refused: ${state.why}") }
                                PairingState.Cancelled -> { result = "cancelled — nothing pinned" }
                                else -> {}
                            }
                            armed?.close(); armed = null; role = null
                        } catch (e: Exception) {
                            log("await failed: ${e.message}")
                            armed?.close(); armed = null; role = null
                        }
                    }
                }) { Text("Accept pairing") }
            }
        }

        // ---- source ----
        if (role == "source") {
            AsciiBox(title = "SOURCE — SCAN OR ENTER PAIRING DATA", lines = listOf(
                "scan the destination's QR with the camera, or type its",
                "address + six-digit code (and paste the payload text if you can).",
            ))
            if (scanning) {
                QrScannerView(
                    onScanned = { text ->
                        scanning = false
                        acceptScan(text)
                    },
                    onError = { why ->
                        scanning = false
                        log("camera unavailable: $why (type or paste instead)")
                    },
                    modifier = Modifier.size(280.dp),
                )
                TextButton(onClick = { scanning = false }) { Text("Stop scanning") }
            } else {
                OutlinedButton(onClick = {
                    val granted = ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
                        PackageManager.PERMISSION_GRANTED
                    if (granted) scanning = true else cameraPermission.launch(Manifest.permission.CAMERA)
                }) { Text("Scan QR (camera)") }
            }
            OutlinedTextField(
                value = srcAddr,
                onValueChange = { srcAddr = it },
                label = { Text("destination addr (e.g. ${srcAddrIfBlank()})") },
                singleLine = true,
            )
            OutlinedTextField(
                value = srcCode,
                onValueChange = { srcCode = it },
                label = { Text("six-digit code") },
                singleLine = true,
            )
            OutlinedTextField(
                value = srcPayload,
                onValueChange = { srcPayload = it },
                label = { Text("payload text (optional — QR path)") },
            )
            Button(onClick = {
                scope.launch {
                    val id = topology.durableIdentity() ?: run {
                        log("identity not loaded yet"); return@launch
                    }
                    try {
                        val state = PairingFlow.joinDestination(
                            identity = id,
                            addr = srcAddr.trim(),
                            code = srcCode.trim(),
                            payloadUri = srcPayload.trim().ifBlank { null },
                            confirm = { step ->
                                awaiting = step
                                val decision = CompletableDeferred<Boolean>()
                                confirmDecision = decision
                                decision.await()
                            },
                            onEvent = ::log,
                        )
                        awaiting = null
                        when (state) {
                            is PairingState.Paired -> {
                                topology.recordPairing(state.peerName, state.peerPin, PairingFlow.utcNow())
                                result = "paired: ${state.peerPin.take(16)}… pinned (sealed)"
                            }
                            is PairingState.Refused -> result = "REFUSED: ${state.why}"
                            PairingState.Cancelled -> result = "cancelled — nothing pinned"
                            else -> {}
                        }
                        role = null
                    } catch (e: Exception) {
                        log("join failed: ${e.message}")
                    }
                }
            }) { Text("Connect & pair") }
        }

        // ---- the mutual confirm (§18.4.4-5) ----
        awaiting?.let { step ->
            AsciiBox(
                title = "VERIFY",
                lines = listOf(
                    "peer:   ${step.peerName}",
                    "pin:    ${step.peerPin.take(16)}…",
                    "STRING: ${step.verification}",
                ),
            )
            Text("Both devices must show the SAME string.", style = ConsoleTextStyle)
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Button(onClick = { confirmDecision?.complete(true) }) { Text("Confirm") }
                OutlinedButton(onClick = { confirmDecision?.complete(false) }) { Text("Decline") }
            }
        }

        result?.let {
            Text(it, style = ConsoleTextStyle, color = MaterialTheme.colorScheme.primary)
        }

        AsciiRule()
        Text("Paired devices (§18.4) — revoke blocks reconnect (§18.4.9)", style = MaterialTheme.typography.titleSmall)
        val trust by topology.trust.collectAsState()
        if (trust.isEmpty()) {
            Text("none yet", style = ConsoleTextStyle, color = MaterialTheme.colorScheme.secondary)
        } else {
            trust.forEach { r ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Checkbox(checked = !r.revoked, onCheckedChange = null)
                    Column(Modifier.weight(1f)) {
                        Text("${r.deviceName}  ${r.pinHex.take(12)}…  gen ${r.trustGeneration}" +
                            if (r.revoked) "  [REVOKED]" else "", style = ConsoleTextStyle)
                    }
                    if (!r.revoked) {
                        TextButton(onClick = {
                            topology.revoke(r.pinHex)
                            log("revoked ${r.deviceName} (gen+1; reconnect will be refused §18.4.9)")
                        }) { Text("Revoke") }
                    }
                }
            }
        }

        AsciiRule()
        events.forEach { Text(it, style = ConsoleTextStyle, color = MaterialTheme.colorScheme.secondary) }
    }
}

private fun srcAddrIfBlank() = "192.168.1.20:48961"

/** The pending user decision for the active confirm step (one at a time). */
private var confirmDecision: CompletableDeferred<Boolean>? = null
