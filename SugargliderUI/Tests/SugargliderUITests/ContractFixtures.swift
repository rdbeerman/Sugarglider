// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

@testable import SugargliderUI

/// The examples of the switcher's JSON contract, as
/// `docs/specs/contexts-switcher-contract.md` and the doc comment on
/// `ContextSwitcherJSON` show them.
enum Fixtures {
  static let showPayload = """
    {
      "display_id": 1,
      "target_window": { "pid": 812, "idx": 9123 },
      "contexts": [
        {
          "id": 1, "name": "Sugarglider", "number": 1, "hotkey": "⌃⌥1",
          "active": true, "apps": ["Ghostty", "Zed", "Google Chrome"], "windows": 3,
          "members": [
            { "record": 0, "app": "Ghostty", "title": "~/src/sugarglider",
              "window": { "pid": 640, "idx": 8812 } },
            { "record": 1, "app": "Zed", "title": "reactor.rs",
              "window": { "pid": 701, "idx": 8920 } },
            { "record": 2, "app": "Google Chrome", "title": "Docs",
              "window": { "pid": 812, "idx": 9123 } },
            { "record": 3, "app": "Mail", "title": "Inbox", "window": null }
          ]
        },
        {
          "id": 4, "name": "Client work", "number": null, "hotkey": null,
          "active": false, "apps": [], "windows": 0, "members": []
        }
      ],
      "unsorted": { "windows": 2, "active": false },
      "everything": { "active": false, "hotkey": "⌃⌥0" },
      "windows": [
        { "id": { "pid": 640, "idx": 8812 }, "title": "~/src/sugarglider",
          "app": "Ghostty", "tab_count": 1, "pinned": false },
        { "id": { "pid": 812, "idx": 9123 }, "title": "Docs",
          "app": "Google Chrome", "tab_count": 3, "pinned": false },
        { "id": { "pid": 903, "idx": 9201 }, "title": "WhatsApp",
          "app": "WhatsApp", "tab_count": 1, "pinned": true },
        { "id": { "pid": 977, "idx": 9310 }, "title": "Downloads",
          "app": "Finder", "tab_count": 1, "pinned": false },
        { "id": { "pid": 988, "idx": 9402 }, "title": "general",
          "app": "Slack", "tab_count": 1, "pinned": false }
      ]
    }
    """

  static let rankForCli = """
    [
      { "key": { "id": 4 }, "match": "name_prefix" }
    ]
    """

  static let rankForEmptyQuery = """
    [
      { "key": { "id": 1 }, "match": "empty_query" },
      { "key": { "id": 4 }, "match": "empty_query" },
      { "key": "unsorted", "match": "empty_query" },
      { "key": "everything", "match": "empty_query" }
    ]
    """

  static let chrome = SwitcherWindowId(pid: 812, idx: 9123)
  static let ghostty = SwitcherWindowId(pid: 640, idx: 8812)
  static let zed = SwitcherWindowId(pid: 701, idx: 8920)
  static let whatsApp = SwitcherWindowId(pid: 903, idx: 9201)
  static let finder = SwitcherWindowId(pid: 977, idx: 9310)
  static let slack = SwitcherWindowId(pid: 988, idx: 9402)
  static let sugarglider = SwitcherContextId(id: 1)
  static let clientWork = SwitcherContextId(id: 4)

  /// Every command example with the command it stands for.
  static let commands: [(json: String, command: SwitcherCommand)] = [
    (#"{ "switch": { "id": 4 } }"#, .switchTo(.named(clientWork))),
    (#"{ "switch": "unsorted" }"#, .switchTo(.unsorted)),
    (#"{ "switch": "everything" }"#, .switchTo(.everything)),
    (
      #"{ "add_window": { "window": { "pid": 812, "idx": 9123 }, "context": { "id": 4 } } }"#,
      .addWindow(window: chrome, context: clientWork)
    ),
    (
      #"{ "move_window": { "window": { "pid": 812, "idx": 9123 }, "context": { "id": 4 } } }"#,
      .moveWindow(window: chrome, context: clientWork)
    ),
    (
      #"{ "toggle_pinned": { "window": { "pid": 812, "idx": 9123 } } }"#,
      .togglePinned(window: chrome)
    ),
    (
      #"""
      { "create": { "name": "Sugarglider",
                    "windows": [{ "pid": 640, "idx": 8812 }, { "pid": 812, "idx": 9123 }] } }
      """#,
      .create(name: "Sugarglider", windows: [ghostty, chrome])
    ),
    (
      #"""
      { "edit": { "context": { "id": 1 },
                  "add": [{ "pid": 977, "idx": 9310 }],
                  "remove": [{ "pid": 701, "idx": 8920 }],
                  "remove_records": [{ "record": 3, "app": "Mail", "title": "Inbox" }] } }
      """#,
      .edit(
        context: sugarglider,
        add: [finder],
        remove: [zed],
        removeRecords: [SwitcherRecordRef(record: 3, app: "Mail", title: "Inbox")]
      )
    ),
    (
      #"{ "rename": { "context": { "id": 4 }, "name": "Client work 2026" } }"#,
      .rename(context: clientWork, name: "Client work 2026")
    ),
    (
      #"{ "set_number": { "context": { "id": 4 }, "number": 2 } }"#,
      .setNumber(context: clientWork, number: 2)
    ),
    (#"{ "delete": { "id": 4 } }"#, .delete(clientWork)),
  ]

  static func payload() throws -> SwitcherPayload {
    try ContextSwitcherJSON.decode(SwitcherPayload.self, from: showPayload)
  }
}
