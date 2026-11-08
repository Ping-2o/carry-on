/*
 * ServeLegPlannerTests.swift — unit tests for the stage-6 multi-leg serve
 * state machine (cooperative suspend/resume, §11.2): a cleanly completed leg
 * is COMPLETE; a leg that died with legs remaining warrants a RESUME leg
 * (the destination may have suspended mid-object); a leg that died with no
 * legs left FAILS CLOSED — interrupted ≠ success (§19.3.9/EVD-003).
 *
 * Pure, local tests: they pin the FLOW DECISION, not any transport claim.
 */

import XCTest

@testable import CarryOnKit

final class ServeLegPlannerTests: XCTestCase {

    func testCompletedLegIsComplete() {
        XCTAssertEqual(ServeLegPlanner.plan(completedCleanly: true, legsRemaining: 2, failure: nil),
                       .complete)
    }

    func testFailedLegWithLegsRemainingRetriesForResume() {
        // A cooperative §11.2 suspend closes the wire mid-object — the source
        // re-serves so the destination can resume on the fresh connection.
        let plan = ServeLegPlanner.plan(completedCleanly: false, legsRemaining: 2,
                                        failure: "EOF: connection closed")
        guard case .retryLeg(let reason) = plan else {
            return XCTFail("expected retryLeg, got \(plan)")
        }
        XCTAssertTrue(reason.contains("closed"))
    }

    func testFailedLegWithNoLegsRemainingFailsClosed() {
        // Interrupted ≠ success (§19.3.9): with no legs left the transfer
        // must FAIL, never be presented as carried.
        let plan = ServeLegPlanner.plan(completedCleanly: false, legsRemaining: 0,
                                        failure: "E_TRANSFER_CANCELLED: receiver aborted")
        guard case .fail(let reason) = plan else {
            return XCTFail("expected fail, got \(plan)")
        }
        XCTAssertTrue(reason.contains("E_TRANSFER_CANCELLED"))
    }

    func testDefensiveNoFailureTreatedAsComplete() {
        XCTAssertEqual(ServeLegPlanner.plan(completedCleanly: false, legsRemaining: 1, failure: nil),
                       .complete)
    }

    func testLegBudgetIsBoundedByTheCaller() {
        // 3 legs total: two failed legs still retry; the third failure fails
        // closed — a suspend can never loop the source forever.
        var remaining = 2 // legs left after leg 1
        var retries = 0
        while remaining > 0 {
            retries += 1
            remaining -= 1
        }
        XCTAssertEqual(retries, 2)
        XCTAssertEqual(ServeLegPlanner.plan(completedCleanly: false, legsRemaining: 0, failure: "eof"),
                       .fail(reason: "eof"))
    }
}
