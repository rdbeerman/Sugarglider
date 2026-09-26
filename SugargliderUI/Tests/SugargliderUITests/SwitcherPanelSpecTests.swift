// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import AppKit
import XCTest

@testable import SugargliderUI

/// The panel and the Rust-to-Swift functions (spec, "Switcher" and "Swift
/// bridge").
///
/// These tests never show a panel: this machine's desktop is live, and a
/// key panel would take the user's keystrokes.
@MainActor
final class SwitcherPanelSpecTests: XCTestCase {
  private var switcherPanelIsVisible: Bool {
    (NSApp?.windows ?? []).contains { $0 is ContextSwitcherPanel && $0.isVisible }
  }

  /// "The panel is an NSPanel with the nonactivatingPanel and borderless
  /// styles, at floating level. It joins all Spaces and overrides
  /// canBecomeKey to return true."
  func testThePanelIsANonActivatingBorderlessFloatingPanelOnAllSpaces() {
    let panel = ContextSwitcherPanel()
    XCTAssertEqual(panel.styleMask, [.nonactivatingPanel, .borderless])
    XCTAssertEqual(panel.level, .floating)
    XCTAssertTrue(panel.collectionBehavior.contains(.canJoinAllSpaces))
    XCTAssertTrue(panel.canBecomeKey)
    XCTAssertFalse(panel.isVisible)
  }

  /// "It appears centered on the focused screen", also on a screen left of
  /// the main one, whose origin is negative.
  func testThePanelCentersOnAScreenLeftOfTheMainScreen() {
    XCTAssertEqual(
      ContextSwitcherPanel.frame(
        size: NSSize(width: 600, height: 460),
        centeredIn: NSRect(x: -1920, y: 0, width: 1920, height: 1055)
      ),
      NSRect(x: -1260, y: 298, width: 600, height: 460)
    )
  }

  /// A centered frame whose midpoint falls on a half point is placed on
  /// whole points.
  func testThePanelCentersOnWholePointsForAnOddScreenWidth() {
    XCTAssertEqual(
      ContextSwitcherPanel.frame(
        size: NSSize(width: 600, height: 460),
        centeredIn: NSRect(x: 0, y: 25, width: 1511, height: 944)
      ),
      NSRect(x: 456, y: 267, width: 600, height: 460)
    )
  }

  /// "Rust to Swift: add sugarglider_show_context_switcher(json) and
  /// sugarglider_hide_context_switcher()." Rust finds them by these names.
  func testShowAndHideAreExportedUnderTheirCNames() {
    XCTAssertNotNil(RustContextSwitcherBackend.lookUp("sugarglider_show_context_switcher"))
    XCTAssertNotNil(RustContextSwitcherBackend.lookUp("sugarglider_hide_context_switcher"))
  }

  /// The contract: hiding is safe at any time, from the reactor's thread
  /// too.
  func testHidingFromAnotherThreadWithNothingShownIsSafe() {
    let done = expectation(description: "hidden")
    DispatchQueue.global().async {
      hideContextSwitcher()
      DispatchQueue.main.async { done.fulfill() }
    }
    wait(for: [done], timeout: 5)
    XCTAssertFalse(switcherPanelIsVisible)
  }

  /// The contract: a payload that doesn't decode is logged and shows
  /// nothing, whether it comes through the C function on another thread or
  /// straight to the controller.
  func testAnUndecodablePayloadShowsNoPanel() {
    let done = expectation(description: "shown")
    DispatchQueue.global().async {
      "{".withCString { showContextSwitcher(json: $0) }
      DispatchQueue.main.async { done.fulfill() }
    }
    wait(for: [done], timeout: 5)
    for json in [#"{"contexts": 4}"#, #"{ "switch": "everything" }"#, ""] {
      ContextSwitcherController.shared.show(json: json)
    }
    XCTAssertFalse(switcherPanelIsVisible)
  }
}
