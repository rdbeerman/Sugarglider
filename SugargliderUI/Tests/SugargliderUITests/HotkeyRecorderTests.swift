// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import AppKit
import XCTest

@testable import SugargliderUI

/// The text that the Hotkeys pane's recorder sends is the text that
/// `parse_hotkey_string` in `src/ui/preferences_json.rs` reads back.
@MainActor
final class HotkeyRecorderTests: XCTestCase {
  private func record(_ flags: NSEvent.ModifierFlags, _ keyCode: UInt16) -> String {
    HotkeyRecorderNSView.hotkeyText(modifierFlags: flags, keyCode: keyCode)
  }

  func testRecordsTheTextOfEveryKindOfKey() {
    XCTAssertEqual(record([.option], 4), "⌥H")
    XCTAssertEqual(record([.control, .option, .shift, .command], 4), "⌃⌥⇧⌘H")
    XCTAssertEqual(record([.option], 36), "⌥↩")
    XCTAssertEqual(record([.option], 48), "⌥⇥")
    XCTAssertEqual(record([.option], 51), "⌥⌫")
    XCTAssertEqual(record([.option], 42), "⌥\\")
    XCTAssertEqual(record([.option], 49), "⌥Space")
    XCTAssertEqual(record([.option], 123), "⌥←")
    XCTAssertEqual(record([.option], 122), "⌥F1")
    XCTAssertEqual(record([.option], 83), "⌥Numpad1")
    XCTAssertEqual(record([.command], 82), "⌘Numpad0")
    XCTAssertEqual(record([.option], 71), "⌥NumLock")
    XCTAssertEqual(record([.option], 117), "⌥Delete")
    XCTAssertEqual(record([.option], 115), "⌥Home")
    XCTAssertEqual(record([.option], 97), "⌥F6")
  }

  /// A key with no modifier, a modifier alone, and a key that the config
  /// has no name for are not recordable.
  func testRecordsNothingWithoutAModifierOrAName() {
    XCTAssertEqual(record([], 4), "")
    XCTAssertEqual(record([.shift], 56), "")
    XCTAssertEqual(record([.option], 200), "")
  }
}
