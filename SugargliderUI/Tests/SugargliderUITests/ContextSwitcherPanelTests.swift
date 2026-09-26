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

/// The panel's settings (spec, "Switcher": the panel).
@MainActor
final class ContextSwitcherPanelSetupTests: XCTestCase {
  func testThePanelTakesKeysWithoutBecomingMainOrHidingWithTheApp() {
    let panel = ContextSwitcherPanel()
    XCTAssertTrue(panel.canBecomeKey)
    XCTAssertFalse(panel.canBecomeMain)
    XCTAssertTrue(panel.isFloatingPanel)
    XCTAssertFalse(panel.hidesOnDeactivate)
    XCTAssertFalse(panel.becomesKeyOnlyIfNeeded)
    XCTAssertFalse(panel.isReleasedWhenClosed)
    XCTAssertTrue(panel.collectionBehavior.contains(.fullScreenAuxiliary))
    XCTAssertFalse(panel.isVisible)
  }

  /// The panel centers on the screen with the payload's display id, or on
  /// the main screen when no screen has it.
  func testThePanelCentersOnThePayloadsScreen() {
    let left = NSRect(x: -1920, y: 0, width: 1920, height: 1055)
    let main = NSRect(x: 0, y: 25, width: 1512, height: 944)
    let screens: [(displayId: UInt32?, visibleFrame: NSRect)] = [(1, main), (5, left)]
    func frame(_ displayId: UInt32?, main: NSRect? = main) -> NSRect? {
      ContextSwitcherPanelPresenter.visibleFrame(
        displayId: displayId, screens: screens, main: main)
    }
    XCTAssertEqual(frame(5), left)
    XCTAssertEqual(frame(1), main)
    XCTAssertEqual(frame(9), main)
    XCTAssertEqual(frame(nil), main)
    XCTAssertEqual(frame(9, main: nil), main)
    XCTAssertNil(
      ContextSwitcherPanelPresenter.visibleFrame(displayId: 1, screens: [], main: nil))
  }
}

/// Records what the controller puts on screen, instead of showing a panel.
@MainActor
final class FakePresenter: ContextSwitcherPresenter {
  var onResignKey: () -> Void = {}
  private(set) var presented: [(model: ContextSwitcherModel, displayId: UInt32?)] = []
  private(set) var dismissed = 0

  func present(_ model: ContextSwitcherModel, displayId: UInt32?) {
    presented.append((model, displayId))
  }

  func dismiss() {
    dismissed += 1
  }
}

/// The controller's show, hide, and lose-focus logic, over a presenter that
/// shows nothing.
@MainActor
final class ContextSwitcherControllerTests: XCTestCase {
  private var presenter = FakePresenter()
  private var backend = FakeBackend()
  private var controller: ContextSwitcherController!

  override func setUp() async throws {
    try await super.setUp()
    presenter = FakePresenter()
    backend = FakeBackend()
    controller = ContextSwitcherController(
      presenter: presenter,
      makeBackend: { [unowned self] in
        self.backend
      })
  }

  func testShowPresentsTheSwitcherOnThePayloadsScreen() throws {
    controller.show(json: Fixtures.showPayload)
    XCTAssertEqual(presenter.presented.count, 1)
    XCTAssertEqual(presenter.presented.first?.displayId, 1)
    XCTAssertEqual(presenter.presented.first?.model.payload, try Fixtures.payload())
    XCTAssertTrue(presenter.presented.first?.model === controller.model)
    XCTAssertEqual(presenter.dismissed, 0)
  }

  /// D8: showing while the switcher is open closes it, so the hotkey
  /// toggles it. The next show opens a fresh switcher: an empty query and
  /// the list.
  func testASecondShowClosesTheSwitcherAndAThirdOpensAFreshOne() throws {
    controller.show(json: Fixtures.showPayload)
    let first = try XCTUnwrap(controller.model)
    first.query = "cli"
    first.handle(.commandN)
    XCTAssertEqual(first.mode, .naming)

    controller.show(json: Fixtures.showPayload)
    XCTAssertNil(controller.model)
    XCTAssertEqual(presenter.presented.count, 1)
    XCTAssertEqual(presenter.dismissed, 1)

    controller.show(json: Fixtures.showPayload)
    let second = try XCTUnwrap(controller.model)
    XCTAssertFalse(second === first)
    XCTAssertEqual(second.query, "")
    XCTAssertEqual(second.mode, .list)
    XCTAssertEqual(presenter.presented.count, 2)
  }

  func testHidingAClosedSwitcherDoesNothing() {
    controller.hide()
    XCTAssertEqual(presenter.dismissed, 0)
    controller.show(json: Fixtures.showPayload)
    controller.hide()
    controller.hide()
    XCTAssertEqual(presenter.dismissed, 1)
  }

  /// The switcher closes when it loses key status.
  func testLosingKeyStatusClosesTheSwitcher() {
    controller.show(json: Fixtures.showPayload)
    presenter.onResignKey()
    XCTAssertNil(controller.model)
    XCTAssertEqual(presenter.dismissed, 1)
    presenter.onResignKey()
    XCTAssertEqual(presenter.dismissed, 1)
  }

  /// Esc from the list and a command that Rust accepts close the switcher.
  func testTheModelClosesTheSwitcher() throws {
    controller.show(json: Fixtures.showPayload)
    try XCTUnwrap(controller.model).handle(.escape)
    XCTAssertNil(controller.model)
    XCTAssertEqual(presenter.dismissed, 1)

    controller.show(json: Fixtures.showPayload)
    try XCTUnwrap(controller.model).handle(.enter)
    XCTAssertEqual(backend.sent, [.switchTo(.named(Fixtures.clientWork))])
    XCTAssertNil(controller.model)
    XCTAssertEqual(presenter.dismissed, 2)
  }

  /// A switcher that was closed can't close the one opened after it.
  func testAClosedSwitcherCantCloseTheNextOne() throws {
    controller.show(json: Fixtures.showPayload)
    let old = try XCTUnwrap(controller.model)
    controller.hide()
    controller.show(json: Fixtures.showPayload)
    old.onClose()
    XCTAssertNotNil(controller.model)
    XCTAssertEqual(presenter.dismissed, 1)
  }

  /// S7: a payload that doesn't decode opens nothing, and a show while the
  /// switcher is open closes it whatever the payload, so an open switcher
  /// never keeps showing an old payload.
  func testAPayloadThatDoesntDecodeOpensNothing() {
    for json in ["", "{", #"{"contexts": 4}"#, #"{ "switch": "everything" }"#] {
      controller.show(json: json)
      XCTAssertNil(controller.model, json)
    }
    XCTAssertEqual(presenter.presented.count, 0)

    controller.show(json: Fixtures.showPayload)
    controller.show(json: "{")
    XCTAssertNil(controller.model)
    XCTAssertEqual(presenter.dismissed, 1)
  }

  /// S7: a NULL payload from Rust is ignored.
  func testANullPayloadIsIgnored() {
    showContextSwitcher(json: nil)
    XCTAssertFalse(
      (NSApp?.windows ?? []).contains { $0 is ContextSwitcherPanel && $0.isVisible })
  }
}
