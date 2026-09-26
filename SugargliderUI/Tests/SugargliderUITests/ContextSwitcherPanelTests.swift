// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import AppKit
import XCTest

@testable import SugargliderUI

/// The panel's key routing. The panel is never shown: this machine's
/// desktop is live, and a key panel would take the user's keystrokes.
@MainActor
final class ContextSwitcherPanelTests: XCTestCase {
  private var panel: ContextSwitcherPanel!
  private var field: NSTextView!
  private var handled: [SwitcherKey] = []

  override func setUp() async throws {
    try await super.setUp()
    panel = ContextSwitcherPanel()
    field = NSTextView(frame: NSRect(x: 0, y: 0, width: 200, height: 24))
    panel.contentView = field
    XCTAssertTrue(panel.makeFirstResponder(field))
    XCTAssertFalse(panel.isVisible)
    handled = []
    panel.keyHandler = { [weak self] key in
      self?.handled.append(key)
      return true
    }
  }

  override func tearDown() async throws {
    panel.makeFirstResponder(nil)
    panel.contentView = nil
    panel = nil
    field = nil
    try await super.tearDown()
  }

  private func keyDown(
    _ keyCode: UInt16,
    _ modifiers: NSEvent.ModifierFlags = [],
    _ characters: String
  ) throws -> NSEvent {
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

  /// S2: while an input method composes text in the field, ↩, Esc, and the
  /// arrows belong to the input method, not to the list.
  func testKeysGoToTheInputMethodWhileTheFieldComposesText() throws {
    let keys = [
      try keyDown(36, [], "\r"), try keyDown(53, [], "\u{1b}"), try keyDown(125, [], ""),
      try keyDown(126, [], ""), try keyDown(45, [.command], "n"),
    ]
    field.setMarkedText(
      "にほ",
      selectedRange: NSRange(location: 2, length: 0),
      replacementRange: NSRange(location: NSNotFound, length: 0)
    )
    XCTAssertTrue(panel.isComposingText)
    for key in keys {
      XCTAssertFalse(panel.handleKeyDown(key))
    }
    XCTAssertEqual(handled, [])

    field.unmarkText()
    XCTAssertFalse(panel.isComposingText)
    for key in keys {
      XCTAssertTrue(panel.handleKeyDown(key))
    }
    XCTAssertEqual(handled, [.enter, .escape, .down, .up, .commandN])
  }

  /// A key the switcher doesn't use goes to the field.
  func testKeysOutsideTheTableGoToTheField() throws {
    XCTAssertFalse(panel.handleKeyDown(try keyDown(0, [], "a")))
    XCTAssertFalse(panel.handleKeyDown(try keyDown(11, [.command], "b")))
    XCTAssertEqual(handled, [])
  }

  /// S3: the standard edit shortcuts map to their actions, also on a
  /// layout that doesn't type Latin letters.
  func testEditShortcutsMapToTheStandardEditActions() {
    func action(
      _ keyCode: UInt16, _ modifiers: NSEvent.ModifierFlags, _ characters: String
    ) -> Selector? {
      ContextSwitcherPanel.editAction(
        keyCode: keyCode, modifiers: modifiers, characters: characters)
    }
    XCTAssertEqual(action(7, [.command], "x"), #selector(NSText.cut(_:)))
    XCTAssertEqual(action(8, [.command], "c"), #selector(NSText.copy(_:)))
    XCTAssertEqual(action(9, [.command], "v"), #selector(NSText.paste(_:)))
    XCTAssertEqual(action(0, [.command], "a"), #selector(NSText.selectAll(_:)))
    XCTAssertEqual(action(6, [.command], "z"), Selector(("undo:")))
    XCTAssertEqual(action(6, [.command, .shift], "Z"), Selector(("redo:")))
    XCTAssertEqual(action(8, [.command], "с"), #selector(NSText.copy(_:)))
    XCTAssertEqual(action(47, [.command], "v"), #selector(NSText.paste(_:)))
    XCTAssertNil(action(8, [.command, .option], "c"))
    XCTAssertNil(action(8, [.control], "c"))
    XCTAssertNil(action(8, [], "c"))
    XCTAssertNil(action(6, [.command], ";"))
  }

  /// S3: ⌘A, ⌘Z, and ⇧⌘Z act on the focused text field. The panel has no
  /// Edit menu to send them. Cut, copy, and paste aren't sent here: they
  /// would use the user's real pasteboard.
  func testEditShortcutsActOnTheFocusedTextField() throws {
    panel.keyHandler = { _ in false }
    field.allowsUndo = true
    field.string = "Client"
    field.setSelectedRange(NSRange(location: 6, length: 0))
    field.insertText(" work", replacementRange: NSRange(location: NSNotFound, length: 0))
    XCTAssertEqual(field.string, "Client work")

    XCTAssertTrue(panel.handleKeyDown(try keyDown(6, [.command], "z")))
    XCTAssertEqual(field.string, "Client")
    XCTAssertTrue(panel.handleKeyDown(try keyDown(6, [.command, .shift], "Z")))
    XCTAssertEqual(field.string, "Client work")
    XCTAssertTrue(panel.handleKeyDown(try keyDown(0, [.command], "a")))
    XCTAssertEqual(field.selectedRange(), NSRange(location: 0, length: 11))
  }

  /// An edit shortcut that the switcher uses itself stays the switcher's.
  func testTheSwitcherKeepsTheKeysItUses() throws {
    XCTAssertTrue(panel.handleKeyDown(try keyDown(45, [.command], "n")))
    XCTAssertEqual(handled, [.commandN])
  }
}
