// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

import AppKit
import Foundation

// MARK: - Keys

/// A key press that the switcher acts on.
enum SwitcherKey: Equatable, Sendable {
  case up
  case down
  case enter
  case commandEnter
  case shiftCommandEnter
  case escape
  case space
  case commandN
  case commandE
  case commandR
  case commandP
  case commandDelete
  case commandDigit(Int)

  private static let upArrow: UInt16 = 126
  private static let downArrow: UInt16 = 125
  private static let returnKey: UInt16 = 36
  private static let keypadEnter: UInt16 = 76
  private static let escapeKey: UInt16 = 53
  private static let spaceKey: UInt16 = 49
  private static let deleteKey: UInt16 = 51

  /// The key codes of the ANSI number row, for layouts whose number row
  /// doesn't type digits.
  private static let digitKeyCodes: [UInt16: Int] = [
    18: 1, 19: 2, 20: 3, 21: 4, 23: 5, 22: 6, 26: 7, 28: 8, 25: 9,
  ]

  /// The key codes of the ANSI letters that the switcher and its text
  /// fields use with Command, for layouts that don't type Latin letters.
  private static let letterKeyCodes: [UInt16: Character] = [
    45: "n", 14: "e", 15: "r", 35: "p", 0: "a", 8: "c", 9: "v", 7: "x", 6: "z",
  ]

  /// Maps a key-down event. `characters` is the event's
  /// `charactersIgnoringModifiers`.
  init?(keyCode: UInt16, modifiers: NSEvent.ModifierFlags, characters: String?) {
    let modifiers = modifiers.intersection([.command, .shift, .option, .control])
    switch keyCode {
    case Self.upArrow where modifiers.isEmpty:
      self = .up
    case Self.downArrow where modifiers.isEmpty:
      self = .down
    case Self.returnKey, Self.keypadEnter:
      switch modifiers {
      case []:
        self = .enter
      case [.command]:
        self = .commandEnter
      case [.command, .shift]:
        self = .shiftCommandEnter
      default:
        return nil
      }
    case Self.escapeKey where modifiers.isEmpty:
      self = .escape
    case Self.spaceKey where modifiers.isEmpty:
      self = .space
    case Self.deleteKey where modifiers == [.command]:
      self = .commandDelete
    default:
      guard modifiers == [.command] else { return nil }
      if let digit = Self.digit(keyCode: keyCode, characters: characters) {
        self = .commandDigit(digit)
        return
      }
      switch Self.letter(keyCode: keyCode, characters: characters) {
      case "n":
        self = .commandN
      case "e":
        self = .commandE
      case "r":
        self = .commandR
      case "p":
        self = .commandP
      default:
        return nil
      }
    }
  }

  private static func digit(keyCode: UInt16, characters: String?) -> Int? {
    if let characters, characters.count == 1, let digit = Int(characters) {
      return (1...9).contains(digit) ? digit : nil
    }
    return digitKeyCodes[keyCode]
  }

  /// The lowercase ASCII letter a key types. A letter outside ASCII, as a
  /// Cyrillic layout types, counts as the ANSI letter of its key.
  /// Punctuation, digits, and symbols are no letter, whatever the key.
  static func letter(keyCode: UInt16, characters: String?) -> Character? {
    guard let characters, characters.count == 1,
      let character = characters.lowercased().first, character.isLetter
    else { return nil }
    return character.isASCII ? character : letterKeyCodes[keyCode]
  }
}

// MARK: - Backend

/// An error from the Rust side, with a message for the user.
struct SwitcherBridgeError: Error, Equatable {
  var message: String
}

/// What the switcher needs from Rust.
@MainActor
protocol ContextSwitcherBackend: AnyObject {
  /// Ranks the switcher's entries for a query, best first.
  func rank(_ query: String) throws -> [SwitcherRankedEntry]
  /// Sends a command. Throws when Rust rejects it.
  func run(_ command: SwitcherCommand) throws
}

// MARK: - Rows

/// A context row: a named context, Unsorted, or Everything.
struct SwitcherEntry: Equatable {
  var key: SwitcherContextKey
  var name: String
  /// Up to three app names, or a description of the entry.
  var detail: String
  /// The key binding that switches to it, or its number.
  var shortcut: String?
  var active: Bool

  var contextId: SwitcherContextId? {
    if case .named(let id) = key { return id }
    return nil
  }
}

enum SwitcherRow: Equatable {
  case entry(SwitcherEntry)
  /// Creates a context with this name.
  case newContext(String)
}

/// A window, or the record of a window that is gone, in the create and edit
/// views.
struct SwitcherChecklistItem: Equatable {
  enum Source: Equatable {
    case window(SwitcherWindowId)
    case record(SwitcherRecordRef)
  }

  var source: Source
  var title: String
  var app: String
  var checked: Bool
  var initiallyChecked: Bool
}

enum SwitcherMode: Equatable {
  /// The searchable list of contexts.
  case list
  /// Typing the name of a new context before choosing its windows.
  case naming
  /// Choosing the windows of a new context.
  case create(name: String)
  /// Choosing the members of a context.
  case edit(SwitcherContextId)
  case rename(SwitcherContextId)
  /// The inline confirmation before deleting a context.
  case confirmDelete(SwitcherContextId)
}

// MARK: - Model

/// The switcher's state and key handling. It shows nothing itself.
@MainActor
final class ContextSwitcherModel: ObservableObject {
  static let unsortedName = "Unsorted"
  static let everythingName = "Everything"
  static let maxApps = 3

  let payload: SwitcherPayload
  private let backend: ContextSwitcherBackend
  /// Called when the panel should close.
  var onClose: () -> Void = {}

  /// The search text. Changing it cancels a delete confirmation.
  @Published var query = "" {
    didSet {
      guard query != oldValue else { return }
      if case .confirmDelete = mode {
        mode = .list
      }
      refreshRows()
    }
  }
  /// The name typed in the naming and rename modes.
  @Published var nameDraft = "" {
    didSet {
      if nameDraft != oldValue { message = nil }
    }
  }
  @Published private(set) var mode = SwitcherMode.list
  @Published private(set) var rows: [SwitcherRow] = []
  /// The index of the highlighted row.
  @Published private(set) var highlight = 0
  @Published private(set) var checklist: [SwitcherChecklistItem] = []
  /// The index of the highlighted checklist item.
  @Published private(set) var checklistHighlight = 0
  /// A message about the last action, such as an error from Rust.
  @Published private(set) var message: String?
  /// Set while ranking is unavailable.
  @Published private(set) var rankError: String?

  /// True until the first context exists: the list stays empty and invites
  /// the user to type a name.
  var isFirstTime: Bool {
    payload.contexts.isEmpty && payload.everything.active
  }

  var targetWindow: SwitcherWindow? {
    guard let target = payload.targetWindow else { return nil }
    return payload.windows.first { $0.id == target }
  }

  var highlightedRow: SwitcherRow? {
    rows.indices.contains(highlight) ? rows[highlight] : nil
  }

  init(payload: SwitcherPayload, backend: ContextSwitcherBackend) {
    self.payload = payload
    self.backend = backend
    refreshRows()
  }

  func context(_ id: SwitcherContextId) -> SwitcherContext? {
    payload.contexts.first { $0.id == id.id }
  }

  // MARK: Rows

  private func refreshRows() {
    message = nil
    var entries: [SwitcherEntry]
    var hasExactMatch = false
    do {
      let ranked = try backend.rank(query)
      rankError = nil
      hasExactMatch = ranked.contains { $0.match == .exact }
      entries = ranked.compactMap { entry(for: $0.key) }
    } catch {
      rankError = "Search is unavailable. \(Self.describe(error))"
      entries = fallbackEntries()
    }
    if isFirstTime {
      entries = []
    }
    var rows = entries.map(SwitcherRow.entry)
    let name = trimmed(query)
    if !name.isEmpty && !hasExactMatch {
      rows.append(.newContext(name))
    }
    self.rows = rows
    highlight = defaultHighlight()
  }

  /// With an empty query the highlight skips the active context, so that ↩
  /// goes back to the context used before it.
  private func defaultHighlight() -> Int {
    guard trimmed(query).isEmpty, rows.count > 1, case .entry(let first) = rows[0], first.active
    else { return 0 }
    return 1
  }

  private func entry(for key: SwitcherContextKey) -> SwitcherEntry? {
    switch key {
    case .named(let id):
      guard let context = context(id) else { return nil }
      let apps = context.apps.prefix(Self.maxApps)
      return SwitcherEntry(
        key: key,
        name: context.name,
        detail: apps.isEmpty ? "no open windows" : apps.joined(separator: " · "),
        shortcut: context.hotkey ?? context.number.map(String.init),
        active: context.active
      )
    case .unsorted:
      let count = payload.unsorted.windows
      return SwitcherEntry(
        key: key,
        name: Self.unsortedName,
        detail: count == 1 ? "1 window" : "\(count) windows",
        shortcut: nil,
        active: payload.unsorted.active
      )
    case .everything:
      return SwitcherEntry(
        key: key,
        name: Self.everythingName,
        detail: "show all windows",
        shortcut: payload.everything.hotkey,
        active: payload.everything.active
      )
    }
  }

  /// The entries in payload order, unfiltered, for when ranking is
  /// unavailable.
  private func fallbackEntries() -> [SwitcherEntry] {
    var keys = payload.contexts.map(\.key)
    if payload.unsorted.windows > 0 {
      keys.append(.unsorted)
    }
    keys.append(.everything)
    return keys.compactMap { entry(for: $0) }
  }

  // MARK: Keys

  /// Handles a key. Returns false, without changing any state, when the
  /// key belongs to the text field.
  @discardableResult
  func handle(_ key: SwitcherKey) -> Bool {
    switch mode {
    case .list:
      return handleListKey(key)
    case .naming, .rename:
      return handleNameKey(key)
    case .create, .edit:
      return handleChecklistKey(key)
    case .confirmDelete(let id):
      switch key {
      case .enter:
        run(.delete(id))
      case .escape:
        showList()
      default:
        break
      }
      return true
    }
  }

  private func handleListKey(_ key: SwitcherKey) -> Bool {
    switch key {
    case .up:
      moveHighlight(by: -1)
    case .down:
      moveHighlight(by: 1)
    case .enter:
      activateHighlightedRow()
    case .commandEnter:
      sendTargetWindow(move: false)
    case .shiftCommandEnter:
      sendTargetWindow(move: true)
    case .commandN:
      newContext()
    case .commandE:
      if let context = highlightedContext(verb: "edited") {
        startEdit(context)
      }
    case .commandR:
      if let context = highlightedContext(verb: "renamed") {
        nameDraft = context.name
        mode = .rename(context.contextId)
        message = nil
      }
    case .commandDigit(let number):
      if let context = highlightedContext(verb: "numbered") {
        run(.setNumber(context: context.contextId, number: number))
      }
    case .commandP:
      if let target = requireTargetWindow() {
        run(.togglePinned(window: target))
      }
    case .commandDelete:
      if let context = highlightedContext(verb: "deleted") {
        mode = .confirmDelete(context.contextId)
        message = nil
      }
    case .escape:
      onClose()
    case .space:
      return false
    }
    return true
  }

  private func handleNameKey(_ key: SwitcherKey) -> Bool {
    switch key {
    case .enter:
      confirmName()
    case .escape:
      showList()
    default:
      return false
    }
    return true
  }

  private func handleChecklistKey(_ key: SwitcherKey) -> Bool {
    switch key {
    case .up:
      checklistHighlight = max(checklistHighlight - 1, 0)
    case .down:
      checklistHighlight = max(min(checklistHighlight + 1, checklist.count - 1), 0)
    case .space:
      toggleItem(at: checklistHighlight)
    case .enter:
      confirmChecklist()
    case .escape:
      showList()
    default:
      return false
    }
    return true
  }

  // MARK: Actions

  /// Highlights a row and acts on it, as ↩ does.
  func click(row index: Int) {
    guard rows.indices.contains(index), mode == .list else { return }
    highlight = index
    activateHighlightedRow()
  }

  func toggleItem(at index: Int) {
    guard checklist.indices.contains(index) else { return }
    checklistHighlight = index
    checklist[index].checked.toggle()
  }

  private func moveHighlight(by offset: Int) {
    guard !rows.isEmpty else { return }
    highlight = min(max(highlight + offset, 0), rows.count - 1)
  }

  private func activateHighlightedRow() {
    switch highlightedRow {
    case .entry(let entry):
      run(.switchTo(entry.key))
    case .newContext(let name):
      startCreate(name: name)
    case nil:
      break
    }
  }

  private func sendTargetWindow(move: Bool) {
    guard let row = highlightedRow else { return }
    switch row {
    case .newContext(let name):
      message = "Press ↩ to create “\(name)” first."
    case .entry(let entry):
      guard let id = entry.contextId else {
        message = "Windows can't be \(move ? "moved" : "added") to \(entry.name)."
        return
      }
      guard let target = requireTargetWindow() else { return }
      run(move ? .moveWindow(window: target, context: id) : .addWindow(window: target, context: id))
    }
  }

  private func requireTargetWindow() -> SwitcherWindowId? {
    guard let target = payload.targetWindow else {
      message = "No window had focus when the switcher opened."
      return nil
    }
    return target
  }

  /// The highlighted named context. Sets a message and returns nil when the
  /// highlighted row is something else.
  private func highlightedContext(verb: String) -> SwitcherContext? {
    switch highlightedRow {
    case .entry(let entry):
      if let id = entry.contextId, let context = context(id) {
        return context
      }
      message = "\(entry.name) can't be \(verb)."
    case .newContext(let name):
      message = "Press ↩ to create “\(name)” first."
    case nil:
      break
    }
    return nil
  }

  private func newContext() {
    let name = trimmed(query)
    if name.isEmpty {
      nameDraft = ""
      mode = .naming
      message = nil
    } else {
      startCreate(name: name)
    }
  }

  private func startCreate(name: String) {
    checklist = payload.windows.map { window in
      SwitcherChecklistItem(
        source: .window(window.id),
        title: window.title,
        app: window.app,
        checked: true,
        initiallyChecked: true
      )
    }
    checklistHighlight = 0
    mode = .create(name: name)
    message = nil
  }

  private func startEdit(_ context: SwitcherContext) {
    let onScreen = Set(payload.windows.map(\.id))
    let members = Set(context.members.compactMap(\.window))
    var items = payload.windows.map { window in
      let member = members.contains(window.id)
      return SwitcherChecklistItem(
        source: .window(window.id),
        title: window.title,
        app: window.app,
        checked: member,
        initiallyChecked: member
      )
    }
    for member in context.members {
      if let window = member.window, onScreen.contains(window) { continue }
      let source: SwitcherChecklistItem.Source =
        member.window.map { .window($0) }
        ?? .record(SwitcherRecordRef(record: member.record, app: member.app, title: member.title))
      items.append(
        SwitcherChecklistItem(
          source: source,
          title: member.title,
          app: member.app,
          checked: true,
          initiallyChecked: true
        )
      )
    }
    checklist = items
    checklistHighlight = 0
    mode = .edit(context.contextId)
    message = nil
  }

  private func confirmName() {
    let name = trimmed(nameDraft)
    guard !name.isEmpty else {
      message = "A context name can't be empty."
      return
    }
    switch mode {
    case .naming:
      startCreate(name: name)
    case .rename(let id):
      if name == context(id)?.name {
        showList()
      } else {
        run(.rename(context: id, name: name))
      }
    default:
      break
    }
  }

  private func confirmChecklist() {
    switch mode {
    case .create(let name):
      let windows = checklist.compactMap { item -> SwitcherWindowId? in
        guard item.checked, case .window(let id) = item.source else { return nil }
        return id
      }
      run(.create(name: name, windows: windows))
    case .edit(let id):
      var add: [SwitcherWindowId] = []
      var remove: [SwitcherWindowId] = []
      var removeRecords: [SwitcherRecordRef] = []
      for item in checklist where item.checked != item.initiallyChecked {
        switch item.source {
        case .window(let window) where item.checked:
          add.append(window)
        case .window(let window):
          remove.append(window)
        case .record(let record):
          if !item.checked {
            removeRecords.append(record)
          }
        }
      }
      if add.isEmpty && remove.isEmpty && removeRecords.isEmpty {
        onClose()
      } else {
        run(.edit(context: id, add: add, remove: remove, removeRecords: removeRecords))
      }
    default:
      break
    }
  }

  private func showList() {
    mode = .list
    message = nil
  }

  /// Sends a command, and closes the panel when Rust accepts it.
  private func run(_ command: SwitcherCommand) {
    do {
      try backend.run(command)
      onClose()
    } catch {
      message = Self.describe(error)
    }
  }

  private func trimmed(_ text: String) -> String {
    text.trimmingCharacters(in: .whitespacesAndNewlines)
  }

  private static func describe(_ error: Error) -> String {
    (error as? SwitcherBridgeError)?.message ?? String(describing: error)
  }
}
