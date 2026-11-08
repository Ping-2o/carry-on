/*
 * ReceiveFlowStates.kt — the PURE §16.2 state machine of the stage-6 receive
 * flow (unit-tested on the JVM without the engine): the app-side UI states
 * derive ONLY through this table, so the exact spec names stay law and the
 * suspend/resume legs are a modeled outcome, not ad-hoc UI logic.
 *
 * Rule reminders encoded here:
 *  - the §16.2 states are the ONLY states shown (never the bare "Ready");
 *  - a cooperative suspend (§11.2) keeps the flow Importing — the transfer
 *    is NOT failed, NOT finished: a resume leg is due;
 *  - partial_budget (§6.8) is a legitimate visible refusal → Failed, never
 *    a silent pass;
 *  - a refused CORE-004 gate → Failed; verify-not-ok → Inconclusive.
 */
package com.carryon.app.handoff

/** The events the receive flow emits, reduced to the machine's inputs. */
sealed class ReceiveFlowEvent {
    data object Armed : ReceiveFlowEvent()
    data object DescriptorReceived : ReceiveFlowEvent()
    /** Outcome of one import leg: completed | suspended | partial_budget. */
    data class ImportLeg(val outcome: String) : ReceiveFlowEvent()
    data object GateVerified : ReceiveFlowEvent()
    data object GateRefused : ReceiveFlowEvent()
    data object Opened : ReceiveFlowEvent()
    data object OpenFailed : ReceiveFlowEvent()
    /** The evidence bundle verified (ok=true) or not (Inconclusive). */
    data class EvidenceVerified(val ok: Boolean) : ReceiveFlowEvent()
}

object ReceiveFlowStates {

    /** The exact §16.2 state names the UI may show (law; no bare "Ready"). */
    const val IDLE = "Idle"
    const val PREPARING = "Preparing"
    const val IMPORTING = "Importing"
    const val ACTION_READY = "ActionReady"
    const val FAILED = "Failed"
    const val INCONCLUSIVE = "Inconclusive"

    /** A pure leg decision (mirrors CarryOnKit's ServeLegPlanner semantics):
     *  suspended → another leg is due (bounded by the caller); completed →
     *  done; anything else → fail closed. */
    fun legAfter(outcome: String): LegDecision = when (outcome) {
        "suspended" -> LegDecision.ResumeLeg
        "completed" -> LegDecision.Complete
        else -> LegDecision.Fail
    }

    enum class LegDecision { ResumeLeg, Complete, Fail }

    /** One step of the machine: the event → the §16.2 state to show. */
    fun state(forEvent: ReceiveFlowEvent): String = when (forEvent) {
        ReceiveFlowEvent.Armed -> IDLE
        ReceiveFlowEvent.DescriptorReceived -> IMPORTING
        is ReceiveFlowEvent.ImportLeg -> when (forEvent.outcome) {
            "suspended" -> IMPORTING // §11.2: a resume leg is due — not failed
            "completed" -> ACTION_READY
            else -> FAILED // partial_budget §6.8: a visible refusal
        }
        ReceiveFlowEvent.GateVerified -> ACTION_READY
        ReceiveFlowEvent.GateRefused -> FAILED
        ReceiveFlowEvent.Opened -> ACTION_READY
        ReceiveFlowEvent.OpenFailed -> ACTION_READY // carried + verified; open failed elsewhere — L1 shows ✗
        is ReceiveFlowEvent.EvidenceVerified -> if (forEvent.ok) ACTION_READY else INCONCLUSIVE
    }
}
