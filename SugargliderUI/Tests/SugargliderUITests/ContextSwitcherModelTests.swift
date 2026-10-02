// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import AppKit
import XCTest

@testable import SugargliderUI

/// Records commands and answers rank queries without Rust.
///
/// Like `model::contexts::rank`, it trims the query first, so a blank query
/// gets the entries of the empty one. `ranks` is keyed by the trimmed query.
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
    return ranks[query.trimmingCharacters(in: .whitespacesAndNewlines)] ?? []
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
    XCTAssertEqual(names(model), ["Sugarglider", "Client work", "Unsorted", "Everything"])
    XCTAssertEqual(model.highlight, 1)

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
    var payload = try Fixtures.payload()
    payload.contexts = []
    payload.everything.active = true
    backend.ranks[""] = [SwitcherRankedEntry(key: .everything, match: .emptyQuery)]
    let model = try makeModel(payload)
    XCTAssertEqual(model.rows, [])
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

  /// A click on a row acts only in the list, not under the delete
  /// confirmation or in the rename view.
  func testClickingARowOutsideTheListDoesNothing() throws {
    let model = try makeModel()
    model.handle(.commandDelete)
    model.click(row: 3)
    XCTAssertEqual(model.mode, .confirmDelete(Fixtures.clientWork))
    XCTAssertEqual(model.highlight, 1)

    model.handle(.escape)
    model.handle(.commandR)
    model.click(row: 3)
    XCTAssertEqual(model.mode, .rename(Fixtures.clientWork))
    XCTAssertEqual(model.highlight, 1)
    XCTAssertEqual(backend.sent, [])
    XCTAssertEqual(closed, 0)
  }

  /// The delete confirmation takes only ↩ and Esc. Space is used up, so it
  /// neither types into the search field nor cancels the confirmation.
  func testSpaceDuringTheDeleteConfirmationDoesNothing() throws {
    let model = try makeModel()
    model.handle(.commandDelete)
    XCTAssertTrue(model.handle(.space))
    XCTAssertEqual(model.mode, .confirmDelete(Fixtures.clientWork))
    XCTAssertEqual(model.query, "")
    XCTAssertEqual(backend.sent, [])
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

  /// C2: Rust sends no target window when the focused window is untracked,
  /// parked, or Sugarglider's own. The window keys are then off, and the
  /// panel says why.
  func testWindowCommandsNeedATargetWindow() throws {
    XCTAssertTrue(try makeModel().hasTargetWindow)

    var payload = try Fixtures.payload()
    payload.targetWindow = nil
    let model = try makeModel(payload)
    XCTAssertFalse(model.hasTargetWindow)
    model.handle(.commandEnter)
    XCTAssertEqual(
      model.message,
      "No window to add, move, or pin. Focus a window that Sugarglider manages first."
    )
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
    XCTAssertEqual(model.mode, .naming)
    XCTAssertEqual(model.nameDraft, "Sugar glider")
    model.handle(.enter)
    XCTAssertEqual(model.mode, .create(name: "Sugar glider"))

    model.handle(.down)
    model.handle(.down)
    model.handle(.down)
    XCTAssertTrue(model.handle(.space))
    model.toggleItem(at: 4)
    model.handle(.enter)

    XCTAssertEqual(
      backend.sent,
      [.create(name: "Sugar glider", windows: [Fixtures.ghostty, Fixtures.chrome])]
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
          windows: [Fixtures.ghostty, Fixtures.chrome, Fixtures.finder, Fixtures.slack]
        )
      ]
    )
  }

  func testEscapeFromTheCreateViewReturnsToTheListWithTheQuery() throws {
    let model = try makeModel()
    model.query = "cli"
    model.handle(.commandN)
    model.handle(.enter)
    XCTAssertEqual(model.mode, .create(name: "cli"))
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
    model.handle(.enter)
    XCTAssertEqual(model.mode, .create(name: "x"))
    model.handle(.up)
    XCTAssertEqual(model.checklistHighlight, 0)
    for _ in 0..<10 { model.handle(.down) }
    XCTAssertEqual(model.checklistHighlight, 4)
  }

  // MARK: Naming a new context

  /// ⌘N fills the name with the trimmed query, and the user can change it
  /// before choosing the windows.
  func testCommandNFillsTheNameWithTheQueryToEdit() throws {
    let model = try makeModel()
    model.query = "  Deep work "
    model.handle(.commandN)
    XCTAssertEqual(model.mode, .naming)
    XCTAssertEqual(model.nameDraft, "Deep work")
    XCTAssertNil(model.message)

    model.nameDraft = "Deep work 2"
    model.handle(.enter)
    XCTAssertEqual(model.mode, .create(name: "Deep work 2"))
    model.handle(.enter)
    XCTAssertEqual(
      backend.sent,
      [
        .create(
          name: "Deep work 2",
          windows: [Fixtures.ghostty, Fixtures.chrome, Fixtures.finder, Fixtures.slack])
      ]
    )
  }

  /// R4: ⌘N with the name of a context, in any case or accents, says the
  /// name is taken, and ↩ waits for another name.
  func testCommandNWithATakenNameAsksForAnother() throws {
    backend.ranks["client work"] = [
      SwitcherRankedEntry(key: .named(Fixtures.clientWork), match: .exact)
    ]
    let model = try makeModel()
    model.query = "client work"
    XCTAssertEqual(names(model), ["Client work"])

    model.handle(.commandN)
    XCTAssertEqual(model.mode, .naming)
    XCTAssertEqual(model.nameDraft, "client work")
    XCTAssertEqual(model.message, "A context named “Client work” already exists.")
    model.handle(.enter)
    XCTAssertEqual(model.mode, .naming)

    model.nameDraft = "Clïent Wörk"
    XCTAssertNil(model.message)
    model.handle(.enter)
    XCTAssertEqual(model.message, "A context named “Client work” already exists.")
    XCTAssertEqual(model.mode, .naming)

    model.nameDraft = "Client work 2"
    model.handle(.enter)
    XCTAssertEqual(model.mode, .create(name: "Client work 2"))
    XCTAssertEqual(backend.sent, [])
  }

  /// R4: "Everything" and "Unsorted" are reserved in any case or accents.
  func testReservedNamesAreRefusedInAnyCaseOrAccents() throws {
    let model = try makeModel()
    for name in ["everything", "  EVERYTHING ", "Évérything"] {
      XCTAssertEqual(model.newNameProblem(name), "“Everything” is a reserved name.", name)
    }
    for name in ["unsorted", "Ünsorted", "UNSORTED"] {
      XCTAssertEqual(model.newNameProblem(name), "“Unsorted” is a reserved name.", name)
    }
    XCTAssertEqual(model.newNameProblem(" \t"), "A context name can't be empty.")
    XCTAssertNil(model.newNameProblem("Everything else"))
    XCTAssertNil(model.newNameProblem("Unsorted mail"))
  }

  /// The "New context" row appears only for a name that can be used, also
  /// when ranking is unavailable and no rank entry says `exact`.
  func testTheNewContextRowNeedsANameThatCanBeUsed() throws {
    backend.rankError = SwitcherBridgeError(message: "missing")
    let model = try makeModel()
    for query in ["CLIENT WORK", "sugarglïder", "Unsorted", "everything"] {
      model.query = query
      XCTAssertFalse(model.rows.contains { $0 == .newContext(query) }, query)
      XCTAssertEqual(model.rows.count, 4, query)
    }
    model.query = "Clients"
    XCTAssertEqual(model.rows.last, .newContext("Clients"))
  }

  /// When Rust rejects a create, for example because the CLI made a
  /// context with that name since the panel opened, the naming view comes
  /// back with the name and Rust's message. The checked windows stay as
  /// the user left them.
  func testARejectedCreateReturnsToNamingAndKeepsTheChecklist() throws {
    backend.runError = SwitcherBridgeError(message: "A context named \"Deep work\" already exists")
    let model = try makeModel()
    model.query = "Deep work"
    XCTAssertEqual(model.rows, [.newContext("Deep work")])
    model.handle(.enter)
    XCTAssertEqual(model.mode, .create(name: "Deep work"))
    model.toggleItem(at: 3)
    model.handle(.enter)

    XCTAssertEqual(model.mode, .naming)
    XCTAssertEqual(model.nameDraft, "Deep work")
    XCTAssertEqual(model.message, "A context named \"Deep work\" already exists")
    XCTAssertEqual(closed, 0)

    model.nameDraft = "Deep work 2"
    XCTAssertNil(model.message)
    model.handle(.enter)
    XCTAssertEqual(model.mode, .create(name: "Deep work 2"))
    XCTAssertEqual(model.checklist.map(\.checked), [true, true, true, false, true])

    backend.runError = nil
    model.handle(.enter)
    XCTAssertEqual(
      backend.sent.last,
      .create(name: "Deep work 2", windows: [Fixtures.ghostty, Fixtures.chrome, Fixtures.slack])
    )
    XCTAssertEqual(closed, 1)
  }

  /// Esc from the naming view drops the checklist of a rejected create.
  func testEscapeAfterARejectedCreateForgetsItsChecklist() throws {
    backend.runError = SwitcherBridgeError(message: "Contexts are turned off")
    let model = try makeModel()
    model.query = "Deep work"
    model.handle(.enter)
    model.toggleItem(at: 0)
    model.handle(.enter)
    XCTAssertEqual(model.mode, .naming)

    model.handle(.escape)
    XCTAssertEqual(model.mode, .list)
    model.handle(.enter)
    XCTAssertEqual(model.mode, .create(name: "Deep work"))
    XCTAssertTrue(model.checklist.allSatisfy(\.checked))
  }

  /// Names compare lowercased and without the accents of Latin letters, as
  /// Rust's `fold` compares them. Other scripts keep their letters, so the
  /// panel never refuses a name that Rust takes.
  func testFoldComparesNamesAsRustDoes() {
    XCTAssertEqual(ContextSwitcherModel.fold("Clïent Wörk"), "client work")
    XCTAssertEqual(ContextSwitcherModel.fold("Straße"), "strasse")
    XCTAssertEqual(ContextSwitcherModel.fold("Æther Łódź"), "aether lodz")
    XCTAssertEqual(ContextSwitcherModel.fold("İstanbul"), "istanbul")
    XCTAssertEqual(ContextSwitcherModel.fold("Cafe\u{0301}"), "cafe")
    XCTAssertEqual(ContextSwitcherModel.fold("Việt"), "viet")
    XCTAssertEqual(ContextSwitcherModel.fold("2×3"), "2×3")
    XCTAssertEqual(ContextSwitcherModel.fold("Ёлка"), "ёлка")
    XCTAssertNotEqual(ContextSwitcherModel.fold("Мой"), ContextSwitcherModel.fold("Мои"))
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
    XCTAssertEqual(model.checklist.map(\.checked), [true, true, true, false, false, true, true])
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

  /// The edit view checks a window on screen when one of the context's
  /// records has it. The records are the only membership in the payload.
  func testEditChecksTheWindowsThatTheContextsRecordsHave() throws {
    var payload = try Fixtures.payload()
    payload.contexts[1].members = [
      SwitcherMember(record: 0, app: "Slack", title: "general", window: Fixtures.slack)
    ]
    let model = try makeModel(payload)
    model.handle(.commandE)
    XCTAssertEqual(model.mode, .edit(Fixtures.clientWork))
    XCTAssertEqual(
      model.checklist.map(\.source),
      [Fixtures.ghostty, Fixtures.chrome, Fixtures.whatsApp, Fixtures.finder, Fixtures.slack]
        .map { .window($0) }
    )
    XCTAssertEqual(model.checklist.map(\.checked), [false, false, true, false, true])

    model.toggleItem(at: 4)
    model.handle(.enter)
    XCTAssertEqual(
      backend.sent,
      [.edit(context: Fixtures.clientWork, add: [], remove: [Fixtures.slack], removeRecords: [])]
    )
  }

  // MARK: Tab groups

  /// R36: a native tab group is one row, named by its main tab, with its
  /// number of tabs. Commands carry the main tab, and Rust applies them to
  /// the whole group.
  func testATabGroupIsOneRowWithItsTabCount() throws {
    let model = try makeModel()
    XCTAssertEqual(model.targetWindow?.detail, "Google Chrome (3 tabs)")

    model.handle(.up)
    model.handle(.commandE)
    XCTAssertEqual(
      model.checklist.map(\.detail),
      ["Ghostty", "Google Chrome (3 tabs)", "WhatsApp", "Finder", "Slack", "Zed", "Mail"]
    )
    XCTAssertEqual(model.checklist.map(\.tabCount), [1, 3, 1, 1, 1, 1, 1])

    model.toggleItem(at: 1)
    model.handle(.enter)
    XCTAssertEqual(
      backend.sent,
      [.edit(context: Fixtures.sugarglider, add: [], remove: [Fixtures.chrome], removeRecords: [])]
    )
  }

  func testTheCreateViewShowsTheTabsOfAGroup() throws {
    let model = try makeModel()
    model.query = "Tabs"
    model.handle(.enter)
    XCTAssertEqual(model.mode, .create(name: "Tabs"))
    XCTAssertEqual(model.checklist[1].source, .window(Fixtures.chrome))
    XCTAssertEqual(model.checklist[1].detail, "Google Chrome (3 tabs)")
  }

  // MARK: Pinned windows

  /// R3: a pinned window is a member of every context. The create view
  /// shows it checked and fixed, and `create` leaves it out, so it gets no
  /// record.
  func testTheCreateViewShowsAPinnedWindowCheckedAndLeavesItOut() throws {
    let model = try makeModel()
    model.query = "cli"
    model.handle(.down)
    model.handle(.enter)
    XCTAssertEqual(model.checklist.map(\.pinned), [false, false, true, false, false])
    XCTAssertTrue(model.checklist[2].checked)

    model.handle(.down)
    model.handle(.down)
    XCTAssertTrue(model.handle(.space))
    model.toggleItem(at: 2)
    XCTAssertTrue(model.checklist[2].checked)
    XCTAssertEqual(model.checklistHighlight, 2)
    model.handle(.enter)
    XCTAssertEqual(
      backend.sent,
      [
        .create(
          name: "cli",
          windows: [Fixtures.ghostty, Fixtures.chrome, Fixtures.finder, Fixtures.slack])
      ]
    )
  }

  /// R3: the edit view shows a pinned window checked and fixed, whether or
  /// not the context has a record of it, and `edit` never adds or removes
  /// it.
  func testTheEditViewShowsAPinnedWindowCheckedAndNeverSendsIt() throws {
    for records in [
      [],
      [SwitcherMember(record: 0, app: "WhatsApp", title: "WhatsApp", window: Fixtures.whatsApp)],
    ] {
      var payload = try Fixtures.payload()
      payload.contexts[1].members = records
      let model = try makeModel(payload)
      model.handle(.commandE)
      XCTAssertEqual(model.checklist[2].source, .window(Fixtures.whatsApp))
      XCTAssertTrue(model.checklist[2].checked)
      XCTAssertTrue(model.checklist[2].pinned)

      model.toggleItem(at: 2)
      XCTAssertTrue(model.checklist[2].checked)
      model.toggleItem(at: 3)
      model.handle(.enter)
      XCTAssertEqual(
        backend.sent.last,
        .edit(context: Fixtures.clientWork, add: [Fixtures.finder], remove: [], removeRecords: [])
      )
    }
  }

  func testEditWithoutChangesClosesWithoutACommand() throws {
    let model = try makeModel()
    model.handle(.commandE)
    XCTAssertEqual(model.mode, .edit(Fixtures.clientWork))
    XCTAssertEqual(model.checklist.map(\.checked), [false, false, true, false, false])
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
