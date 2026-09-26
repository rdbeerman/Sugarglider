// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import Foundation
import XCTest

@testable import SugargliderUI

/// The switcher's JSON contract: unknown keys, missing and null keys, and
/// the exact JSON of commands with empty lists (spec, "Swift bridge").
@MainActor
final class SwitcherContractSpecTests: SwitcherSpecTestCase {
  private let mail = SwitcherWindowId(pid: 650, idx: 7001)

  /// A show payload with every key, nullable ones included.
  private let fullPayload: [String: Any] = [
    "display_id": 1,
    "target_window": ["pid": 650, "idx": 7001],
    "contexts": [
      [
        "id": 1, "name": "Comms", "number": 1, "hotkey": "⌃⌥1", "active": true,
        "apps": ["Mail"], "windows": 1,
        "members": [
          ["record": 0, "app": "Mail", "title": "Inbox", "window": ["pid": 650, "idx": 7001]]
        ],
      ]
    ],
    "unsorted": ["windows": 0, "active": false],
    "everything": ["active": false, "hotkey": "⌃⌥0"],
    "windows": [
      [
        "id": ["pid": 650, "idx": 7001], "title": "Inbox", "app": "Mail", "tab_count": 1,
        "pinned": false,
      ]
    ],
  ]

  private var decodedFullPayload: SwitcherPayload {
    SwitcherPayload(
      displayId: 1,
      targetWindow: mail,
      contexts: [
        SwitcherContext(
          id: 1, name: "Comms", number: 1, hotkey: "⌃⌥1", active: true, apps: ["Mail"],
          windows: 1,
          members: [SwitcherMember(record: 0, app: "Mail", title: "Inbox", window: mail)]
        )
      ],
      unsorted: SwitcherUnsorted(windows: 0, active: false),
      everything: SwitcherEverything(active: false, hotkey: "⌃⌥0"),
      windows: [
        SwitcherWindow(id: mail, title: "Inbox", app: "Mail", tabCount: 1, pinned: false)
      ]
    )
  }

  /// Removes the key at a path. An array step is its index as a string.
  private func removing(_ path: ArraySlice<String>, from value: Any) -> Any {
    guard let step = path.first else { return value }
    let rest = path.dropFirst()
    if var dictionary = value as? [String: Any] {
      if rest.isEmpty {
        dictionary.removeValue(forKey: step)
      } else {
        dictionary[step] = removing(rest, from: dictionary[step] as Any)
      }
      return dictionary
    }
    if var array = value as? [Any], let index = Int(step) {
      array[index] = removing(rest, from: array[index])
      return array
    }
    return value
  }

  private func decodePayload(removing path: String) throws -> SwitcherPayload {
    let object = removing(path.split(separator: ".").map(String.init)[...], from: fullPayload)
    let data = try JSONSerialization.data(withJSONObject: object)
    return try ContextSwitcherJSON.decode(
      SwitcherPayload.self, from: String(decoding: data, as: UTF8.self))
  }

  // MARK: Show payload

  func testTheFullPayloadDecodes() throws {
    XCTAssertEqual(try decodePayload(removing: ""), decodedFullPayload)
  }

  /// Keys the contract doesn't name are ignored at every level.
  func testTheShowPayloadIgnoresUnknownKeysAtEveryLevel() throws {
    let json = """
      {
        "version": 2,
        "display_id": 1,
        "focused_screen": "Built-in Retina Display",
        "target_window": { "pid": 650, "idx": 7001, "title": "Inbox" },
        "contexts": [
          { "id": 1, "name": "Comms", "number": 1, "hotkey": "⌃⌥1", "active": true,
            "apps": ["Mail"], "windows": 1, "color": "blue", "layout": { "kind": "tree" },
            "members": [
              { "record": 0, "app": "Mail", "title": "Inbox", "bundle_id": "com.apple.mail",
                "window": { "pid": 650, "idx": 7001, "space": 3 }, "pending": false }
            ] }
        ],
        "unsorted": { "windows": 0, "active": false, "pinned": 1 },
        "everything": { "active": false, "hotkey": "⌃⌥0", "layout": "bsp" },
        "windows": [
          { "id": { "pid": 650, "idx": 7001, "space": null }, "title": "Inbox", "app": "Mail",
            "contexts": [{ "id": 1, "name": "Comms" }], "tab_count": 1, "pinned": false,
            "frame": [0, 0, 800, 600] }
        ],
        "scope": "global"
      }
      """
    XCTAssertEqual(
      try ContextSwitcherJSON.decode(SwitcherPayload.self, from: json), decodedFullPayload)
  }

  /// A key whose type says "or null" may be null.
  func testTheShowPayloadTakesExplicitNullsAsNull() throws {
    let json = """
      {
        "display_id": null,
        "target_window": null,
        "contexts": [
          { "id": 1, "name": "Comms", "number": null, "hotkey": null, "active": false,
            "apps": [], "windows": 0,
            "members": [{ "record": 0, "app": "Mail", "title": "Inbox", "window": null }] }
        ],
        "unsorted": { "windows": 0, "active": false },
        "everything": { "active": true, "hotkey": null },
        "windows": []
      }
      """
    XCTAssertEqual(
      try ContextSwitcherJSON.decode(SwitcherPayload.self, from: json),
      SwitcherPayload(
        displayId: nil,
        targetWindow: nil,
        contexts: [
          SwitcherContext(
            id: 1, name: "Comms", number: nil, hotkey: nil, active: false, apps: [], windows: 0,
            members: [SwitcherMember(record: 0, app: "Mail", title: "Inbox", window: nil)]
          )
        ],
        unsorted: SwitcherUnsorted(windows: 0, active: false),
        everything: SwitcherEverything(active: true, hotkey: nil),
        windows: []
      )
    )
  }

  /// A key whose type says "or null" may also be missing, one at a time.
  func testEachNullableKeyMayBeMissing() throws {
    var expected = decodedFullPayload
    expected.displayId = nil
    XCTAssertEqual(try decodePayload(removing: "display_id"), expected)

    expected = decodedFullPayload
    expected.targetWindow = nil
    XCTAssertEqual(try decodePayload(removing: "target_window"), expected)

    expected = decodedFullPayload
    expected.contexts[0].number = nil
    XCTAssertEqual(try decodePayload(removing: "contexts.0.number"), expected)

    expected = decodedFullPayload
    expected.contexts[0].hotkey = nil
    XCTAssertEqual(try decodePayload(removing: "contexts.0.hotkey"), expected)

    expected = decodedFullPayload
    expected.contexts[0].members[0].window = nil
    XCTAssertEqual(try decodePayload(removing: "contexts.0.members.0.window"), expected)

    expected = decodedFullPayload
    expected.everything.hotkey = nil
    XCTAssertEqual(try decodePayload(removing: "everything.hotkey"), expected)
  }

  /// The contract: a payload that doesn't decode shows nothing. Every key
  /// whose type doesn't say "or null" is needed.
  func testEveryKeyThatCantBeNullIsNeeded() {
    let required = [
      "contexts", "unsorted", "everything", "windows",
      "target_window.pid", "target_window.idx",
      "contexts.0.id", "contexts.0.name", "contexts.0.active", "contexts.0.apps",
      "contexts.0.windows", "contexts.0.members",
      "contexts.0.members.0.record", "contexts.0.members.0.app", "contexts.0.members.0.title",
      "contexts.0.members.0.window.pid", "contexts.0.members.0.window.idx",
      "unsorted.windows", "unsorted.active", "everything.active",
      "windows.0.id", "windows.0.id.pid", "windows.0.id.idx", "windows.0.title", "windows.0.app",
      "windows.0.tab_count", "windows.0.pinned",
    ]
    for path in required {
      XCTAssertThrowsError(try decodePayload(removing: path), path)
    }
  }

  /// A payload with every nullable key missing: no shortcut where no
  /// binding is known, and no target window, so ⌘↩ is disabled.
  func testThePanelWorksWithEveryNullableKeyMissing() throws {
    let json = """
      {
        "contexts": [
          { "id": 2, "name": "Comms", "active": false, "apps": [], "windows": 0, "members": [] }
        ],
        "unsorted": { "windows": 0, "active": false },
        "everything": { "active": true },
        "windows": []
      }
      """
    backend.ranks[""] = SpecPayload.ranked([
      (.everything, .emptyQuery), (SpecPayload.named(2), .emptyQuery),
    ])
    let model = try makeModel(ContextSwitcherJSON.decode(SwitcherPayload.self, from: json))
    XCTAssertEqual(
      model.rows,
      [
        .entry(
          SwitcherEntry(
            key: .everything, name: "Everything", detail: "show all windows", shortcut: nil,
            active: true)),
        .entry(
          SwitcherEntry(
            key: SpecPayload.named(2), name: "Comms", detail: "no open windows", shortcut: nil,
            active: false)),
      ]
    )
    XCTAssertEqual(model.highlight, 1)
    XCTAssertNil(model.targetWindow)
    model.handle(.commandEnter)
    XCTAssertEqual(model.message, "No window had focus when the switcher opened.")
    XCTAssertEqual(backend.sent, [])
  }

  // MARK: Rank result

  func testRankEntriesIgnoreUnknownKeys() throws {
    let json = """
      [
        { "key": { "id": 4, "name": "Client work" }, "match": "exact", "score": 12 },
        { "key": "unsorted", "match": "letters_in_order", "positions": [0, 3] }
      ]
      """
    XCTAssertEqual(
      try ContextSwitcherJSON.decode([SwitcherRankedEntry].self, from: json),
      [
        SwitcherRankedEntry(key: .named(Fixtures.clientWork), match: .exact),
        SwitcherRankedEntry(key: .unsorted, match: .lettersInOrder),
      ]
    )
  }

  /// The key is `{"id": N}`, `"unsorted"`, or `"everything"`, and the match
  /// is one of the `NameMatch` names. Anything else doesn't decode.
  func testARankEntryNeedsAKnownKeyAndMatch() {
    for json in [
      #"[{ "key": 4, "match": "exact" }]"#,
      #"[{ "key": "Client work", "match": "exact" }]"#,
      #"[{ "key": "Everything", "match": "exact" }]"#,
      #"[{ "key": { "id": 4 }, "match": "fuzzy" }]"#,
      #"[{ "key": { "id": 4 }, "match": "namePrefix" }]"#,
      #"[{ "key": { "id": 4 } }]"#,
      #"[{ "match": "exact" }]"#,
    ] {
      XCTAssertThrowsError(
        try ContextSwitcherJSON.decode([SwitcherRankedEntry].self, from: json), json)
    }
  }

  // MARK: Commands

  func testCommandBodiesIgnoreUnknownKeys() throws {
    let cases: [(String, SwitcherCommand)] = [
      (
        #"""
        { "add_window": { "window": { "pid": 812, "idx": 9123, "app": "Chrome" },
                          "context": { "id": 4, "name": "Client work" },
                          "source": "switcher" } }
        """#,
        .addWindow(window: Fixtures.chrome, context: Fixtures.clientWork)
      ),
      (
        #"""
        { "edit": { "context": { "id": 1 }, "add": [], "remove": [],
                    "remove_records": [{ "record": 3, "app": "Mail", "title": "Inbox",
                                         "bundle_id": "com.apple.mail" }],
                    "note": 1 } }
        """#,
        .edit(
          context: Fixtures.sugarglider, add: [], remove: [],
          removeRecords: [SwitcherRecordRef(record: 3, app: "Mail", title: "Inbox")])
      ),
      (
        #"{ "set_number": { "context": { "id": 4 }, "number": 2, "previous": null } }"#,
        .setNumber(context: Fixtures.clientWork, number: 2)
      ),
    ]
    for (json, command) in cases {
      XCTAssertEqual(
        try ContextSwitcherJSON.decode(SwitcherCommand.self, from: json), command, json)
    }
  }

  /// The contract: `create` can have no windows. The list is sent, empty.
  func testCreateWithoutWindowsSendsAnEmptyList() throws {
    XCTAssertEqual(
      try ContextSwitcherJSON.encode(SwitcherCommand.create(name: "Empty", windows: [])),
      #"{"create":{"name":"Empty","windows":[]}}"#
    )
  }

  /// The contract: `edit` carries only what changed, and its three lists
  /// are always there, empty or not.
  func testEditSendsEveryListEvenWhenEmpty() throws {
    XCTAssertEqual(
      try ContextSwitcherJSON.encode(
        SwitcherCommand.edit(
          context: Fixtures.sugarglider, add: [], remove: [],
          removeRecords: [SwitcherRecordRef(record: 3, app: "Mail", title: "Inbox")])
      ),
      #"{"edit":{"add":[],"context":{"id":1},"remove":[],"remove_records":[{"app":"Mail","record":3,"title":"Inbox"}]}}"#
    )
    XCTAssertEqual(
      try ContextSwitcherJSON.encode(
        SwitcherCommand.edit(
          context: Fixtures.sugarglider, add: [Fixtures.finder], remove: [], removeRecords: [])
      ),
      #"{"edit":{"add":[{"idx":9310,"pid":977}],"context":{"id":1},"remove":[],"remove_records":[]}}"#
    )
  }

  /// Names in `create` and `rename` keep quotes, backslashes, slashes, and
  /// non-ASCII letters through the JSON.
  func testNamesKeepEveryCharacterThroughTheJSON() throws {
    let name = #"Café “Client” "work" \ / 日本"#
    for command in [
      SwitcherCommand.create(name: name, windows: []),
      SwitcherCommand.rename(context: Fixtures.clientWork, name: name),
    ] {
      let json = try ContextSwitcherJSON.encode(command)
      let object = try XCTUnwrap(
        try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: [String: Any]])
      XCTAssertEqual(object.values.first?["name"] as? String, name, json)
      XCTAssertEqual(try ContextSwitcherJSON.decode(SwitcherCommand.self, from: json), command)
    }
  }

  /// A window is Rust's `WindowId`: `pid` is a `pid_t` and `idx` a
  /// `NonZeroU32`. Both keep their full range.
  func testWindowIdsKeepTheirFullRange() throws {
    let window = SwitcherWindowId(pid: Int32.max, idx: UInt32.max)
    XCTAssertEqual(
      try ContextSwitcherJSON.encode(SwitcherCommand.togglePinned(window: window)),
      #"{"toggle_pinned":{"window":{"idx":4294967295,"pid":2147483647}}}"#
    )
    XCTAssertThrowsError(
      try ContextSwitcherJSON.decode(
        SwitcherWindowId.self, from: #"{ "pid": 1, "idx": 4294967296 }"#)
    )
  }
}
