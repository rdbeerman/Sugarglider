// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import XCTest

@testable import SugargliderUI

/// The switcher's list: the rows, the "New context" row, the highlight, and
/// the first-time and flag-off states (spec, "Switcher").
///
/// Each rank result stands for what `model::contexts::rank` returns for the
/// query: it trims the query, compares folded names, lists Unsorted only
/// when it has windows, and orders ties by most recent use.
@MainActor
final class SwitcherListSpecTests: SwitcherSpecTestCase {
  private let sugarglider = SwitcherEntry(
    key: .named(Fixtures.sugarglider),
    name: "Sugarglider",
    detail: "Ghostty · Zed · Google Chrome",
    shortcut: "⌃⌥1",
    active: true
  )
  private let clientWork = SwitcherEntry(
    key: .named(Fixtures.clientWork),
    name: "Client work",
    detail: "no open windows",
    shortcut: nil,
    active: false
  )
  private let unsorted = SwitcherEntry(
    key: .unsorted,
    name: "Unsorted",
    detail: "2 windows",
    shortcut: nil,
    active: false
  )
  private let everything = SwitcherEntry(
    key: .everything,
    name: "Everything",
    detail: "show all windows",
    shortcut: "⌃⌥0",
    active: false
  )

  // MARK: Rows

  /// Typing filters the list. The rows follow the rank result's order, not
  /// the payload's.
  func testRowsFollowTheRankOrderNotThePayloadOrder() throws {
    var payload = try Fixtures.payload()
    payload.contexts[0].active = false
    payload.everything.active = true
    backend.ranks[""] = SpecPayload.ranked([
      (.everything, .emptyQuery), (.unsorted, .emptyQuery),
      (.named(Fixtures.clientWork), .emptyQuery), (.named(Fixtures.sugarglider), .emptyQuery),
    ])
    let model = try makeModel(payload)
    XCTAssertEqual(names(model), ["Everything", "Unsorted", "Client work", "Sugarglider"])
    XCTAssertEqual(entries(model).map(\.active), [true, false, false, false])
    XCTAssertEqual(model.highlight, 1)
  }

  /// ● marks the active context, and only it. Here Unsorted is active.
  func testOnlyTheActiveEntryIsMarkedActive() throws {
    var payload = try Fixtures.payload()
    payload.contexts[0].active = false
    payload.unsorted.active = true
    backend.ranks[""] = SpecPayload.ranked([
      (.unsorted, .emptyQuery), (.named(Fixtures.sugarglider), .emptyQuery),
      (.named(Fixtures.clientWork), .emptyQuery), (.everything, .emptyQuery),
    ])
    let model = try makeModel(payload)
    XCTAssertEqual(names(model), ["Unsorted", "Sugarglider", "Client work", "Everything"])
    XCTAssertEqual(entries(model).map(\.active), [true, false, false, false])
    XCTAssertEqual(model.highlight, 1)
  }

  /// Each row shows the name, up to three app names, and the number
  /// shortcut.
  func testRowsShowOneOrTwoAppsAndOneUnsortedWindow() throws {
    var payload = try Fixtures.payload()
    payload.contexts[0].apps = ["Ghostty"]
    payload.contexts[1].apps = ["Outlook", "Word"]
    payload.contexts[1].number = 2
    payload.unsorted.windows = 1
    payload.everything.hotkey = nil
    let model = try makeModel(payload)
    XCTAssertEqual(
      entries(model).map(\.detail),
      ["Ghostty", "Outlook · Word", "1 window", "show all windows"]
    )
    XCTAssertEqual(entries(model).map(\.shortcut), ["⌃⌥1", "2", nil, nil])
  }

  // MARK: The "New context" row

  /// R4: "Everything" is reserved. Rust ranks it as an exact match in any
  /// case, so no "New context" row appears.
  func testTheReservedNameEverythingOffersNoNewContextRow() throws {
    let model = try makeModel()
    for query in ["everything", "EVERYTHING", "  Everything "] {
      backend.ranks[query.trimmingCharacters(in: .whitespaces)] = SpecPayload.ranked([
        (.everything, .exact)
      ])
      model.query = query
      XCTAssertEqual(model.rows, [.entry(everything)], query)
      XCTAssertEqual(model.highlight, 0, query)
    }
  }

  /// R4: "Unsorted" is reserved. While Unsorted has windows it is listed,
  /// so it matches exactly and no "New context" row appears.
  func testTheReservedNameUnsortedOffersNoNewContextRowWhileListed() throws {
    backend.ranks["Unsorted"] = SpecPayload.ranked([(.unsorted, .exact)])
    let model = try makeModel()
    model.query = "Unsorted"
    XCTAssertEqual(model.rows, [.entry(unsorted)])
  }

  /// R4: without unsorted windows the rank result leaves Unsorted out, but
  /// the panel knows the reserved names, so "unsorted" offers no "New
  /// context" row, and ⌘N says why.
  func testUnsortedWithoutWindowsOffersNoNewContextRow() throws {
    var payload = try Fixtures.payload()
    payload.unsorted.windows = 0
    backend.ranks[""] = SpecPayload.ranked([
      (.named(Fixtures.sugarglider), .emptyQuery), (.named(Fixtures.clientWork), .emptyQuery),
      (.everything, .emptyQuery),
    ])
    backend.ranks["unsorted"] = []
    let model = try makeModel(payload)
    model.query = "unsorted"
    XCTAssertEqual(model.rows, [])

    model.handle(.enter)
    model.handle(.commandN)
    XCTAssertEqual(model.mode, .naming)
    XCTAssertEqual(model.message, "“Unsorted” is a reserved name.")
    model.handle(.enter)
    XCTAssertEqual(model.mode, .naming)
    XCTAssertEqual(backend.sent, [])
    XCTAssertEqual(closed, 0)
  }

  /// R4 and "Names are compared lowercased and without accents": a case or
  /// accent variant of a name is an exact match, so no "New context" row
  /// appears.
  func testCaseAndAccentVariantsOfANameOfferNoNewContextRow() throws {
    let model = try makeModel()
    for query in ["CLIENT WORK", "clïent wörk", "Clíent Work"] {
      backend.ranks[query] = SpecPayload.ranked([(.named(Fixtures.clientWork), .exact)])
      model.query = query
      XCTAssertEqual(model.rows, [.entry(clientWork)], query)
    }
  }

  /// The "New context" row carries the typed name, trimmed, with its case
  /// and accents.
  func testTheNewContextRowKeepsTheTypedCaseAndAccents() throws {
    let model = try makeModel()
    model.query = "  Café Rècherche "
    XCTAssertEqual(model.rows, [.newContext("Café Rècherche")])
    model.handle(.enter)
    XCTAssertEqual(model.mode, .create(name: "Café Rècherche"))
  }

  /// The "New context" row comes last, after every ranked entry, and the
  /// arrows reach it.
  func testTheNewContextRowComesLastAndTheArrowsReachIt() throws {
    backend.ranks["er"] = SpecPayload.ranked([
      (.named(Fixtures.sugarglider), .lettersInOrder),
      (.named(Fixtures.clientWork), .lettersInOrder),
      (.everything, .lettersInOrder),
    ])
    let model = try makeModel()
    model.query = "er"
    XCTAssertEqual(
      model.rows,
      [.entry(sugarglider), .entry(clientWork), .entry(everything), .newContext("er")]
    )
    XCTAssertEqual(model.highlight, 0)
    for _ in 0..<10 { model.handle(.down) }
    XCTAssertEqual(model.highlight, 3)
    model.handle(.enter)
    XCTAssertEqual(model.mode, .create(name: "er"))
    XCTAssertEqual(backend.sent, [])
  }

  /// A prefix of the active context's name highlights it, and ↩ switches to
  /// it again (R16).
  func testAQueryHighlightsTheActiveContextWhenItRanksFirst() throws {
    backend.ranks["sug"] = SpecPayload.ranked([(.named(Fixtures.sugarglider), .namePrefix)])
    let model = try makeModel()
    model.query = "sug"
    XCTAssertEqual(model.rows, [.entry(sugarglider), .newContext("sug")])
    XCTAssertEqual(model.highlight, 0)
    model.handle(.enter)
    XCTAssertEqual(backend.sent, [.switchTo(.named(Fixtures.sugarglider))])
  }

  // MARK: Highlight

  /// The highlight stops at both ends of a one-row list.
  func testTheHighlightStaysOnASingleRow() throws {
    backend.ranks["client work"] = SpecPayload.ranked([(.named(Fixtures.clientWork), .exact)])
    let model = try makeModel()
    model.query = "client work"
    XCTAssertEqual(model.rows, [.entry(clientWork)])
    model.handle(.down)
    model.handle(.down)
    XCTAssertEqual(model.highlight, 0)
    model.handle(.up)
    XCTAssertEqual(model.highlight, 0)
  }

  /// The highlight stops at the ends and doesn't wrap around.
  func testTheHighlightDoesNotWrapAroundTheEnds() throws {
    let model = try makeModel()
    highlight(model, row: 0)
    model.handle(.up)
    XCTAssertEqual(model.highlight, 0)
    highlight(model, row: 3)
    model.handle(.down)
    XCTAssertEqual(model.highlight, 3)
    model.handle(.enter)
    XCTAssertEqual(backend.sent, [.switchTo(.everything)])
  }

  /// With only the active context listed, it is highlighted.
  func testTheOnlyRowIsHighlightedEvenWhenItIsActive() throws {
    backend.ranks[""] = SpecPayload.ranked([(.named(Fixtures.sugarglider), .emptyQuery)])
    let model = try makeModel()
    XCTAssertEqual(model.rows, [.entry(sugarglider)])
    XCTAssertEqual(model.highlight, 0)
  }

  // MARK: First time

  /// The first time, the list is empty, even when Rust ranks Unsorted and
  /// Everything.
  func testTheFirstTimeListIsEmptyEvenWithUnsortedWindows() throws {
    backend.ranks[""] = SpecPayload.ranked([(.unsorted, .emptyQuery), (.everything, .emptyQuery)])
    let model = try makeModel(
      SpecPayload.payload(unsorted: SwitcherUnsorted(windows: 3, active: false))
    )
    XCTAssertTrue(model.isFirstTime)
    XCTAssertEqual(model.rows, [])
    XCTAssertEqual(model.highlight, 0)

    model.query = "x"
    XCTAssertEqual(model.rows, [.newContext("x")])
    model.query = ""
    XCTAssertEqual(model.rows, [])
  }

  /// The first time, typing the reserved name "Everything" offers nothing
  /// to create (R4), and ↩ does nothing.
  func testTheFirstTimeReservedNameOffersNothing() throws {
    backend.ranks[""] = SpecPayload.ranked([(.everything, .emptyQuery)])
    backend.ranks["Everything"] = SpecPayload.ranked([(.everything, .exact)])
    let model = try makeModel(SpecPayload.payload())
    model.query = "Everything"
    XCTAssertEqual(model.rows, [])
    XCTAssertTrue(model.handle(.enter))
    XCTAssertEqual(model.mode, .list)
    XCTAssertEqual(backend.sent, [])
    XCTAssertEqual(closed, 0)
  }

  /// User story 1: type a name, choose "New context", uncheck the windows
  /// that don't belong, and press ↩.
  func testUserStoryOneSavesAContextFromTheWindowsOnScreen() throws {
    let ghostty = SwitcherWindowId(pid: 640, idx: 8812)
    let zed = SwitcherWindowId(pid: 701, idx: 8920)
    let chrome = SwitcherWindowId(pid: 812, idx: 9123)
    let slack = SwitcherWindowId(pid: 988, idx: 9402)
    backend.ranks[""] = SpecPayload.ranked([(.everything, .emptyQuery)])
    let model = try makeModel(
      SpecPayload.payload(
        target: chrome,
        unsorted: SwitcherUnsorted(windows: 4, active: false),
        windows: [
          SpecPayload.window(ghostty, "~/src/sugarglider", "Ghostty"),
          SpecPayload.window(zed, "reactor.rs", "Zed"),
          SpecPayload.window(chrome, "Docs", "Google Chrome"),
          SpecPayload.window(slack, "general", "Slack"),
        ]
      )
    )
    model.query = "Sugarglider"
    XCTAssertEqual(model.rows, [.newContext("Sugarglider")])
    XCTAssertEqual(model.highlight, 0)

    model.handle(.enter)
    XCTAssertEqual(model.mode, .create(name: "Sugarglider"))
    XCTAssertEqual(
      model.checklist.map(\.source), [ghostty, zed, chrome, slack].map { .window($0) })
    XCTAssertEqual(model.checklist.map(\.checked), [true, true, true, true])

    for _ in 0..<3 { model.handle(.down) }
    XCTAssertTrue(model.handle(.space))
    XCTAssertEqual(model.checklist.map(\.checked), [true, true, true, false])
    model.handle(.enter)
    XCTAssertEqual(backend.sent, [.create(name: "Sugarglider", windows: [ghostty, zed, chrome])])
    XCTAssertEqual(closed, 1)
  }

  /// The contract: `create` can have no windows.
  func testWithNoWindowsOnScreenANewContextHasNoWindows() throws {
    backend.ranks[""] = SpecPayload.ranked([(.everything, .emptyQuery)])
    let model = try makeModel(SpecPayload.payload())
    model.query = "Empty"
    model.handle(.enter)
    XCTAssertEqual(model.mode, .create(name: "Empty"))
    XCTAssertEqual(model.checklist, [])
    for key: SwitcherKey in [.down, .up, .space] {
      XCTAssertTrue(model.handle(key), "\(key)")
    }
    XCTAssertEqual(model.checklistHighlight, 0)
    model.handle(.enter)
    XCTAssertEqual(backend.sent, [.create(name: "Empty", windows: [])])
    XCTAssertEqual(closed, 1)
  }

  // MARK: Flag off

  /// With the feature off, rank returns NULL, which the bridge reports as
  /// "Contexts are unavailable.". The list falls back to the payload, and a
  /// command that Rust rejects shows its message and keeps the panel open.
  func testWithContextsOffTheSwitcherExplainsAndStaysOpen() throws {
    backend.rankError = SwitcherBridgeError(message: "Contexts are unavailable.")
    backend.runError = SwitcherBridgeError(message: "Contexts are turned off")
    let model = try makeModel(SpecPayload.payload())

    XCTAssertEqual(model.rankError, "Search is unavailable. Contexts are unavailable.")
    XCTAssertTrue(model.isFirstTime)
    XCTAssertEqual(model.rows, [])

    model.query = "Work"
    XCTAssertEqual(model.rows, [.newContext("Work")])
    model.handle(.enter)
    model.handle(.enter)
    XCTAssertEqual(backend.sent, [.create(name: "Work", windows: [])])
    XCTAssertEqual(model.message, "Contexts are turned off")
    XCTAssertEqual(closed, 0)
  }

  /// The fallback keeps the payload's entries and order, with Unsorted only
  /// when it has windows, whatever the query.
  func testTheFallbackListsThePayloadForEveryQuery() throws {
    backend.rankError = SwitcherBridgeError(message: "Contexts are unavailable.")
    let model = try makeModel()
    XCTAssertEqual(
      model.rows,
      [.entry(sugarglider), .entry(clientWork), .entry(unsorted), .entry(everything)]
    )
    XCTAssertEqual(model.highlight, 1)
    model.query = "zzz"
    XCTAssertEqual(
      model.rows,
      [
        .entry(sugarglider), .entry(clientWork), .entry(unsorted), .entry(everything),
        .newContext("zzz"),
      ]
    )
    XCTAssertEqual(backend.queries, ["", "zzz"])
  }

  /// Once rank works again, the inline error goes and the rows are ranked.
  func testTheRankErrorClearsWhenRankWorksAgain() throws {
    backend.rankError = SwitcherBridgeError(message: "Contexts are unavailable.")
    let model = try makeModel()
    XCTAssertNotNil(model.rankError)

    backend.rankError = nil
    model.query = "cli"
    XCTAssertNil(model.rankError)
    XCTAssertEqual(model.rows, [.entry(clientWork), .newContext("cli")])
  }
}
