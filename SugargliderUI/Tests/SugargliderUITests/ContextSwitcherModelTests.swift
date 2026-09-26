// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import AppKit
import XCTest

@testable import SugargliderUI

/// Records commands and answers rank queries without Rust.
@MainActor
final class FakeBackend: ContextSwitcherBackend {
  var ranks: [String: [SwitcherRankedEntry]] = [:]
  var rankError: SwitcherBridgeError?
  var runError: SwitcherBridgeError?
  private(set) var sent: [SwitcherCommand] = []
  private(set) var queries: [String] = []

  init() {
    ranks[""] = try? ContextSwitcherJSON.decode(
      [SwitcherRankedEntry].self,
      from: Fixtures.rankForEmptyQuery
    )
    ranks["cli"] = try? ContextSwitcherJSON.decode(
      [SwitcherRankedEntry].self,
      from: Fixtures.rankForCli
    )
  }

  func rank(_ query: String) throws -> [SwitcherRankedEntry] {
    queries.append(query)
    if let rankError { throw rankError }
    return ranks[query] ?? []
  }

  func run(_ command: SwitcherCommand) throws {
    sent.append(command)
    if let runError { throw runError }
  }
}

@MainActor
final class ContextSwitcherModelTests: XCTestCase {
  private var backend = FakeBackend()
  private var closed = 0

  override func setUp() async throws {
    try await super.setUp()
    backend = FakeBackend()
    closed = 0
  }

  private func makeModel(_ payload: SwitcherPayload? = nil) throws -> ContextSwitcherModel {
    let model = ContextSwitcherModel(payload: try payload ?? Fixtures.payload(), backend: backend)
    model.onClose = { [weak self] in self?.closed += 1 }
    return model
  }

  private func names(_ model: ContextSwitcherModel) -> [String] {
    model.rows.map { row in
      switch row {
      case .entry(let entry): return entry.name
      case .newContext(let name): return "New context “\(name)”"
      }
    }
  }

  // MARK: Rows

  func testEmptyQueryListsTheRankedEntries() throws {
    let model = try makeModel()

    XCTAssertEqual(backend.queries, [""])
    XCTAssertEqual(
      model.rows,
      [
        .entry(
          SwitcherEntry(
            key: .named(Fixtures.sugarglider),
            name: "Sugarglider",
            detail: "Ghostty · Zed · Google Chrome",
            shortcut: "⌃⌥1",
            active: true
          )
        ),
        .entry(
          SwitcherEntry(
            key: .named(Fixtures.clientWork),
            name: "Client work",
            detail: "no open windows",
            shortcut: nil,
            active: false
          )
        ),
        .entry(
          SwitcherEntry(
            key: .unsorted,
            name: "Unsorted",
            detail: "2 windows",
            shortcut: nil,
            active: false
          )
        ),
        .entry(
          SwitcherEntry(
            key: .everything,
            name: "Everything",
            detail: "show all windows",
            shortcut: "⌃⌥0",
            active: false
          )
        ),
      ]
    )
    XCTAssertNil(model.rankError)
  }

  func testRowShowsAtMostThreeAppsAndTheBareNumberWithoutAHotkey() throws {
    var payload = try Fixtures.payload()
    payload.contexts[0].apps = ["Ghostty", "Zed", "Google Chrome", "Mail"]
    payload.contexts[0].hotkey = nil
    let model = try makeModel(payload)

    guard case .entry(let entry) = model.rows[0] else { return XCTFail("\(model.rows)") }
    XCTAssertEqual(entry.detail, "Ghostty · Zed · Google Chrome")
    XCTAssertEqual(entry.shortcut, "1")
  }

  func testRowsSkipRankedIdsThatThePayloadLacks() throws {
    backend.ranks[""] = [
      SwitcherRankedEntry(key: .named(SwitcherContextId(id: 99)), match: .emptyQuery),
      SwitcherRankedEntry(key: .everything, match: .emptyQuery),
    ]
    let model = try makeModel()
    XCTAssertEqual(names(model), ["Everything"])
  }

  func testMissingRankShowsAnErrorAndListsThePayloadUnfiltered() throws {
    backend.rankError = SwitcherBridgeError(message: "sugarglider_rank_contexts is missing.")
    let model = try makeModel()

    XCTAssertEqual(model.rankError, "Search is unavailable. sugarglider_rank_contexts is missing.")
    XCTAssertEqual(names(model), ["Sugarglider", "Client work", "Unsorted", "Everything"])

    model.query = "Work"
    XCTAssertEqual(
      names(model),
      ["Sugarglider", "Client work", "Unsorted", "Everything", "New context “Work”"]
    )
  }

  func testFallbackLeavesOutUnsortedWithoutWindows() throws {
    backend.rankError = SwitcherBridgeError(message: "missing")
    var payload = try Fixtures.payload()
    payload.unsorted.windows = 0
    let model = try makeModel(payload)
    XCTAssertEqual(names(model), ["Sugarglider", "Client work", "Everything"])
  }

  // MARK: The "New context" row

  func testNewContextRowAppearsWhenNoNameMatchesExactly() throws {
    let model = try makeModel()
    model.query = "cli"
    XCTAssertEqual(names(model), ["Client work", "New context “cli”"])
  }

  func testNewContextRowIsHiddenWhenANameMatchesExactly() throws {
    backend.ranks["client work"] = [
      SwitcherRankedEntry(key: .named(Fixtures.clientWork), match: .exact)
    ]
    let model = try makeModel()
    model.query = "client work"
    XCTAssertEqual(names(model), ["Client work"])
  }

  func testNewContextRowUsesTheTrimmedQueryAndNeedsText() throws {
    let model = try makeModel()
    model.query = "   "
    XCTAssertFalse(model.rows.contains(.newContext("")))
    XCTAssertEqual(model.rows.count, 0)

    model.query = "  Deep work "
    XCTAssertEqual(model.rows, [.newContext("Deep work")])
    XCTAssertEqual(backend.queries.last, "  Deep work ")
  }

  func testFirstTimeListIsEmptyUntilTheUserTypes() throws {
    var payload = try Fixtures.payload()
    payload.contexts = []
    payload.everything.active = true
    backend.ranks[""] = [SwitcherRankedEntry(key: .everything, match: .emptyQuery)]
    backend.ranks["every"] = [SwitcherRankedEntry(key: .everything, match: .namePrefix)]
    let model = try makeModel(payload)

    XCTAssertTrue(model.isFirstTime)
    XCTAssertEqual(model.rows, [])

    model.query = "every"
    XCTAssertEqual(model.rows, [.newContext("every")])
  }

  func testNoNamedContextsWithUnsortedActiveIsNotTheFirstTime() throws {
    var payload = try Fixtures.payload()
    payload.contexts = []
    payload.unsorted.active = true
    backend.ranks[""] = [
      SwitcherRankedEntry(key: .unsorted, match: .emptyQuery),
      SwitcherRankedEntry(key: .everything, match: .emptyQuery),
    ]
    let model = try makeModel(payload)

    XCTAssertFalse(model.isFirstTime)
    XCTAssertEqual(names(model), ["Unsorted", "Everything"])
  }

  // MARK: Highlight

  func testHighlightStartsAfterTheActiveContextForAnEmptyQuery() throws {
    let model = try makeModel()
    XCTAssertEqual(model.highlight, 1)
  }

  func testHighlightStartsOnTheFirstRowWhenTheFirstRowIsNotActive() throws {
    var payload = try Fixtures.payload()
    payload.contexts[0].active = false
    payload.everything.active = true
    let model = try makeModel(payload)
    XCTAssertEqual(model.highlight, 0)
  }

  func testHighlightStartsOnTheFirstRowForAQuery() throws {
    let model = try makeModel()
    model.query = "cli"
    XCTAssertEqual(model.highlight, 0)
  }

  func testArrowsMoveTheHighlightAndStopAtTheEnds() throws {
    let model = try makeModel()

    XCTAssertTrue(model.handle(.down))
    XCTAssertEqual(model.highlight, 2)
    model.handle(.down)
    model.handle(.down)
    XCTAssertEqual(model.highlight, 3)

    model.handle(.up)
    model.handle(.up)
    model.handle(.up)
    model.handle(.up)
    XCTAssertEqual(model.highlight, 0)
  }

  func testTypingResetsTheHighlight() throws {
    let model = try makeModel()
    model.handle(.down)
    model.query = "cli"
    XCTAssertEqual(model.highlight, 0)
    model.query = ""
    XCTAssertEqual(model.highlight, 1)
  }

  func testArrowsDoNothingOnAnEmptyList() throws {
    let model = try makeModel()
    model.query = "   "
    XCTAssertTrue(model.handle(.down))
    XCTAssertEqual(model.highlight, 0)
    XCTAssertTrue(model.handle(.enter))
    XCTAssertEqual(backend.sent, [])
  }

  // MARK: Keys to commands

  func testEnterSwitchesToTheHighlightedEntryAndCloses() throws {
    let model = try makeModel()
    model.handle(.enter)
    XCTAssertEqual(backend.sent, [.switchTo(.named(Fixtures.clientWork))])
    XCTAssertEqual(closed, 1)
  }

  func testEnterSwitchesToUnsortedAndEverything() throws {
    let model = try makeModel()
    model.handle(.down)
    model.handle(.enter)
    model.handle(.down)
    model.handle(.enter)
    XCTAssertEqual(backend.sent, [.switchTo(.unsorted), .switchTo(.everything)])
  }

  func testClickingARowActsLikeEnter() throws {
    let model = try makeModel()
    model.click(row: 3)
    XCTAssertEqual(model.highlight, 3)
    XCTAssertEqual(backend.sent, [.switchTo(.everything)])
  }

  func testCommandEnterAddsTheTargetWindowToTheHighlightedContext() throws {
    let model = try makeModel()
    model.handle(.commandEnter)
    XCTAssertEqual(
      backend.sent,
      [.addWindow(window: Fixtures.chrome, context: Fixtures.clientWork)]
    )
    XCTAssertEqual(closed, 1)
  }

  func testShiftCommandEnterMovesTheTargetWindow() throws {
    let model = try makeModel()
    model.handle(.shiftCommandEnter)
    XCTAssertEqual(
      backend.sent,
      [.moveWindow(window: Fixtures.chrome, context: Fixtures.clientWork)]
    )
  }

  func testTheTargetWindowComesFromThePayloadNotTheHighlight() throws {
    var payload = try Fixtures.payload()
    payload.targetWindow = Fixtures.slack
    let model = try makeModel(payload)
    model.handle(.up)
    model.handle(.commandEnter)
    XCTAssertEqual(
      backend.sent,
      [.addWindow(window: Fixtures.slack, context: Fixtures.sugarglider)]
    )
    XCTAssertEqual(model.targetWindow?.app, "Slack")
  }

  func testWindowsCantBeAddedOrMovedToBuiltInEntries() throws {
    let model = try makeModel()
    model.handle(.down)
    model.handle(.commandEnter)
    XCTAssertEqual(model.message, "Windows can't be added to Unsorted.")
    model.handle(.down)
    model.handle(.shiftCommandEnter)
    XCTAssertEqual(model.message, "Windows can't be moved to Everything.")
    XCTAssertEqual(backend.sent, [])
    XCTAssertEqual(closed, 0)
  }

  func testWindowCommandsNeedATargetWindow() throws {
    var payload = try Fixtures.payload()
    payload.targetWindow = nil
    let model = try makeModel(payload)
    model.handle(.commandEnter)
    XCTAssertEqual(model.message, "No window had focus when the switcher opened.")
    model.handle(.commandP)
    XCTAssertEqual(backend.sent, [])
    XCTAssertNil(model.targetWindow)
  }

  func testNamedContextCommandsOnTheNewContextRowAskToCreateItFirst() throws {
    let model = try makeModel()
    model.query = "Deep work"
    for key: SwitcherKey in [.commandEnter, .commandE, .commandR, .commandDigit(2), .commandDelete]
    {
      model.handle(key)
      XCTAssertEqual(model.message, "Press ↩ to create “Deep work” first.", "\(key)")
    }
    XCTAssertEqual(backend.sent, [])
    XCTAssertEqual(model.mode, .list)
  }

  func testCommandDigitNumbersTheHighlightedContext() throws {
    let model = try makeModel()
    model.handle(.commandDigit(3))
    XCTAssertEqual(backend.sent, [.setNumber(context: Fixtures.clientWork, number: 3)])
  }

  func testBuiltInEntriesCantBeNumberedRenamedEditedOrDeleted() throws {
    let model = try makeModel()
    model.handle(.down)
    model.handle(.commandDigit(3))
    XCTAssertEqual(model.message, "Unsorted can't be numbered.")
    model.handle(.commandR)
    XCTAssertEqual(model.message, "Unsorted can't be renamed.")
    model.handle(.commandE)
    XCTAssertEqual(model.message, "Unsorted can't be edited.")
    model.handle(.down)
    model.handle(.commandDelete)
    XCTAssertEqual(model.message, "Everything can't be deleted.")
    XCTAssertEqual(backend.sent, [])
    XCTAssertEqual(model.mode, .list)
  }

  func testCommandPTogglesPinningOfTheTargetWindow() throws {
    let model = try makeModel()
    model.handle(.commandP)
    XCTAssertEqual(backend.sent, [.togglePinned(window: Fixtures.chrome)])
  }

  func testDeleteAsksInlineAndEnterConfirms() throws {
    let model = try makeModel()
    model.handle(.commandDelete)
    XCTAssertEqual(model.mode, .confirmDelete(Fixtures.clientWork))
    XCTAssertEqual(backend.sent, [])

    XCTAssertTrue(model.handle(.down))
    XCTAssertEqual(model.mode, .confirmDelete(Fixtures.clientWork))
    model.handle(.enter)
    XCTAssertEqual(backend.sent, [.delete(Fixtures.clientWork)])
    XCTAssertEqual(closed, 1)
  }

  func testEscapeCancelsTheDeleteConfirmation() throws {
    let model = try makeModel()
    model.handle(.commandDelete)
    model.handle(.escape)
    XCTAssertEqual(model.mode, .list)
    XCTAssertEqual(backend.sent, [])
    XCTAssertEqual(closed, 0)
  }

  func testTypingCancelsTheDeleteConfirmation() throws {
    let model = try makeModel()
    model.handle(.commandDelete)
    model.query = "cli"
    XCTAssertEqual(model.mode, .list)
    model.handle(.enter)
    XCTAssertEqual(backend.sent, [.switchTo(.named(Fixtures.clientWork))])
  }

  func testRenameSendsTheTrimmedName() throws {
    let model = try makeModel()
    model.handle(.commandR)
    XCTAssertEqual(model.mode, .rename(Fixtures.clientWork))
    XCTAssertEqual(model.nameDraft, "Client work")

    model.nameDraft = "  Clients "
    model.handle(.enter)
    XCTAssertEqual(backend.sent, [.rename(context: Fixtures.clientWork, name: "Clients")])
  }

  func testRenameToTheSameNameSendsNothing() throws {
    let model = try makeModel()
    model.handle(.commandR)
    model.handle(.enter)
    XCTAssertEqual(model.mode, .list)
    XCTAssertEqual(backend.sent, [])
  }

  func testRenameRejectsABlankName() throws {
    let model = try makeModel()
    model.handle(.commandR)
    model.nameDraft = "  "
    model.handle(.enter)
    XCTAssertEqual(model.message, "A context name can't be empty.")
    XCTAssertEqual(model.mode, .rename(Fixtures.clientWork))
    XCTAssertEqual(backend.sent, [])
  }

  func testNameModesLeaveOtherKeysToTheTextField() throws {
    let model = try makeModel()
    model.handle(.commandR)
    for key: SwitcherKey in [.up, .down, .space, .commandDelete, .commandN, .commandDigit(1)] {
      XCTAssertFalse(model.handle(key), "\(key)")
    }
    XCTAssertEqual(model.mode, .rename(Fixtures.clientWork))
  }

  func testSpaceInTheListBelongsToTheTextField() throws {
    let model = try makeModel()
    let rows = model.rows
    XCTAssertFalse(model.handle(.space))
    XCTAssertEqual(model.rows, rows)
    XCTAssertEqual(model.highlight, 1)
    XCTAssertEqual(model.mode, .list)
  }

  func testEscapeInTheListCloses() throws {
    let model = try makeModel()
    XCTAssertTrue(model.handle(.escape))
    XCTAssertEqual(closed, 1)
    XCTAssertEqual(backend.sent, [])
  }

  func testRejectedCommandShowsTheErrorAndStaysOpen() throws {
    backend.runError = SwitcherBridgeError(message: "No such context")
    let model = try makeModel()
    model.handle(.enter)
    XCTAssertEqual(model.message, "No such context")
    XCTAssertEqual(closed, 0)
    XCTAssertEqual(model.mode, .list)

    model.query = "c"
    XCTAssertNil(model.message)
  }

  // MARK: Create

  func testEnterOnTheNewContextRowListsTheWindowsOnScreenAllChecked() throws {
    let model = try makeModel()
    model.query = "cli"
    model.handle(.down)
    model.handle(.enter)

    XCTAssertEqual(model.mode, .create(name: "cli"))
    XCTAssertEqual(
      model.checklist.map(\.source),
      [Fixtures.ghostty, Fixtures.chrome, Fixtures.whatsApp, Fixtures.finder, Fixtures.slack]
        .map { .window($0) }
    )
    XCTAssertTrue(model.checklist.allSatisfy(\.checked))
    XCTAssertEqual(backend.sent, [])
  }

  func testCreateSendsTheCheckedWindows() throws {
    let model = try makeModel()
    model.query = "Sugar glider"
    model.handle(.commandN)
    XCTAssertEqual(model.mode, .create(name: "Sugar glider"))

    model.handle(.down)
    model.handle(.down)
    XCTAssertTrue(model.handle(.space))
    model.toggleItem(at: 4)
    model.handle(.enter)

    XCTAssertEqual(
      backend.sent,
      [
        .create(
          name: "Sugar glider", windows: [Fixtures.ghostty, Fixtures.chrome, Fixtures.finder])
      ]
    )
    XCTAssertEqual(closed, 1)
  }

  func testCommandNWithoutAQueryAsksForAName() throws {
    let model = try makeModel()
    model.handle(.commandN)
    XCTAssertEqual(model.mode, .naming)

    model.handle(.enter)
    XCTAssertEqual(model.message, "A context name can't be empty.")

    model.nameDraft = " Research "
    model.handle(.enter)
    XCTAssertEqual(model.mode, .create(name: "Research"))
    model.handle(.enter)
    XCTAssertEqual(
      backend.sent,
      [
        .create(
          name: "Research",
          windows: [
            Fixtures.ghostty, Fixtures.chrome, Fixtures.whatsApp, Fixtures.finder, Fixtures.slack,
          ]
        )
      ]
    )
  }

  func testEscapeFromTheCreateViewReturnsToTheListWithTheQuery() throws {
    let model = try makeModel()
    model.query = "cli"
    model.handle(.commandN)
    model.handle(.escape)
    XCTAssertEqual(model.mode, .list)
    XCTAssertEqual(model.query, "cli")
    XCTAssertEqual(names(model), ["Client work", "New context “cli”"])
    XCTAssertEqual(closed, 0)
  }

  func testChecklistHighlightStopsAtTheEnds() throws {
    let model = try makeModel()
    model.query = "x"
    model.handle(.commandN)
    model.handle(.up)
    XCTAssertEqual(model.checklistHighlight, 0)
    for _ in 0..<10 { model.handle(.down) }
    XCTAssertEqual(model.checklistHighlight, 4)
  }

  // MARK: Edit

  func testEditListsWindowsOnScreenThenOtherMembers() throws {
    let model = try makeModel()
    model.handle(.up)
    model.handle(.commandE)

    XCTAssertEqual(model.mode, .edit(Fixtures.sugarglider))
    XCTAssertEqual(
      model.checklist.map(\.source),
      [
        .window(Fixtures.ghostty), .window(Fixtures.chrome), .window(Fixtures.whatsApp),
        .window(Fixtures.finder), .window(Fixtures.slack), .window(Fixtures.zed),
        .record(SwitcherRecordRef(record: 3, app: "Mail", title: "Inbox")),
      ]
    )
    XCTAssertEqual(model.checklist.map(\.checked), [true, true, false, false, false, true, true])
    XCTAssertEqual(model.checklist[6].title, "Inbox")
  }

  func testEditSendsOnlyWhatChanged() throws {
    let model = try makeModel()
    model.handle(.up)
    model.handle(.commandE)
    model.toggleItem(at: 0)
    model.toggleItem(at: 3)
    model.toggleItem(at: 4)
    model.toggleItem(at: 4)
    model.toggleItem(at: 5)
    model.toggleItem(at: 6)
    model.handle(.enter)

    XCTAssertEqual(
      backend.sent,
      [
        .edit(
          context: Fixtures.sugarglider,
          add: [Fixtures.finder],
          remove: [Fixtures.ghostty, Fixtures.zed],
          removeRecords: [SwitcherRecordRef(record: 3, app: "Mail", title: "Inbox")]
        )
      ]
    )
    XCTAssertEqual(closed, 1)
  }

  func testEditWithoutChangesClosesWithoutACommand() throws {
    let model = try makeModel()
    model.handle(.commandE)
    XCTAssertEqual(model.mode, .edit(Fixtures.clientWork))
    XCTAssertEqual(model.checklist.map(\.checked), [false, false, false, false, false])
    model.handle(.enter)
    XCTAssertEqual(backend.sent, [])
    XCTAssertEqual(closed, 1)
  }

  // MARK: Key mapping

  private func key(
    _ keyCode: UInt16,
    _ modifiers: NSEvent.ModifierFlags = [],
    _ characters: String? = nil
  ) -> SwitcherKey? {
    SwitcherKey(keyCode: keyCode, modifiers: modifiers, characters: characters)
  }

  func testKeysMapToSwitcherKeys() {
    XCTAssertEqual(key(126, [.numericPad, .function]), .up)
    XCTAssertEqual(key(125, [.numericPad, .function]), .down)
    XCTAssertEqual(key(36, [], "\r"), .enter)
    XCTAssertEqual(key(76, [], "\u{3}"), .enter)
    XCTAssertEqual(key(36, [.command], "\r"), .commandEnter)
    XCTAssertEqual(key(36, [.command, .shift], "\r"), .shiftCommandEnter)
    XCTAssertEqual(key(53, [], "\u{1b}"), .escape)
    XCTAssertEqual(key(49, [], " "), .space)
    XCTAssertEqual(key(51, [.command], "\u{7f}"), .commandDelete)
    XCTAssertEqual(key(45, [.command], "n"), .commandN)
    XCTAssertEqual(key(14, [.command], "e"), .commandE)
    XCTAssertEqual(key(15, [.command], "r"), .commandR)
    XCTAssertEqual(key(35, [.command], "p"), .commandP)
    XCTAssertEqual(key(45, [.command, .capsLock], "N"), .commandN)
    for (digit, keyCode) in [
      (1, 18), (2, 19), (3, 20), (4, 21), (5, 23), (6, 22), (7, 26), (8, 28), (9, 25),
    ] {
      XCTAssertEqual(key(UInt16(keyCode), [.command], "\(digit)"), .commandDigit(digit))
    }
  }

  func testKeysForOtherLayoutsMapByKeyCode() {
    XCTAssertEqual(key(18, [.command], "&"), .commandDigit(1))
    XCTAssertEqual(key(25, [.command], "ç"), .commandDigit(9))
    XCTAssertEqual(key(45, [.command], "т"), .commandN)
    XCTAssertEqual(key(11, [.command], "n"), .commandN)
  }

  func testKeysTheSwitcherLeavesAlone() {
    XCTAssertNil(key(51, [], "\u{7f}"))
    XCTAssertNil(key(0, [], "a"))
    XCTAssertNil(key(29, [.command], "0"))
    XCTAssertNil(key(45, [.command, .option], "n"))
    XCTAssertNil(key(45, [.control], "n"))
    XCTAssertNil(key(36, [.shift], "\r"))
    XCTAssertNil(key(36, [.option], "\r"))
    XCTAssertNil(key(126, [.command]))
    XCTAssertNil(key(49, [.command], " "))
    XCTAssertNil(key(0, [.command], "a"))
  }
}
