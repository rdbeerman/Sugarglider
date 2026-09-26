// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import AppKit
import XCTest

@testable import SugargliderUI

/// The keys table of the spec's "Switcher" section, through the view model.
///
/// The fixture lists Sugarglider (active), Client work, Unsorted, and
/// Everything, and its target window is the Chrome window.
@MainActor
final class SwitcherKeysSpecTests: SwitcherSpecTestCase {
  // MARK: ↩

  /// R16: ↩ on the active context switches to it again, so a switch to the
  /// active context parks windows that drifted in.
  func testEnterOnTheActiveContextSwitchesToItAgain() throws {
    let model = try makeModel()
    highlight(model, row: 0)
    model.handle(.enter)
    XCTAssertEqual(backend.sent, [.switchTo(.named(Fixtures.sugarglider))])
    XCTAssertEqual(closed, 1)
  }

  /// User story 2: type "cli", press ↩, and "Client work" becomes active.
  func testTypingANamePrefixAndEnterSwitchesToThatContext() throws {
    let model = try makeModel()
    model.query = "cli"
    XCTAssertEqual(model.highlight, 0)
    XCTAssertTrue(model.handle(.enter))
    XCTAssertEqual(backend.sent, [.switchTo(.named(Fixtures.clientWork))])
    XCTAssertEqual(closed, 1)
  }

  // MARK: ⌘↩ and ⇧⌘↩

  /// User story 7: ⌘↩ and ⇧⌘↩ carry the target window to the highlighted
  /// named context, the active one included.
  func testWindowKeysOnTheActiveContextCarryTheTargetWindow() throws {
    let model = try makeModel()
    highlight(model, row: 0)
    model.handle(.commandEnter)
    let second = try makeModel()
    highlight(second, row: 0)
    second.handle(.shiftCommandEnter)
    XCTAssertEqual(
      backend.sent,
      [
        .addWindow(window: Fixtures.chrome, context: Fixtures.sugarglider),
        .moveWindow(window: Fixtures.chrome, context: Fixtures.sugarglider),
      ]
    )
    XCTAssertEqual(closed, 2)
  }

  func testShiftCommandEnterOnTheNewContextRowAsksToCreateItFirst() throws {
    let model = try makeModel()
    model.query = "Deep work"
    XCTAssertEqual(model.rows, [.newContext("Deep work")])
    XCTAssertTrue(model.handle(.shiftCommandEnter))
    XCTAssertEqual(model.message, "Press ↩ to create “Deep work” first.")
    XCTAssertEqual(model.mode, .list)
    XCTAssertEqual(backend.sent, [])
    XCTAssertEqual(closed, 0)
  }

  /// The contract: a null target window disables ⌘↩, ⇧⌘↩, and ⌘P.
  func testWithoutATargetWindowEachWindowKeyExplainsAndSendsNothing() throws {
    var payload = try Fixtures.payload()
    payload.targetWindow = nil
    for key: SwitcherKey in [.commandEnter, .shiftCommandEnter, .commandP] {
      let model = try makeModel(payload)
      XCTAssertTrue(model.handle(key), "\(key)")
      XCTAssertEqual(model.message, ContextSwitcherModel.noTargetWindowMessage, "\(key)")
      XCTAssertEqual(model.mode, .list, "\(key)")
    }
    XCTAssertEqual(backend.sent, [])
    XCTAssertEqual(closed, 0)
  }

  // MARK: Built-in entries

  /// R29 and the contract: the panel never sends `add_window`,
  /// `move_window`, `edit`, `rename`, `set_number`, or `delete` for Unsorted
  /// or Everything. It explains inline instead.
  func testBuiltInEntriesRefuseEveryKeyThatNeedsANamedContext() throws {
    for (row, name) in [(2, "Unsorted"), (3, "Everything")] {
      let cases: [(SwitcherKey, String)] = [
        (.commandEnter, "Windows can't be added to \(name)."),
        (.shiftCommandEnter, "Windows can't be moved to \(name)."),
        (.commandE, "\(name) can't be edited."),
        (.commandR, "\(name) can't be renamed."),
        (.commandDigit(1), "\(name) can't be numbered."),
        (.commandDigit(9), "\(name) can't be numbered."),
        (.commandDelete, "\(name) can't be deleted."),
      ]
      for (key, message) in cases {
        let model = try makeModel()
        highlight(model, row: row)
        XCTAssertTrue(model.handle(key), "\(name) \(key)")
        XCTAssertEqual(model.message, message, "\(name) \(key)")
        XCTAssertEqual(model.mode, .list, "\(name) \(key)")
        XCTAssertEqual(model.highlight, row, "\(name) \(key)")
      }
    }
    XCTAssertEqual(backend.sent, [])
    XCTAssertEqual(closed, 0)
  }

  // MARK: ⌘1 – ⌘9

  /// R5: ⌘1 to ⌘9 each give the highlighted context that number.
  func testEveryCommandDigitGivesTheHighlightedContextThatNumber() throws {
    for number in 1...9 {
      let model = try makeModel()
      XCTAssertTrue(model.handle(.commandDigit(number)))
    }
    XCTAssertEqual(
      backend.sent,
      (1...9).map { SwitcherCommand.setNumber(context: Fixtures.clientWork, number: $0) }
    )
    XCTAssertEqual(closed, 9)
  }

  /// R5: a context that already has a number can be given another one.
  func testCommandDigitRenumbersTheActiveContext() throws {
    let model = try makeModel()
    highlight(model, row: 0)
    model.handle(.commandDigit(2))
    XCTAssertEqual(backend.sent, [.setNumber(context: Fixtures.sugarglider, number: 2)])
  }

  // MARK: ⌘P

  /// ⌘P pins or unpins the target window, whichever row is highlighted,
  /// the built-in entries and the "New context" row included (R3).
  func testCommandPTogglesTheTargetWindowOnEveryRow() throws {
    for row in 0..<4 {
      let model = try makeModel()
      highlight(model, row: row)
      XCTAssertTrue(model.handle(.commandP), "row \(row)")
    }
    let model = try makeModel()
    model.query = "Deep work"
    model.handle(.commandP)
    XCTAssertEqual(backend.sent, Array(repeating: .togglePinned(window: Fixtures.chrome), count: 5))
    XCTAssertEqual(closed, 5)
  }

  /// ⌘P works before the first context exists (R3: a pinned window is a
  /// member of every context, including contexts created later).
  func testCommandPTogglesTheTargetWindowOnTheFirstTime() throws {
    let target = SwitcherWindowId(pid: 42, idx: 7)
    backend.ranks[""] = SpecPayload.ranked([(.everything, .emptyQuery)])
    let model = try makeModel(
      SpecPayload.payload(target: target, windows: [SpecPayload.window(target, "Chat", "Slack")])
    )
    XCTAssertEqual(model.rows, [])
    model.handle(.commandP)
    XCTAssertEqual(backend.sent, [.togglePinned(window: target)])
  }

  // MARK: An empty list

  /// The first time the list is empty. Keys that act on a row do nothing,
  /// ⌘N asks for a name, and Esc closes.
  func testKeysOnTheEmptyFirstTimeListDoNothing() throws {
    backend.ranks[""] = SpecPayload.ranked([(.everything, .emptyQuery)])
    let model = try makeModel(
      SpecPayload.payload(
        target: Fixtures.slack, windows: [SpecPayload.window(Fixtures.slack, "general", "Slack")])
    )
    XCTAssertTrue(model.isFirstTime)
    for key: SwitcherKey in [
      .up, .down, .enter, .commandEnter, .shiftCommandEnter, .commandE, .commandR,
      .commandDigit(4), .commandDelete,
    ] {
      XCTAssertTrue(model.handle(key), "\(key)")
      XCTAssertEqual(model.mode, .list, "\(key)")
      XCTAssertNil(model.message, "\(key)")
      XCTAssertEqual(model.highlight, 0, "\(key)")
    }
    XCTAssertEqual(backend.sent, [])
    XCTAssertEqual(closed, 0)

    model.handle(.commandN)
    XCTAssertEqual(model.mode, .naming)
    model.handle(.escape)
    XCTAssertEqual(model.mode, .list)
    XCTAssertEqual(closed, 0)
    model.handle(.escape)
    XCTAssertEqual(closed, 1)
  }

  // MARK: Esc

  /// Esc closes from the list. From the other views it goes back to the
  /// list, keeping the query.
  func testEscapeFromEveryOtherViewReturnsToTheList() throws {
    let model = try makeModel()
    model.handle(.commandN)
    XCTAssertEqual(model.mode, .naming)
    XCTAssertTrue(model.handle(.escape))
    XCTAssertEqual(model.mode, .list)

    model.handle(.commandR)
    XCTAssertEqual(model.mode, .rename(Fixtures.clientWork))
    XCTAssertTrue(model.handle(.escape))
    XCTAssertEqual(model.mode, .list)

    model.handle(.commandE)
    XCTAssertEqual(model.mode, .edit(Fixtures.clientWork))
    XCTAssertTrue(model.handle(.escape))
    XCTAssertEqual(model.mode, .list)

    XCTAssertEqual(model.query, "")
    XCTAssertEqual(model.highlight, 1)
    XCTAssertEqual(backend.sent, [])
    XCTAssertEqual(closed, 0)
  }

  // MARK: ⌘⌫

  /// R6: the active context can be deleted, after the inline confirm.
  func testDeletingTheActiveContextAsksInlineFirst() throws {
    let model = try makeModel()
    highlight(model, row: 0)
    model.handle(.commandDelete)
    XCTAssertEqual(model.mode, .confirmDelete(Fixtures.sugarglider))
    XCTAssertEqual(backend.sent, [])
    model.handle(.enter)
    XCTAssertEqual(backend.sent, [.delete(Fixtures.sugarglider)])
    XCTAssertEqual(closed, 1)
  }

  /// While the inline confirm shows, only ↩ and Esc act. Every other key of
  /// the table is used up without a command or a change.
  func testTheDeleteConfirmationIgnoresEveryOtherKey() throws {
    let model = try makeModel()
    model.handle(.commandDelete)
    for key: SwitcherKey in [
      .up, .down, .commandEnter, .shiftCommandEnter, .commandN, .commandE, .commandR, .commandP,
      .commandDelete, .commandDigit(2),
    ] {
      XCTAssertTrue(model.handle(key), "\(key)")
      XCTAssertEqual(model.mode, .confirmDelete(Fixtures.clientWork), "\(key)")
      XCTAssertEqual(model.highlight, 1, "\(key)")
      XCTAssertNil(model.message, "\(key)")
    }
    XCTAssertEqual(backend.sent, [])
    XCTAssertEqual(closed, 0)

    model.handle(.enter)
    XCTAssertEqual(backend.sent, [.delete(Fixtures.clientWork)])
  }

  /// The contract: a rejected command shows Rust's message inline, and the
  /// panel stays open.
  func testARejectedDeleteShowsTheErrorAndStaysOpen() throws {
    backend.runError = SwitcherBridgeError(message: "Contexts are turned off")
    let model = try makeModel()
    model.handle(.commandDelete)
    model.handle(.enter)
    XCTAssertEqual(backend.sent, [.delete(Fixtures.clientWork)])
    XCTAssertEqual(model.message, "Contexts are turned off")
    XCTAssertEqual(closed, 0)
  }

  // MARK: Key mapping

  private func key(
    _ keyCode: UInt16,
    _ modifiers: NSEvent.ModifierFlags = [],
    _ characters: String? = nil
  ) -> SwitcherKey? {
    SwitcherKey(keyCode: keyCode, modifiers: modifiers, characters: characters)
  }

  func testKeypadEnterTakesTheSameModifiersAsReturn() {
    XCTAssertEqual(key(76, [.command, .numericPad], "\u{3}"), .commandEnter)
    XCTAssertEqual(key(76, [.command, .shift, .numericPad], "\u{3}"), .shiftCommandEnter)
    XCTAssertNil(key(76, [.option], "\u{3}"))
    XCTAssertNil(key(76, [.command, .control], "\u{3}"))
  }

  func testCommandDigitsFromTheKeypad() {
    XCTAssertEqual(key(83, [.command, .numericPad], "1"), .commandDigit(1))
    XCTAssertEqual(key(92, [.command, .numericPad], "9"), .commandDigit(9))
    XCTAssertNil(key(82, [.command, .numericPad], "0"))
  }

  /// Each key of the table needs exactly its modifiers.
  func testKeysWithOtherModifiersAreNotTheTablesKeys() {
    XCTAssertNil(key(126, [.shift]))
    XCTAssertNil(key(125, [.option]))
    XCTAssertNil(key(53, [.command], "\u{1b}"))
    XCTAssertNil(key(49, [.shift], " "))
    XCTAssertNil(key(51, [.command, .shift], "\u{7f}"))
    XCTAssertNil(key(51, [.command, .option], "\u{7f}"))
    XCTAssertNil(key(18, [.command, .shift], "1"))
    XCTAssertNil(key(35, [.command, .shift], "p"))
    XCTAssertNil(key(14, [.command, .control], "e"))
  }

  /// ⌘E and ⌘P are letter shortcuts. On a layout that types punctuation on
  /// the ANSI E or P key, that punctuation with ⌘ is not ⌘E or ⌘P: Dvorak
  /// types "." on the E key, and Colemak types ";" on the P key.
  func testCommandPunctuationOnTheEOrPKeyIsNotALetterShortcut() {
    XCTAssertNil(key(14, [.command], "."))
    XCTAssertNil(key(35, [.command], ";"))
    XCTAssertEqual(key(2, [.command], "e"), .commandE)
    XCTAssertEqual(key(15, [.command], "p"), .commandP)
  }

  /// Only a letter outside ASCII falls back to the ANSI letter of its key.
  /// A digit or a symbol on a letter key, or a key without characters, is
  /// no letter shortcut.
  func testOnlyLettersOutsideASCIIMapByKeyCode() {
    XCTAssertEqual(key(14, [.command], "у"), .commandE)
    XCTAssertEqual(key(35, [.command], "з"), .commandP)
    XCTAssertNil(key(45, [.command], "0"))
    XCTAssertNil(key(15, [.command], "§"))
    XCTAssertNil(key(45, [.command], nil))
    XCTAssertNil(key(45, [.command], ""))
  }
}
