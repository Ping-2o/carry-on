/*
 * PhotoCraftAuthorityTests.swift — pure, engine-free tests for the #24 authority
 * pane: who holds single-writer authority, and the gate that makes two-writer
 * attempts fail closed in the UI (PhotoCraftModel.send/receive consult exactly
 * PhotoCraftAuthority.refusal). The wire hand-over itself is the engine's
 * serve/request authority transfer, covered by the Rust/FFI tests.
 */

import CarryOnKit
import XCTest

final class PhotoCraftAuthorityTests: XCTestCase {
    func testReconcileDetectsTwoWritersAndNoWriter() {
        XCTAssertEqual(PhotoCraftAuthority.reconcile(localMayMutate: true, peerHolds: true), .conflict)
        XCTAssertEqual(PhotoCraftAuthority.reconcile(localMayMutate: true, peerHolds: false), .thisInstance)
        XCTAssertEqual(PhotoCraftAuthority.reconcile(localMayMutate: false, peerHolds: true), .peer)
        XCTAssertEqual(PhotoCraftAuthority.reconcile(localMayMutate: false, peerHolds: false), .ambiguous)
    }

    func testTwoWriterAttemptsFailClosed() {
        // Conflict, ambiguity and an in-flight hand-over refuse every write.
        for state in [PhotoCraftAuthorityState.conflict, .ambiguous, .handingOver] {
            for attempt in [PhotoCraftWriteAttempt.send, .receive, .receiveAndTakeAuthority] {
                XCTAssertNotNil(PhotoCraftAuthority.refusal(attempt, state: state), "\(attempt) in \(state)")
            }
        }
        // A replica sealing a new cut would be a second writer.
        XCTAssertNotNil(PhotoCraftAuthority.refusal(.send, state: .peer))
        // Taking authority back by an explicit hand-over stays possible.
        XCTAssertNil(PhotoCraftAuthority.refusal(.receiveAndTakeAuthority, state: .peer))
        XCTAssertNil(PhotoCraftAuthority.refusal(.receive, state: .peer))
    }

    func testTheSingleWriterMayAct() {
        for state in [PhotoCraftAuthorityState.origin, .thisInstance] {
            XCTAssertNil(PhotoCraftAuthority.refusal(.send, state: state), "\(state)")
            XCTAssertNil(PhotoCraftAuthority.refusal(.receive, state: state), "\(state)")
        }
    }

    func testOnlySingleWriterStatesMayWrite() {
        let writers = PhotoCraftAuthorityState.allCases.filter(\.mayWrite)
        XCTAssertEqual(Set(writers), [.origin, .thisInstance])
        for state in PhotoCraftAuthorityState.allCases {
            XCTAssertFalse(state.holder.isEmpty)
        }
    }

    func testHandOverRequestIsExplicitAndDistinct() {
        XCTAssertNotEqual(PhotoCraftAuthority.handOverRequest, PhotoCraftDescriptor.marker)
        XCTAssertNotEqual(PhotoCraftAuthority.handOverRequest, PhotoCraftDescriptor.done)
        XCTAssertTrue(PhotoCraftAuthority.isHandOverRequest(
            features: [PhotoCraftDescriptor.marker, PhotoCraftAuthority.handOverRequest]))
        // A plain window hop or the done-ack never triggers a hand-over.
        XCTAssertFalse(PhotoCraftAuthority.isHandOverRequest(features: [PhotoCraftDescriptor.marker]))
        XCTAssertFalse(PhotoCraftAuthority.isHandOverRequest(
            features: [PhotoCraftDescriptor.marker, PhotoCraftDescriptor.done]))
        XCTAssertFalse(PhotoCraftAuthority.isHandOverRequest(
            features: [PhotoCraftDescriptor.marker, PhotoCraftDescriptor.done, PhotoCraftAuthority.handOverRequest]))
    }

    func testReportCarriesDataOnly() {
        let r = PhotoCraftAuthorityReport(state: .peer, mayMutate: false,
                                          receiptJSON: #"{"new_epoch":2,"receipts":[]}"#,
                                          detail: "handed over")
        XCTAssertEqual(r.state, .peer)
        XCTAssertFalse(r.mayMutate)
        XCTAssertEqual(PhotoCraftAuthorityReport(state: .origin, mayMutate: true, detail: "x").receiptJSON, nil)
    }
}
