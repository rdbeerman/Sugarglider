// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import Foundation

/// The JSON contract between the context switcher panel and Rust.
///
/// The full contract, including what the Rust half must do, is
/// `docs/specs/contexts-switcher-contract.md`. This comment summarizes it,
/// and its examples are the same as the ones there. The design is
/// `docs/specs/contexts.md` ("Switcher" and "Swift bridge").
///
/// All keys are snake_case. A named context is always tagged as
/// `{"id": N}`, never a bare number, so it can't be read as a context
/// number. The strings `"everything"` and `"unsorted"` name the two
/// built-in entries. This key is not the key-binding `ContextRef`, where a
/// bare string is a name: the switcher never sends a name, except as the
/// new name in `create` and `rename`.
///
/// A window is Rust's `WindowId` in its serde shape,
/// `{"pid": 812, "idx": 9123}`. The switcher never reads its fields. It
/// sends the value back unchanged.
///
/// Functions
/// ---------
///
/// Rust to Swift, exported by SugargliderUI:
///
/// - `sugarglider_show_context_switcher(json: *const c_char)` shows a
///   fresh panel with the show payload below, or closes the panel when it
///   is open, so the switcher's hotkey toggles it. The reactor calls it from
///   its own thread for every `open_context_switcher`, and doesn't track
///   whether the panel is open. Swift copies the string before it returns
///   and acts on the main queue. A NULL `json`, or a payload that doesn't
///   decode, opens nothing.
/// - `sugarglider_hide_context_switcher()` hides the panel if it is open.
///
/// Swift to Rust, looked up with `dlsym(RTLD_DEFAULT, …)` when the panel
/// needs them, so a binary without them still loads, and the panel shows an
/// inline error instead:
///
/// - `sugarglider_rank_contexts(query: *const c_char) -> *mut c_char`
///   returns `model::contexts::rank(query, …)` as the rank result below,
///   computed from the latest published state. NULL means contexts are
///   unavailable, for example because the feature is off.
/// - `sugarglider_run_context_command(json: *const c_char) -> *mut c_char`
///   takes one command below. NULL means Rust accepted the command and sent
///   it to the reactor. Otherwise it returns an error message for the user:
///   invalid JSON, the feature is off, a context id that no longer exists,
///   a name that is empty, reserved, or taken (R4), or a number outside 1 to
///   9. Rust checks these against the published snapshot before it sends
///   the command. The reactor logs failures that come later.
///
/// Swift frees every returned string with the existing
/// `sugarglider_free_string`. Each input string is NUL-terminated UTF-8 and
/// is valid only during the call.
///
/// Show payload
/// ------------
///
/// ```json
/// {
///   "display_id": 1,
///   "target_window": { "pid": 812, "idx": 9123 },
///   "contexts": [
///     {
///       "id": 1, "name": "Sugarglider", "number": 1, "hotkey": "⌃⌥1",
///       "active": true, "apps": ["Ghostty", "Zed", "Google Chrome"], "windows": 3,
///       "members": [
///         { "record": 0, "app": "Ghostty", "title": "~/src/sugarglider",
///           "window": { "pid": 640, "idx": 8812 } },
///         { "record": 1, "app": "Zed", "title": "reactor.rs",
///           "window": { "pid": 701, "idx": 8920 } },
///         { "record": 2, "app": "Google Chrome", "title": "Docs",
///           "window": { "pid": 812, "idx": 9123 } },
///         { "record": 3, "app": "Mail", "title": "Inbox", "window": null }
///       ]
///     },
///     {
///       "id": 4, "name": "Client work", "number": null, "hotkey": null,
///       "active": false, "apps": [], "windows": 0, "members": []
///     }
///   ],
///   "unsorted": { "windows": 2, "active": false },
///   "everything": { "active": false, "hotkey": "⌃⌥0" },
///   "windows": [
///     { "id": { "pid": 640, "idx": 8812 }, "title": "~/src/sugarglider",
///       "app": "Ghostty", "tab_count": 1, "pinned": false },
///     { "id": { "pid": 812, "idx": 9123 }, "title": "Docs",
///       "app": "Google Chrome", "tab_count": 3, "pinned": false },
///     { "id": { "pid": 903, "idx": 9201 }, "title": "WhatsApp",
///       "app": "WhatsApp", "tab_count": 1, "pinned": true },
///     { "id": { "pid": 977, "idx": 9310 }, "title": "Downloads",
///       "app": "Finder", "tab_count": 1, "pinned": false },
///     { "id": { "pid": 988, "idx": 9402 }, "title": "general",
///       "app": "Slack", "tab_count": 1, "pinned": false }
///   ]
/// }
/// ```
///
/// - `display_id`: the `CGDirectDisplayID` of the focused screen. The panel
///   centers on it. Null or unknown means the main screen.
/// - `target_window`: the window that had focus when the reactor handled
///   `open_context_switcher`. Every command about a window carries this id,
///   never the window focused when the command is sent. It is null when no
///   window had focus, or when the focused window is untracked, parked, or
///   Sugarglider's own. Then the panel disables ⌘↩, ⇧⌘↩, and ⌘P and says
///   why. Otherwise `windows` holds it.
/// - `contexts`: every named context. `number` is 1 to 9 or null.
///   `hotkey` is the binding that runs `switch_context` with that number,
///   formatted like the Preferences hotkey list, or null when none is bound.
///   `active` is true for the active context. `apps` holds the distinct app
///   names of the context's open member windows, in member order; the panel
///   shows the first three. `windows` counts those open windows. `members`
///   lists every member record in the model's order: `record` is the
///   record's index in that list, `app` is its app name (the bundle id when
///   the name is unknown), and `window` is its open window, or null when the
///   window is gone (the record is empty or pending, R23).
/// - `unsorted`: the number of unsorted windows (R3, R29) and whether
///   Unsorted is active.
/// - `everything`: whether Everything is active, and the binding that runs
///   `show_everything`, or null.
/// - `windows`: the windows on screen that the create and edit views list.
///   These are the tracked windows that show now on the visible Spaces (the
///   focused screen in `per_screen` scope), including the target window.
///   They leave out parked windows, Sugarglider's own windows, and untracked
///   windows. A native tab group is one entry, its main tab, and
///   `tab_count` says how many tabs it has (R36). `pinned` says whether it
///   is pinned (R3). Which contexts hold a window is only in
///   `contexts[].members`.
///
/// Every window id in the payload names a tab group by its main tab, and a
/// command about a window applies to its whole group (R36).
///
/// Exactly one of the `active` flags is true. A key whose value can be null
/// may also be missing.
///
/// Rank result
/// -----------
///
/// The entries of `rank`, best first. `match` is the `NameMatch` variant in
/// snake_case: `exact`, `name_prefix`, `word_prefix`, `initials`,
/// `all_word_prefixes`, `letters_in_order`, or `empty_query`. For the query
/// "cli":
///
/// ```json
/// [
///   { "key": { "id": 4 }, "match": "name_prefix" }
/// ]
/// ```
///
/// For the empty query, every entry, most recently used first. Unsorted is
/// listed only when it has windows (R29):
///
/// ```json
/// [
///   { "key": { "id": 1 }, "match": "empty_query" },
///   { "key": { "id": 4 }, "match": "empty_query" },
///   { "key": "unsorted", "match": "empty_query" },
///   { "key": "everything", "match": "empty_query" }
/// ]
/// ```
///
/// The panel shows the entries in this order and skips ids that the show
/// payload doesn't have. It shows the "New context" row when no entry has
/// `match` equal to `exact` and the trimmed query can name a new context:
/// it isn't empty, reserved, or the name of a context in the payload
/// (`ContextSwitcherModel.newNameProblem`).
///
/// Commands
/// --------
///
/// Each command is an object with one key, the command's name.
///
/// ```json
/// { "switch": { "id": 4 } }
/// ```
///
/// ```json
/// { "switch": "unsorted" }
/// ```
///
/// ```json
/// { "switch": "everything" }
/// ```
/// Switches to the entry (R12). `"everything"` shows Everything (R27).
///
/// ```json
/// { "add_window": { "window": { "pid": 812, "idx": 9123 }, "context": { "id": 4 } } }
/// ```
/// Adds the target window to the context, as `add_window_to_context` does
/// for the focused window (R37).
///
/// ```json
/// { "move_window": { "window": { "pid": 812, "idx": 9123 }, "context": { "id": 4 } } }
/// ```
/// Moves the target window out of the active context and into this one, as
/// `move_window_to_context` does (R37).
///
/// ```json
/// { "toggle_pinned": { "window": { "pid": 812, "idx": 9123 } } }
/// ```
/// Pins or unpins the target window, as `toggle_window_pinned` does (R3).
///
/// ```json
/// { "create": { "name": "Sugarglider",
///               "windows": [{ "pid": 640, "idx": 8812 }, { "pid": 812, "idx": 9123 }] } }
/// ```
/// Creates a context with this name (R4) and the lowest free number (R5),
/// whose members are exactly these windows, and switches to it, as
/// `sugarglider context create` does. `windows` can be empty. ⌘N asks for
/// the name first, filled with the query. When Rust rejects `create`, the
/// panel asks for the name again and keeps the checked windows.
///
/// A pinned window is a member of every context already (R3). The create
/// and edit views show it checked and fixed, and `create.windows`,
/// `edit.add`, and `edit.remove` never hold it. Rust ignores a pinned
/// window there and gives it no record.
///
/// ```json
/// { "edit": { "context": { "id": 1 },
///             "add": [{ "pid": 977, "idx": 9310 }],
///             "remove": [{ "pid": 701, "idx": 8920 }],
///             "remove_records": [{ "record": 3, "app": "Mail", "title": "Inbox" }] } }
/// ```
/// Changes a context's members. Rust first removes each record in
/// `remove_records` (`Contexts::remove_record`, R23), from the highest
/// `record` down. It removes a record only when the record at that index
/// still has no open window and still has this app and title; otherwise it
/// skips the item and logs it. Then it removes the windows in `remove`, which
/// takes effect at once, as `remove_window_from_context` does (R37). Then it
/// adds the windows in `add`, which takes effect at the next switch, as
/// `add_window_to_context` does (R37).
///
/// ```json
/// { "rename": { "context": { "id": 4 }, "name": "Client work 2026" } }
/// ```
/// Renames the context (R4).
///
/// ```json
/// { "set_number": { "context": { "id": 4 }, "number": 2 } }
/// ```
/// Gives the context a number from 1 to 9, and takes it from the context
/// that had it (R5).
///
/// ```json
/// { "delete": { "id": 4 } }
/// ```
/// Deletes the context (R6).
///
/// Error return
/// ------------
///
/// `sugarglider_run_context_command` returns NULL on success, or a message
/// such as `A context named "Comms" already exists`. The panel shows the
/// message inline and stays open. After a command succeeds, the panel
/// closes. The panel never sends a command that needs a named context for
/// Unsorted or Everything (R29).
enum ContextSwitcherJSON {
  static func decoder() -> JSONDecoder {
    JSONDecoder()
  }

  static func encoder() -> JSONEncoder {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
    return encoder
  }

  static func decode<T: Decodable>(_ type: T.Type, from json: String) throws -> T {
    try decoder().decode(type, from: Data(json.utf8))
  }

  static func encode<T: Encodable>(_ value: T) throws -> String {
    let data = try encoder().encode(value)
    return String(decoding: data, as: UTF8.self)
  }
}

// MARK: - Identifiers

/// An open window, as Rust's `WindowId` serializes it.
struct SwitcherWindowId: Codable, Hashable, Sendable {
  var pid: Int32
  var idx: UInt32
}

/// A named context, as `{"id": N}`.
struct SwitcherContextId: Codable, Hashable, Sendable {
  var id: UInt32
}

/// An entry the switcher can switch to.
enum SwitcherContextKey: Hashable, Sendable {
  case everything
  case unsorted
  case named(SwitcherContextId)
}

extension SwitcherContextKey: Codable {
  init(from decoder: Decoder) throws {
    if let container = try? decoder.singleValueContainer(),
      let text = try? container.decode(String.self)
    {
      switch text {
      case "everything":
        self = .everything
      case "unsorted":
        self = .unsorted
      default:
        throw DecodingError.dataCorruptedError(
          in: container,
          debugDescription: "Expected \"everything\", \"unsorted\", or {\"id\": N}, got \"\(text)\""
        )
      }
      return
    }
    self = .named(try SwitcherContextId(from: decoder))
  }

  func encode(to encoder: Encoder) throws {
    switch self {
    case .everything:
      var container = encoder.singleValueContainer()
      try container.encode("everything")
    case .unsorted:
      var container = encoder.singleValueContainer()
      try container.encode("unsorted")
    case .named(let id):
      try id.encode(to: encoder)
    }
  }
}

// MARK: - Show payload

struct SwitcherPayload: Codable, Equatable, Sendable {
  var displayId: UInt32?
  var targetWindow: SwitcherWindowId?
  var contexts: [SwitcherContext]
  var unsorted: SwitcherUnsorted
  var everything: SwitcherEverything
  var windows: [SwitcherWindow]

  enum CodingKeys: String, CodingKey {
    case displayId = "display_id"
    case targetWindow = "target_window"
    case contexts
    case unsorted
    case everything
    case windows
  }
}

struct SwitcherContext: Codable, Equatable, Sendable {
  var id: UInt32
  var name: String
  var number: Int?
  var hotkey: String?
  var active: Bool
  var apps: [String]
  var windows: Int
  var members: [SwitcherMember]

  var contextId: SwitcherContextId { SwitcherContextId(id: id) }
  var key: SwitcherContextKey { .named(contextId) }
}

/// A member record of a context.
struct SwitcherMember: Codable, Hashable, Sendable {
  var record: Int
  var app: String
  var title: String
  var window: SwitcherWindowId?

  enum CodingKeys: String, CodingKey {
    case record
    case app
    case title
    case window
  }

  init(record: Int, app: String, title: String, window: SwitcherWindowId?) {
    self.record = record
    self.app = app
    self.title = title
    self.window = window
  }

  init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: CodingKeys.self)
    record = try container.decode(Int.self, forKey: .record)
    app = try container.decode(String.self, forKey: .app)
    title = try container.decode(String.self, forKey: .title)
    window = try container.decodeIfPresent(SwitcherWindowId.self, forKey: .window)
  }

  func encode(to encoder: Encoder) throws {
    var container = encoder.container(keyedBy: CodingKeys.self)
    try container.encode(record, forKey: .record)
    try container.encode(app, forKey: .app)
    try container.encode(title, forKey: .title)
    try container.encode(window, forKey: .window)
  }
}

struct SwitcherUnsorted: Codable, Equatable, Sendable {
  var windows: Int
  var active: Bool
}

struct SwitcherEverything: Codable, Equatable, Sendable {
  var active: Bool
  var hotkey: String?
}

/// A window on screen: a window without tabs, or the main tab of a native
/// tab group (R36).
struct SwitcherWindow: Codable, Equatable, Sendable {
  var id: SwitcherWindowId
  var title: String
  var app: String
  /// How many tabs its group has, 1 for a window without tabs.
  var tabCount: Int
  var pinned: Bool

  enum CodingKeys: String, CodingKey {
    case id
    case title
    case app
    case tabCount = "tab_count"
    case pinned
  }
}

// MARK: - Rank result

/// How well a query matches a name, as Rust's `NameMatch` names it.
enum SwitcherNameMatch: String, Codable, Sendable {
  case exact
  case namePrefix = "name_prefix"
  case wordPrefix = "word_prefix"
  case initials
  case allWordPrefixes = "all_word_prefixes"
  case lettersInOrder = "letters_in_order"
  case emptyQuery = "empty_query"
}

struct SwitcherRankedEntry: Codable, Equatable, Sendable {
  var key: SwitcherContextKey
  var match: SwitcherNameMatch
}

// MARK: - Commands

/// A member record that `edit` removes. Rust removes it only when the
/// record at `record` still has this app and title.
struct SwitcherRecordRef: Codable, Hashable, Sendable {
  var record: Int
  var app: String
  var title: String
}

/// A command the panel sends to Rust.
enum SwitcherCommand: Equatable, Sendable {
  case switchTo(SwitcherContextKey)
  case addWindow(window: SwitcherWindowId, context: SwitcherContextId)
  case moveWindow(window: SwitcherWindowId, context: SwitcherContextId)
  case togglePinned(window: SwitcherWindowId)
  case create(name: String, windows: [SwitcherWindowId])
  case edit(
    context: SwitcherContextId,
    add: [SwitcherWindowId],
    remove: [SwitcherWindowId],
    removeRecords: [SwitcherRecordRef]
  )
  case rename(context: SwitcherContextId, name: String)
  case setNumber(context: SwitcherContextId, number: Int)
  case delete(SwitcherContextId)
}

extension SwitcherCommand: Codable {
  private enum Name: String, CodingKey {
    case switchTo = "switch"
    case addWindow = "add_window"
    case moveWindow = "move_window"
    case togglePinned = "toggle_pinned"
    case create
    case edit
    case rename
    case setNumber = "set_number"
    case delete
  }

  private struct WindowAndContext: Codable {
    var window: SwitcherWindowId
    var context: SwitcherContextId
  }

  private struct WindowOnly: Codable {
    var window: SwitcherWindowId
  }

  private struct Create: Codable {
    var name: String
    var windows: [SwitcherWindowId]
  }

  private struct Edit: Codable {
    var context: SwitcherContextId
    var add: [SwitcherWindowId]
    var remove: [SwitcherWindowId]
    var removeRecords: [SwitcherRecordRef]

    enum CodingKeys: String, CodingKey {
      case context
      case add
      case remove
      case removeRecords = "remove_records"
    }
  }

  private struct Rename: Codable {
    var context: SwitcherContextId
    var name: String
  }

  private struct SetNumber: Codable {
    var context: SwitcherContextId
    var number: Int
  }

  init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: Name.self)
    guard container.allKeys.count == 1, let name = container.allKeys.first else {
      throw DecodingError.dataCorrupted(
        DecodingError.Context(
          codingPath: decoder.codingPath,
          debugDescription: "A command is an object with exactly one known key"
        )
      )
    }
    switch name {
    case .switchTo:
      self = .switchTo(try container.decode(SwitcherContextKey.self, forKey: name))
    case .addWindow:
      let body = try container.decode(WindowAndContext.self, forKey: name)
      self = .addWindow(window: body.window, context: body.context)
    case .moveWindow:
      let body = try container.decode(WindowAndContext.self, forKey: name)
      self = .moveWindow(window: body.window, context: body.context)
    case .togglePinned:
      self = .togglePinned(window: try container.decode(WindowOnly.self, forKey: name).window)
    case .create:
      let body = try container.decode(Create.self, forKey: name)
      self = .create(name: body.name, windows: body.windows)
    case .edit:
      let body = try container.decode(Edit.self, forKey: name)
      self = .edit(
        context: body.context,
        add: body.add,
        remove: body.remove,
        removeRecords: body.removeRecords
      )
    case .rename:
      let body = try container.decode(Rename.self, forKey: name)
      self = .rename(context: body.context, name: body.name)
    case .setNumber:
      let body = try container.decode(SetNumber.self, forKey: name)
      self = .setNumber(context: body.context, number: body.number)
    case .delete:
      self = .delete(try container.decode(SwitcherContextId.self, forKey: name))
    }
  }

  func encode(to encoder: Encoder) throws {
    var container = encoder.container(keyedBy: Name.self)
    switch self {
    case .switchTo(let key):
      try container.encode(key, forKey: .switchTo)
    case .addWindow(let window, let context):
      try container.encode(WindowAndContext(window: window, context: context), forKey: .addWindow)
    case .moveWindow(let window, let context):
      try container.encode(WindowAndContext(window: window, context: context), forKey: .moveWindow)
    case .togglePinned(let window):
      try container.encode(WindowOnly(window: window), forKey: .togglePinned)
    case .create(let name, let windows):
      try container.encode(Create(name: name, windows: windows), forKey: .create)
    case .edit(let context, let add, let remove, let removeRecords):
      try container.encode(
        Edit(context: context, add: add, remove: remove, removeRecords: removeRecords),
        forKey: .edit
      )
    case .rename(let context, let name):
      try container.encode(Rename(context: context, name: name), forKey: .rename)
    case .setNumber(let context, let number):
      try container.encode(SetNumber(context: context, number: number), forKey: .setNumber)
    case .delete(let context):
      try container.encode(context, forKey: .delete)
    }
  }
}
