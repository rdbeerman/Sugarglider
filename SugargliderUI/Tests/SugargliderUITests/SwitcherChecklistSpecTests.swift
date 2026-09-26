// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import XCTest

@testable import SugargliderUI

/// The create, edit, and rename views (spec, "Switcher").
///
/// The shared payload has a WhatsApp window in both Comms and Relax (R1).
/// Comms is active. It also holds two closed Slack records with the same
/// title and an open Zoom window that isn't on screen.
@MainActor
final class SwitcherChecklistSpecTests: SwitcherSpecTestCase {
  private let whatsApp = SwitcherWindowId(pid: 903, idx: 9201)
  private let mail = SwitcherWindowId(pid: 650, idx: 7001)
  private let safari = SwitcherWindowId(pid: 660, idx: 7002)
  private let terminal = SwitcherWindowId(pid: 670, idx: 7003)
  private let zoom = SwitcherWindowId(pid: 680, idx: 7004)
  private let comms = SwitcherContextId(id: 2)
  private let relax = SwitcherContextId(id: 3)

  private var slackRecord2: SwitcherRecordRef {
    SwitcherRecordRef(record: 2, app: "Slack", title: "general")
  }
  private var slackRecord3: SwitcherRecordRef {
    SwitcherRecordRef(record: 3, app: "Slack", title: "general")
  }

  private func sharedPayload() -> SwitcherPayload {
    SpecPayload.payload(
      target: whatsApp,
      contexts: [
        SpecPayload.context(
          2, "Comms", number: 3, hotkey: "⌃⌥3", active: true,
          apps: ["WhatsApp", "Mail", "zoom.us"],
          members: [
            SwitcherMember(record: 0, app: "WhatsApp", title: "WhatsApp", window: whatsApp),
            SwitcherMember(record: 1, app: "Mail", title: "Inbox", window: mail),
            SwitcherMember(record: 2, app: "Slack", title: "general", window: nil),
            SwitcherMember(record: 3, app: "Slack", title: "general", window: nil),
            SwitcherMember(record: 4, app: "zoom.us", title: "Meeting", window: zoom),
          ]
        ),
        SpecPayload.context(
          3, "Relax", apps: ["WhatsApp", "Safari"],
          members: [
            SwitcherMember(record: 0, app: "WhatsApp", title: "WhatsApp", window: whatsApp),
            SwitcherMember(record: 1, app: "Safari", title: "News", window: safari),
          ]
        ),
      ],
      unsorted: SwitcherUnsorted(windows: 1, active: false),
      everything: SwitcherEverything(active: false, hotkey: "⌃⌥0"),
      windows: [
        SpecPayload.window(whatsApp, "WhatsApp", "WhatsApp", in: [2, 3]),
        SpecPayload.window(mail, "Inbox", "Mail", in: [2]),
        SpecPayload.window(safari, "News", "Safari", in: [3]),
        SpecPayload.window(terminal, "zsh", "Terminal"),
      ]
    )
  }

  /// Rows: Comms (active), Relax, Unsorted, Everything.
  private func makeSharedModel() throws -> ContextSwitcherModel {
    backend.ranks[""] = SpecPayload.ranked([
      (.named(comms), .emptyQuery), (.named(relax), .emptyQuery), (.unsorted, .emptyQuery),
      (.everything, .emptyQuery),
    ])
    return try makeModel(sharedPayload())
  }

  private func editComms() throws -> ContextSwitcherModel {
    let model = try makeSharedModel()
    highlight(model, row: 0)
    model.handle(.commandE)
    XCTAssertEqual(model.mode, .edit(comms))
    return model
  }

  private func editRelax() throws -> ContextSwitcherModel {
    let model = try makeSharedModel()
    highlight(model, row: 1)
    model.handle(.commandE)
    XCTAssertEqual(model.mode, .edit(relax))
    return model
  }

  // MARK: Edit

  /// R1: a window in two contexts is a checked member in the edit view of
  /// each. The edit view lists the windows on screen, then the members that
  /// aren't on screen: closed records and open windows elsewhere.
  func testASharedWindowIsCheckedInTheEditViewOfEachOfItsContexts() throws {
    let comms = try editComms()
    XCTAssertEqual(
      comms.checklist.map(\.source),
      [
        .window(whatsApp), .window(mail), .window(safari), .window(terminal),
        .record(slackRecord2), .record(slackRecord3), .window(zoom),
      ]
    )
    XCTAssertEqual(comms.checklist.map(\.checked), [true, true, false, false, true, true, true])
    XCTAssertEqual(
      comms.checklist.map(\.title),
      ["WhatsApp", "Inbox", "News", "zsh", "general", "general", "Meeting"]
    )
    XCTAssertEqual(
      comms.checklist.map(\.app),
      ["WhatsApp", "Mail", "Safari", "Terminal", "Slack", "Slack", "zoom.us"]
    )

    let relax = try editRelax()
    XCTAssertEqual(
      relax.checklist.map(\.source),
      [.window(whatsApp), .window(mail), .window(safari), .window(terminal)]
    )
    XCTAssertEqual(relax.checklist.map(\.checked), [true, false, true, false])
  }

  /// R1, R37: unchecking a shared window removes it from the edited context
  /// only.
  func testUncheckingASharedWindowRemovesItOnlyFromTheEditedContext() throws {
    let model = try editRelax()
    model.toggleItem(at: 0)
    model.handle(.enter)
    XCTAssertEqual(
      backend.sent,
      [.edit(context: relax, add: [], remove: [whatsApp], removeRecords: [])]
    )
    XCTAssertEqual(closed, 1)
  }

  /// R1, R37: checking a window of another context adds it, and it stays in
  /// the other context.
  func testCheckingAWindowOfAnotherContextAddsItToTheEditedContext() throws {
    let model = try editRelax()
    model.handle(.down)
    XCTAssertTrue(model.handle(.space))
    model.handle(.enter)
    XCTAssertEqual(
      backend.sent,
      [.edit(context: relax, add: [mail], remove: [], removeRecords: [])]
    )
  }

  /// R23: two closed records with the same app and title are separate
  /// items. Unchecking one removes only that record, by its index.
  func testTwoClosedRecordsWithTheSameTitleAreSeparateItems() throws {
    let model = try editComms()
    model.toggleItem(at: 5)
    model.handle(.enter)
    XCTAssertEqual(
      backend.sent,
      [.edit(context: comms, add: [], remove: [], removeRecords: [slackRecord3])]
    )
  }

  /// A closed record unchecked and checked again stays, and nothing is sent.
  func testAClosedRecordCheckedAgainIsKept() throws {
    let model = try editComms()
    for _ in 0..<4 { model.handle(.down) }
    XCTAssertEqual(model.checklistHighlight, 4)
    model.handle(.space)
    model.handle(.space)
    XCTAssertTrue(model.checklist[4].checked)
    model.handle(.enter)
    XCTAssertEqual(backend.sent, [])
    XCTAssertEqual(closed, 1)
  }

  /// R23: a member whose `window` key is missing is a closed record, and
  /// unchecking it removes the record. A blank title is kept as it is.
  func testAMemberWithoutAWindowKeyIsAClosedRecord() throws {
    let json = """
      {
        "contexts": [
          { "id": 5, "name": "Writing", "active": true, "apps": ["Mail"], "windows": 1,
            "members": [
              { "record": 0, "app": "Mail", "title": "Inbox",
                "window": { "pid": 650, "idx": 7001 } },
              { "record": 1, "app": "com.apple.Notes", "title": "Draft" },
              { "record": 2, "app": "Mail", "title": "", "window": null }
            ] }
        ],
        "unsorted": { "windows": 0, "active": false },
        "everything": { "active": false },
        "windows": [
          { "id": { "pid": 650, "idx": 7001 }, "title": "Inbox", "app": "Mail",
            "contexts": [{ "id": 5 }], "pinned": false }
        ]
      }
      """
    let writing = SwitcherContextId(id: 5)
    backend.ranks[""] = SpecPayload.ranked([
      (.named(writing), .emptyQuery), (.everything, .emptyQuery),
    ])
    let model = try makeModel(ContextSwitcherJSON.decode(SwitcherPayload.self, from: json))
    highlight(model, row: 0)
    model.handle(.commandE)

    let notes = SwitcherRecordRef(record: 1, app: "com.apple.Notes", title: "Draft")
    let blank = SwitcherRecordRef(record: 2, app: "Mail", title: "")
    XCTAssertEqual(model.checklist.map(\.source), [.window(mail), .record(notes), .record(blank)])
    XCTAssertEqual(model.checklist.map(\.checked), [true, true, true])

    model.toggleItem(at: 1)
    model.toggleItem(at: 2)
    model.handle(.enter)
    XCTAssertEqual(
      backend.sent,
      [.edit(context: writing, add: [], remove: [], removeRecords: [notes, blank])]
    )
  }

  /// A context whose windows are all gone lists only its records, and with
  /// no windows on screen nothing else.
  func testEditingAContextWhoseWindowsAreAllGone() throws {
    let gone = SpecPayload.context(
      7, "Archive",
      members: [
        SwitcherMember(record: 0, app: "Preview", title: "scan.pdf", window: nil),
        SwitcherMember(record: 1, app: "Pages", title: "Letter", window: nil),
      ]
    )
    backend.ranks[""] = SpecPayload.ranked([
      (.everything, .emptyQuery), (.named(gone.contextId), .emptyQuery),
    ])
    let model = try makeModel(SpecPayload.payload(contexts: [gone]))
    XCTAssertEqual(model.highlight, 1)
    model.handle(.commandE)
    XCTAssertEqual(model.mode, .edit(gone.contextId))
    let preview = SwitcherRecordRef(record: 0, app: "Preview", title: "scan.pdf")
    let pages = SwitcherRecordRef(record: 1, app: "Pages", title: "Letter")
    XCTAssertEqual(model.checklist.map(\.source), [.record(preview), .record(pages)])

    model.handle(.space)
    model.handle(.down)
    model.handle(.space)
    model.handle(.enter)
    XCTAssertEqual(
      backend.sent,
      [.edit(context: gone.contextId, add: [], remove: [], removeRecords: [preview, pages])]
    )
  }

  /// A context with no members and no windows on screen gives an empty
  /// checklist. The keys do nothing, and ↩ closes without a command.
  func testEditingWithNothingToListClosesWithoutACommand() throws {
    let empty = SpecPayload.context(8, "Empty")
    backend.ranks[""] = SpecPayload.ranked([
      (.everything, .emptyQuery), (.named(empty.contextId), .emptyQuery),
    ])
    let model = try makeModel(SpecPayload.payload(contexts: [empty]))
    XCTAssertEqual(model.highlight, 1)
    model.handle(.commandE)
    XCTAssertEqual(model.mode, .edit(empty.contextId))
    XCTAssertEqual(model.checklist, [])
    for key: SwitcherKey in [.down, .up, .space] {
      XCTAssertTrue(model.handle(key), "\(key)")
    }
    XCTAssertEqual(model.checklistHighlight, 0)
    model.handle(.enter)
    XCTAssertEqual(backend.sent, [])
    XCTAssertEqual(closed, 1)
  }

  /// The contract: a rejected edit shows Rust's message, and the panel
  /// stays open.
  func testARejectedEditShowsTheErrorAndStaysOpen() throws {
    backend.runError = SwitcherBridgeError(message: "No context has the id 2")
    let model = try editComms()
    model.toggleItem(at: 1)
    model.handle(.enter)
    XCTAssertEqual(
      backend.sent,
      [.edit(context: comms, add: [], remove: [mail], removeRecords: [])]
    )
    XCTAssertEqual(model.message, "No context has the id 2")
    XCTAssertEqual(closed, 0)
  }

  // MARK: Create

  /// User story 1: the create view lists the windows on screen, all
  /// checked, whatever contexts they are in. Members that aren't on screen
  /// and closed records aren't listed.
  func testTheCreateViewListsOnlyTheWindowsOnScreenAllChecked() throws {
    let model = try makeSharedModel()
    model.query = "Evening"
    model.handle(.commandN)
    XCTAssertEqual(model.mode, .create(name: "Evening"))
    XCTAssertEqual(
      model.checklist.map(\.source),
      [.window(whatsApp), .window(mail), .window(safari), .window(terminal)]
    )
    XCTAssertEqual(model.checklist.map(\.checked), [true, true, true, true])
    model.handle(.enter)
    XCTAssertEqual(
      backend.sent,
      [.create(name: "Evening", windows: [whatsApp, mail, safari, terminal])]
    )
  }

  // MARK: Rename

  /// R4 compares names ignoring case, but a context's own name doesn't
  /// clash with it, so a change of case only is a rename.
  func testRenamingToACaseVariantOfTheSameNameIsSent() throws {
    let model = try makeModel()
    model.handle(.commandR)
    model.nameDraft = "client WORK"
    model.handle(.enter)
    XCTAssertEqual(backend.sent, [.rename(context: Fixtures.clientWork, name: "client WORK")])
    XCTAssertEqual(closed, 1)
  }

  /// The panel sends the trimmed name, so spaces around the same name are
  /// no rename.
  func testRenamingToTheSameNameWithSpacesAroundItSendsNothing() throws {
    let model = try makeModel()
    model.handle(.commandR)
    model.nameDraft = "  Client work\t"
    model.handle(.enter)
    XCTAssertEqual(backend.sent, [])
    XCTAssertEqual(model.mode, .list)
    XCTAssertEqual(closed, 0)
  }

  /// R4: Rust rejects a taken or reserved name. The rename view stays with
  /// the message until the user edits the name.
  func testARejectedRenameKeepsTheRenameViewUntilTheNameChanges() throws {
    let model = try makeModel()
    model.handle(.commandR)
    backend.runError = SwitcherBridgeError(
      message: #"A context named "Sugarglider" already exists"#)
    model.nameDraft = "sugarglider"
    model.handle(.enter)
    XCTAssertEqual(model.message, #"A context named "Sugarglider" already exists"#)
    XCTAssertEqual(model.mode, .rename(Fixtures.clientWork))
    XCTAssertEqual(closed, 0)

    backend.runError = SwitcherBridgeError(message: #""Everything" is a reserved name"#)
    model.nameDraft = "Everything"
    XCTAssertNil(model.message)
    model.handle(.enter)
    XCTAssertEqual(model.message, #""Everything" is a reserved name"#)
    XCTAssertEqual(model.mode, .rename(Fixtures.clientWork))

    backend.runError = nil
    model.nameDraft = "Clients"
    model.handle(.enter)
    XCTAssertEqual(
      backend.sent,
      [
        .rename(context: Fixtures.clientWork, name: "sugarglider"),
        .rename(context: Fixtures.clientWork, name: "Everything"),
        .rename(context: Fixtures.clientWork, name: "Clients"),
      ]
    )
    XCTAssertEqual(closed, 1)
  }
}
