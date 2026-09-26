// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import Foundation
import XCTest

@testable import SugargliderUI

final class ContextSwitcherContractTests: XCTestCase {
  func testShowPayloadExampleDecodes() throws {
    let payload = try Fixtures.payload()

    XCTAssertEqual(payload.displayId, 1)
    XCTAssertEqual(payload.targetWindow, Fixtures.chrome)
    XCTAssertEqual(payload.contexts.map(\.name), ["Sugarglider", "Client work"])

    let sugarglider = payload.contexts[0]
    XCTAssertEqual(sugarglider.key, .named(Fixtures.sugarglider))
    XCTAssertEqual(sugarglider.number, 1)
    XCTAssertEqual(sugarglider.hotkey, "⌃⌥1")
    XCTAssertTrue(sugarglider.active)
    XCTAssertEqual(sugarglider.apps, ["Ghostty", "Zed", "Google Chrome"])
    XCTAssertEqual(sugarglider.windows, 3)
    XCTAssertEqual(
      sugarglider.members.last,
      SwitcherMember(record: 3, app: "Mail", title: "Inbox", window: nil)
    )
    XCTAssertEqual(sugarglider.members[1].window, Fixtures.zed)

    let clientWork = payload.contexts[1]
    XCTAssertNil(clientWork.number)
    XCTAssertNil(clientWork.hotkey)
    XCTAssertFalse(clientWork.active)
    XCTAssertEqual(clientWork.members, [])

    XCTAssertEqual(payload.unsorted, SwitcherUnsorted(windows: 2, active: false))
    XCTAssertEqual(payload.everything, SwitcherEverything(active: false, hotkey: "⌃⌥0"))

    XCTAssertEqual(payload.windows.count, 5)
    XCTAssertEqual(
      payload.windows[1],
      SwitcherWindow(
        id: Fixtures.chrome, title: "Docs", app: "Google Chrome", tabCount: 3, pinned: false)
    )
    XCTAssertTrue(payload.windows[2].pinned)
  }

  func testShowPayloadTakesMissingNullableKeysAsNull() throws {
    let json = """
      {
        "contexts": [
          { "id": 2, "name": "Comms", "active": false, "apps": [], "windows": 0,
            "members": [{ "record": 0, "app": "Mail", "title": "Inbox" }] }
        ],
        "unsorted": { "windows": 0, "active": false },
        "everything": { "active": true },
        "windows": []
      }
      """
    let payload = try ContextSwitcherJSON.decode(SwitcherPayload.self, from: json)

    XCTAssertNil(payload.displayId)
    XCTAssertNil(payload.targetWindow)
    XCTAssertNil(payload.contexts[0].number)
    XCTAssertNil(payload.contexts[0].hotkey)
    XCTAssertNil(payload.contexts[0].members[0].window)
    XCTAssertNil(payload.everything.hotkey)
  }

  func testShowPayloadRoundTrips() throws {
    let payload = try Fixtures.payload()
    let json = try ContextSwitcherJSON.encode(payload)
    XCTAssertEqual(try ContextSwitcherJSON.decode(SwitcherPayload.self, from: json), payload)
  }

  func testRankExamplesDecode() throws {
    XCTAssertEqual(
      try ContextSwitcherJSON.decode([SwitcherRankedEntry].self, from: Fixtures.rankForCli),
      [SwitcherRankedEntry(key: .named(Fixtures.clientWork), match: .namePrefix)]
    )
    XCTAssertEqual(
      try ContextSwitcherJSON.decode([SwitcherRankedEntry].self, from: Fixtures.rankForEmptyQuery),
      [
        SwitcherRankedEntry(key: .named(Fixtures.sugarglider), match: .emptyQuery),
        SwitcherRankedEntry(key: .named(Fixtures.clientWork), match: .emptyQuery),
        SwitcherRankedEntry(key: .unsorted, match: .emptyQuery),
        SwitcherRankedEntry(key: .everything, match: .emptyQuery),
      ]
    )
  }

  func testEveryNameMatchDecodesFromItsRustName() throws {
    let names = [
      "exact", "name_prefix", "word_prefix", "initials", "all_word_prefixes",
      "letters_in_order", "empty_query",
    ]
    let json =
      "[" + names.map { #"{ "key": "everything", "match": "\#($0)" }"# }.joined(separator: ",")
      + "]"
    let entries = try ContextSwitcherJSON.decode([SwitcherRankedEntry].self, from: json)
    XCTAssertEqual(entries.map(\.match.rawValue), names)
  }

  func testEveryCommandExampleDecodesToItsCommand() throws {
    for (json, command) in Fixtures.commands {
      XCTAssertEqual(
        try ContextSwitcherJSON.decode(SwitcherCommand.self, from: json), command, json)
    }
  }

  func testEveryCommandEncodesToItsExample() throws {
    for (json, command) in Fixtures.commands {
      let encoded = try ContextSwitcherJSON.encode(command)
      XCTAssertEqual(try jsonObject(encoded), try jsonObject(json), encoded)
    }
  }

  func testEveryCommandRoundTrips() throws {
    for (_, command) in Fixtures.commands {
      let encoded = try ContextSwitcherJSON.encode(command)
      XCTAssertEqual(try ContextSwitcherJSON.decode(SwitcherCommand.self, from: encoded), command)
    }
  }

  func testCommandExamplesCoverEveryCommand() {
    let names = Set(Fixtures.commands.map { commandName($0.command) })
    XCTAssertEqual(
      names,
      [
        "switch", "add_window", "move_window", "toggle_pinned", "create", "edit", "rename",
        "set_number", "delete",
      ]
    )
  }

  func testContextIdsAreTaggedNeverBareNumbersOrNames() {
    for json in [#"{ "switch": 4 }"#, #"{ "switch": "Comms" }"#, #"{ "delete": 4 }"#] {
      XCTAssertThrowsError(try ContextSwitcherJSON.decode(SwitcherCommand.self, from: json), json)
    }
    XCTAssertEqual(
      try ContextSwitcherJSON.encode(SwitcherCommand.delete(Fixtures.clientWork)),
      #"{"delete":{"id":4}}"#
    )
  }

  func testCommandNeedsExactlyOneKnownKey() {
    for json in [#"{}"#, #"{ "launch": {} }"#, #"{ "switch": "unsorted", "delete": { "id": 4 } }"#]
    {
      XCTAssertThrowsError(try ContextSwitcherJSON.decode(SwitcherCommand.self, from: json), json)
    }
  }

  /// `docs/specs/contexts-switcher-contract.md`, the doc comment on
  /// `ContextSwitcherJSON`, and `Fixtures` hold the same examples, in the
  /// same order.
  func testTheContractExamplesAreTheSameInEveryCopy() throws {
    let package = URL(fileURLWithPath: #filePath)
      .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    let markdown = try String(
      contentsOf: package.deletingLastPathComponent()
        .appendingPathComponent("docs/specs/contexts-switcher-contract.md"),
      encoding: .utf8
    )
    let source = try String(
      contentsOf: package.appendingPathComponent(
        "Sources/SugargliderUI/ContextSwitcherContract.swift"),
      encoding: .utf8
    )
    let fixtures =
      [Fixtures.showPayload, Fixtures.rankForCli, Fixtures.rankForEmptyQuery]
      + Fixtures.commands.map(\.json)
    let expected = try fixtures.map(jsonValue)

    XCTAssertEqual(try jsonBlocks(in: markdown, linePrefix: "").map(jsonValue), expected)
    XCTAssertEqual(try jsonBlocks(in: source, linePrefix: "///").map(jsonValue), expected)
  }

  func testMemberWithoutWindowEncodesNull() throws {
    let member = SwitcherMember(record: 3, app: "Mail", title: "Inbox", window: nil)
    XCTAssertEqual(
      try ContextSwitcherJSON.encode(member),
      #"{"app":"Mail","record":3,"title":"Inbox","window":null}"#
    )
  }

  /// The text of each fenced `json` block, in order. `linePrefix` is
  /// removed from each line first, such as the `///` of a doc comment.
  private func jsonBlocks(in text: String, linePrefix: String) -> [String] {
    var blocks: [String] = []
    var current: [String]?
    for line in text.components(separatedBy: "\n") {
      let trimmed = line.trimmingCharacters(in: .whitespaces)
      guard trimmed.hasPrefix(linePrefix) else {
        current = nil
        continue
      }
      let content = String(trimmed.dropFirst(linePrefix.count))
      let fence = content.trimmingCharacters(in: .whitespaces)
      if let lines = current {
        if fence == "```" {
          blocks.append(lines.joined(separator: "\n"))
          current = nil
        } else {
          current = lines + [content]
        }
      } else if fence == "```json" {
        current = []
      }
    }
    return blocks
  }

  private func jsonValue(_ json: String) throws -> NSObject {
    let value = try JSONSerialization.jsonObject(with: Data(json.utf8))
    return try XCTUnwrap(value as? NSObject)
  }

  private func jsonObject(_ json: String) throws -> NSDictionary {
    let object = try JSONSerialization.jsonObject(with: Data(json.utf8))
    return try XCTUnwrap(object as? NSDictionary)
  }

  private func commandName(_ command: SwitcherCommand) -> String {
    let json = (try? ContextSwitcherJSON.encode(command)) ?? ""
    let object = (try? jsonObject(json)) ?? [:]
    return (object.allKeys.first as? String) ?? ""
  }
}
