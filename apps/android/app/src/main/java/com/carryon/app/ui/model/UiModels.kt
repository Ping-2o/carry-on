/*
 * UiModels.kt — app-level, presentation-only models. Screens are stateless
 * `XContent(state, callbacks)` composables over these, so they render the same
 * from live engine state (mappers below) and from deterministic fixtures
 * (Roborazzi screenshots, previews). Nothing here touches JNI.
 */
package com.carryon.app.ui.model

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.DesktopMac
import androidx.compose.material.icons.outlined.Devices
import androidx.compose.material.icons.outlined.Laptop
import androidx.compose.material.icons.outlined.PhoneAndroid
import androidx.compose.material.icons.outlined.TabletAndroid
import androidx.compose.ui.graphics.vector.ImageVector

enum class Tone { Neutral, Info, Success, Warning, Danger }

enum class DeviceKind(val icon: ImageVector) {
    Laptop(Icons.Outlined.Laptop),
    Desktop(Icons.Outlined.DesktopMac),
    Phone(Icons.Outlined.PhoneAndroid),
    Tablet(Icons.Outlined.TabletAndroid),
    Unknown(Icons.Outlined.Devices);

    companion object {
        fun fromDeviceClass(deviceClass: String): DeviceKind = when (deviceClass.lowercase()) {
            "laptop" -> Laptop
            "desktop" -> Desktop
            "mobile", "phone" -> Phone
            "tablet" -> Tablet
            else -> Unknown
        }

        fun guess(name: String): DeviceKind {
            val n = name.lowercase()
            return when {
                "macbook" in n || "laptop" in n -> Laptop
                "mac" in n || "imac" in n || "desktop" in n || "pc" in n -> Desktop
                "tab" in n || "pad" in n -> Tablet
                "pixel" in n || "phone" in n || "galaxy" in n -> Phone
                else -> Unknown
            }
        }
    }
}

enum class DeviceStatus(val label: String, val tone: Tone) {
    Nearby("Nearby", Tone.Info),
    Paired("Trusted", Tone.Success),
    Revoked("Revoked", Tone.Danger),
    ThisDevice("This device", Tone.Neutral),
}

data class DeviceUi(
    val id: String,
    val name: String,
    val kind: DeviceKind,
    val detail: String,
    val status: DeviceStatus,
    val address: String? = null,
)

enum class ActivityRole(val label: String) {
    Received("Received"),
    Sent("Sent"),
    Prepared("Prepared"),
    Demo("Demo"),
}

data class ActivityUi(
    val id: String,
    val title: String,
    val subtitle: String,
    /** Exact engine state string (never the bare word "Ready"). */
    val state: String,
    val role: ActivityRole,
    val whenLabel: String = "",
) {
    val stateLabel: String get() = SessionStates.label(state)
    val tone: Tone get() = SessionStates.tone(state)
}

/** Human wording for the exact §16.2 session states. */
object SessionStates {
    fun label(state: String): String = when (state) {
        "Idle" -> "Idle"
        "Preparing" -> "Preparing"
        "CutProposed" -> "Cut proposed"
        "CutSealed" -> "Sealed"
        "Importing" -> "Importing"
        "ActionReady" -> "Action ready"
        "ReadOnlyActive" -> "Read-only"
        "Failed" -> "Failed"
        "Inconclusive" -> "Inconclusive"
        "Aborted" -> "Aborted"
        else -> state
    }

    fun tone(state: String): Tone = when (state) {
        "ActionReady", "ReadOnlyActive" -> Tone.Success
        "CutSealed", "CutProposed", "Importing", "Preparing" -> Tone.Info
        "Inconclusive", "Aborted" -> Tone.Warning
        "Failed" -> Tone.Danger
        else -> Tone.Neutral
    }

    fun explanation(state: String): String = when (state) {
        "Idle" -> "Nothing in flight."
        "Preparing" -> "Capturing a consistent snapshot of the item."
        "CutProposed" -> "A snapshot is proposed and waiting to be sealed."
        "CutSealed" -> "Sealed and ready to be carried."
        "Importing" -> "Receiving and verifying the bytes."
        "ActionReady" -> "Verified and handed to the app that opens it."
        "ReadOnlyActive" -> "Verified; opened read-only."
        "Failed" -> "Stopped safely. Nothing was opened."
        "Inconclusive" -> "Could not prove the outcome either way."
        "Aborted" -> "Cancelled before anything was opened."
        else -> ""
    }

    val ALL = listOf(
        "Idle", "Preparing", "CutProposed", "CutSealed", "Importing",
        "ActionReady", "ReadOnlyActive", "Failed", "Inconclusive", "Aborted",
    )
}

data class HomeUiState(
    val deviceName: String,
    val shortPin: String,
    val receiving: Boolean,
    val nearby: List<DeviceUi>,
    val trustedCount: Int,
    val recent: List<ActivityUi>,
    val transfersToday: Int,
)

enum class ReceivePhase { Arming, Waiting, Receiving, Done, Failed }

data class ReceiveUiState(
    val phase: ReceivePhase,
    val deviceName: String,
    val address: String,
    val shortPin: String,
    val fullPin: String,
    val advertising: Boolean,
    val advertisedName: String,
    val trustedCount: Int,
    val sessionState: String?,
    val bytesReceived: Long,
    val elapsedMs: Long,
    val verified: Boolean?,
    val error: String?,
    val console: List<String> = emptyList(),
)

data class DevicesUiState(
    val thisDevice: DeviceUi,
    val browsing: Boolean,
    val nearby: List<DeviceUi>,
    val trusted: List<DeviceUi>,
    val firstPeerLatencyMs: Long?,
)

fun formatBytes(bytes: Long): String {
    if (bytes < 1024) return "$bytes B"
    val kb = bytes / 1024.0
    if (kb < 1024) return String.format(java.util.Locale.US, "%.1f KB", kb)
    val mb = kb / 1024.0
    if (mb < 1024) return String.format(java.util.Locale.US, "%.1f MB", mb)
    return String.format(java.util.Locale.US, "%.2f GB", mb / 1024.0)
}

fun formatMs(ms: Long): String =
    if (ms < 1000) "$ms ms" else String.format(java.util.Locale.US, "%.1f s", ms / 1000.0)

/** Deterministic fixtures for screenshots and previews. */
object UiFixtures {
    val thisPhone = DeviceUi(
        id = "self", name = "Pixel 9 Pro", kind = DeviceKind.Phone,
        detail = "Pin 7f3a9c21… · Android 15", status = DeviceStatus.ThisDevice,
        address = "192.168.1.42:47021",
    )

    val nearby = listOf(
        DeviceUi("n1", "MacBook Pro (Studio)", DeviceKind.Laptop, "Pairing required · just now", DeviceStatus.Nearby, "192.168.1.20:47100"),
        DeviceUi("n2", "Galaxy Tab S10", DeviceKind.Tablet, "Trusted · 2 min ago", DeviceStatus.Nearby, "192.168.1.31:47100"),
    )

    val trusted = listOf(
        DeviceUi("t1", "MacBook Pro (Studio)", DeviceKind.Laptop, "Paired 6 Nov 2026 · gen 1", DeviceStatus.Paired),
        DeviceUi("t2", "Mac mini (Office)", DeviceKind.Desktop, "Paired 2 Nov 2026 · gen 2", DeviceStatus.Paired),
        DeviceUi("t3", "Old Nexus", DeviceKind.Phone, "Revoked 28 Oct 2026", DeviceStatus.Revoked),
    )

    val activity = listOf(
        ActivityUi("a1", "Quarterly-plan.key", "From MacBook Pro · 4.8 MB · 412 ms", "ActionReady", ActivityRole.Received, "Today 21:14"),
        ActivityUi("a2", "Site-photos.zip", "From Mac mini · 12.1 MB", "ReadOnlyActive", ActivityRole.Received, "Today 18:02"),
        ActivityUi("a3", "Editor session", "Loopback demo · cut 3", "CutSealed", ActivityRole.Demo, "Yesterday"),
        ActivityUi("a4", "Contract-draft.pdf", "Integrity check refused the bytes", "Failed", ActivityRole.Received, "6 Nov"),
    )

    val home = HomeUiState(
        deviceName = thisPhone.name,
        shortPin = "7f3a9c21",
        receiving = true,
        nearby = nearby,
        trustedCount = 2,
        recent = activity.take(3),
        transfersToday = 2,
    )

    val receiveWaiting = ReceiveUiState(
        phase = ReceivePhase.Waiting,
        deviceName = thisPhone.name,
        address = "192.168.1.42:47021",
        shortPin = "7f3a9c21d0e4b8a6…",
        fullPin = "7f3a9c21d0e4b8a6" + "0".repeat(48),
        advertising = true,
        advertisedName = "carryon-7f3a9c21",
        trustedCount = 2,
        sessionState = null,
        bytesReceived = 0,
        elapsedMs = 0,
        verified = null,
        error = null,
    )

    val receiveDone = receiveWaiting.copy(
        phase = ReceivePhase.Done,
        sessionState = "ActionReady",
        bytesReceived = 5_033_165,
        elapsedMs = 412,
        verified = true,
    )

    val devices = DevicesUiState(
        thisDevice = thisPhone,
        browsing = true,
        nearby = nearby,
        trusted = trusted,
        firstPeerLatencyMs = 184,
    )
}
