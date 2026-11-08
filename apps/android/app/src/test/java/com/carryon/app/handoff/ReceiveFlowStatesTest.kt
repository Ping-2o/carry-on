/*
 * ReceiveFlowStatesTest.kt — JVM unit tests for the pure §16.2 state machine
 * of the stage-6 receive flow: exact state names, the cooperative-suspend
 * resume decision (§11.2 — the transfer survives, it does not fail), and the
 * fail-closed paths (partial_budget §6.8, CORE-004 refusal, Inconclusive).
 */
package com.carryon.app.handoff

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ReceiveFlowStatesTest {

    @Test
    fun states_are_the_exact_162_names_never_bare_Ready() {
        val all = listOf(
            ReceiveFlowEvent.Armed,
            ReceiveFlowEvent.DescriptorReceived,
            ReceiveFlowEvent.ImportLeg("suspended"),
            ReceiveFlowEvent.ImportLeg("completed"),
            ReceiveFlowEvent.GateVerified,
            ReceiveFlowEvent.GateRefused,
            ReceiveFlowEvent.Opened,
            ReceiveFlowEvent.OpenFailed,
            ReceiveFlowEvent.EvidenceVerified(true),
            ReceiveFlowEvent.EvidenceVerified(false),
        ).map { ReceiveFlowStates.state(forEvent = it) }
        all.forEach {
            assertTrue("state name must be a §16.2 name: $it",
                it in listOf(
                    ReceiveFlowStates.IDLE, ReceiveFlowStates.PREPARING,
                    ReceiveFlowStates.IMPORTING, ReceiveFlowStates.ACTION_READY,
                    ReceiveFlowStates.FAILED, ReceiveFlowStates.INCONCLUSIVE,
                ))
        }
        assertTrue("never the bare word Ready", all.none { it == "Ready" })
    }

    @Test
    fun full_happy_chain_reaches_ActionReady() {
        assertEquals(ReceiveFlowStates.IDLE, ReceiveFlowStates.state(forEvent = ReceiveFlowEvent.Armed))
        assertEquals(ReceiveFlowStates.IMPORTING, ReceiveFlowStates.state(forEvent = ReceiveFlowEvent.DescriptorReceived))
        assertEquals(ReceiveFlowStates.ACTION_READY, ReceiveFlowStates.state(forEvent = ReceiveFlowEvent.ImportLeg("completed")))
        assertEquals(ReceiveFlowStates.ACTION_READY, ReceiveFlowStates.state(forEvent = ReceiveFlowEvent.Opened))
        assertEquals(ReceiveFlowStates.ACTION_READY, ReceiveFlowStates.state(forEvent = ReceiveFlowEvent.EvidenceVerified(true)))
    }

    @Test
    fun cooperative_suspend_stays_Importing_and_orders_a_resume_leg() {
        // §11.2: the suspend is a CHECKPOINT, not a failure — the flow keeps
        // Importing and the machine orders another leg.
        assertEquals(ReceiveFlowStates.IMPORTING, ReceiveFlowStates.state(forEvent = ReceiveFlowEvent.ImportLeg("suspended")))
        assertEquals(ReceiveFlowStates.LegDecision.ResumeLeg, ReceiveFlowStates.legAfter("suspended"))
        // …and the resumed leg can complete normally afterwards.
        assertEquals(ReceiveFlowStates.LegDecision.Complete, ReceiveFlowStates.legAfter("completed"))
    }

    @Test
    fun partial_budget_is_a_visible_refusal_not_a_silent_pass() {
        assertEquals(ReceiveFlowStates.FAILED, ReceiveFlowStates.state(forEvent = ReceiveFlowEvent.ImportLeg("partial_budget")))
        assertEquals(ReceiveFlowStates.LegDecision.Fail, ReceiveFlowStates.legAfter("partial_budget"))
    }

    @Test
    fun core004_refusal_fails_closed_and_bad_bundles_are_Inconclusive() {
        assertEquals(ReceiveFlowStates.FAILED, ReceiveFlowStates.state(forEvent = ReceiveFlowEvent.GateRefused))
        assertEquals(ReceiveFlowStates.INCONCLUSIVE, ReceiveFlowStates.state(forEvent = ReceiveFlowEvent.EvidenceVerified(false)))
        assertEquals(ReceiveFlowStates.FAILED, ReceiveFlowStates.state(forEvent = ReceiveFlowEvent.GateRefused))
    }
}
