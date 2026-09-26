// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import AppKit
import XCTest

@testable import SugargliderUI

/// The test process has no Rust side, so these tests cover the path where
/// the switcher's Rust functions are missing.
@MainActor
final class ContextSwitcherBridgeTests: XCTestCase {
  func testMissingRustFunctionsAreNotFound() {
    XCTAssertNil(RustContextSwitcherBackend.lookUp(RustContextSwitcherBackend.rankSymbol))
    XCTAssertNil(RustContextSwitcherBackend.lookUp(RustContextSwitcherBackend.runSymbol))
  }

  func testLookUpFindsAFunctionThatIsLoaded() {
    XCTAssertNotNil(RustContextSwitcherBackend.lookUp("strdup"))
  }

  func testMissingRankFailsWithAMessage() {
    XCTAssertThrowsError(try RustContextSwitcherBackend().rank("cli")) { error in
      XCTAssertEqual(
        error as? SwitcherBridgeError,
        SwitcherBridgeError(
          message: "sugarglider_rank_contexts is missing from this Sugarglider build.")
      )
    }
  }

  func testMissingRunFailsWithAMessage() {
    XCTAssertThrowsError(try RustContextSwitcherBackend().run(.switchTo(.everything))) { error in
      XCTAssertEqual(
        error as? SwitcherBridgeError,
        SwitcherBridgeError(
          message: "sugarglider_run_context_command is missing from this Sugarglider build."
        )
      )
    }
  }

  func testModelWithoutRustShowsInlineErrorsInsteadOfCrashing() throws {
    let model = ContextSwitcherModel(
      payload: try Fixtures.payload(),
      backend: RustContextSwitcherBackend()
    )
    var closed = false
    model.onClose = { closed = true }

    XCTAssertEqual(
      model.rankError,
      "Search is unavailable. sugarglider_rank_contexts is missing from this Sugarglider build."
    )
    XCTAssertEqual(model.rows.count, 4)

    model.handle(.enter)
    XCTAssertEqual(
      model.message,
      "sugarglider_run_context_command is missing from this Sugarglider build."
    )
    XCTAssertFalse(closed)
  }

  func testPanelFrameIsCenteredOnTheVisibleFrame() {
    let visibleFrame = NSRect(x: 1440, y: 25, width: 1920, height: 1055)
    XCTAssertEqual(
      ContextSwitcherPanel.frame(size: NSSize(width: 600, height: 460), centeredIn: visibleFrame),
      NSRect(x: 2100, y: 323, width: 600, height: 460)
    )
  }

  func testKeyEventsMapThroughTheirKeyCodeAndCharacters() throws {
    func event(_ keyCode: UInt16, _ modifiers: NSEvent.ModifierFlags, _ characters: String)
      throws -> NSEvent
    {
      try XCTUnwrap(
        NSEvent.keyEvent(
          with: .keyDown,
          location: .zero,
          modifierFlags: modifiers,
          timestamp: 0,
          windowNumber: 0,
          context: nil,
          characters: characters,
          charactersIgnoringModifiers: characters,
          isARepeat: false,
          keyCode: keyCode
        )
      )
    }

    XCTAssertEqual(SwitcherKey(event: try event(45, [.command], "n")), .commandN)
    XCTAssertEqual(SwitcherKey(event: try event(36, [.command, .shift], "\r")), .shiftCommandEnter)
    XCTAssertEqual(SwitcherKey(event: try event(20, [.command], "3")), .commandDigit(3))
    XCTAssertNil(SwitcherKey(event: try event(0, [], "a")))
  }
}
